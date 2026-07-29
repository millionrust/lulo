//! Bounded directory-event coalescing and Linux mount-watcher restart policy.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::RecommendedWatcher;

pub(crate) const DIRECTORY_STALL_NOTICE_DELAY: Duration = Duration::from_secs(8);
pub(crate) const DIRECTORY_STALL_NOTICE: &str =
    "This location is responding slowly; you can navigate elsewhere while Files keeps checking";
pub(crate) const FILESYSTEM_WATCH_INTERRUPTED_MESSAGE: &str =
    "Live folder updates were interrupted; Files is rechecking";
pub(crate) const FILESYSTEM_WATCH_UNAVAILABLE_MESSAGE: &str =
    "This folder could not be watched; Files will verify every refresh";
#[cfg(target_os = "linux")]
pub(crate) const MOUNT_WATCH_UNAVAILABLE_MESSAGE: &str =
    "Automatic mounted-volume updates are temporarily unavailable";

const MAX_RENAME_HINTS: usize = 16;
#[cfg(any(target_os = "linux", test))]
const MOUNT_WATCH_STABLE_PERIOD: Duration = Duration::from_secs(60);
#[cfg(any(target_os = "linux", test))]
const MOUNT_WATCH_MAX_RETRY: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct FilesystemHints {
    pub(crate) watch_error: bool,
    pub(crate) renames: Vec<(PathBuf, PathBuf)>,
}

impl FilesystemHints {
    pub(crate) fn record(&mut self, result: notify::Result<notify::Event>) {
        match result {
            Ok(event) => {
                if matches!(
                    event.kind,
                    notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                ) && event.paths.len() == 2
                    && event.paths[0] != event.paths[1]
                    && self.renames.len() < MAX_RENAME_HINTS
                {
                    self.renames
                        .push((event.paths[0].clone(), event.paths[1].clone()));
                }
            }
            Err(_) => self.watch_error = true,
        }
    }
}

pub(crate) fn filesystem_watcher(
    events: async_channel::Sender<()>,
    hints: Arc<Mutex<FilesystemHints>>,
) -> notify::Result<RecommendedWatcher> {
    notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(mut hints) = hints.lock() {
            hints.record(result);
        }
        let _ = events.try_send(());
    })
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MountWatchNotice {
    None,
    Unavailable,
    Restored,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
pub(crate) struct MountWatchHealth {
    pub(crate) unavailable: bool,
}

#[cfg(any(target_os = "linux", test))]
impl MountWatchHealth {
    pub(crate) fn record(&mut self, event: rmac_mounts::WatchEvent) -> MountWatchNotice {
        match event {
            rmac_mounts::WatchEvent::Unavailable if !self.unavailable => {
                self.unavailable = true;
                MountWatchNotice::Unavailable
            }
            rmac_mounts::WatchEvent::Unavailable => MountWatchNotice::None,
            rmac_mounts::WatchEvent::Changed if self.unavailable => {
                self.unavailable = false;
                MountWatchNotice::Restored
            }
            rmac_mounts::WatchEvent::Changed => MountWatchNotice::None,
        }
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn next_mount_watch_retry(
    failures: u32,
    previous_attempt_lifetime: Duration,
) -> (u32, Duration) {
    let failures = if previous_attempt_lifetime >= MOUNT_WATCH_STABLE_PERIOD {
        1
    } else {
        failures.saturating_add(1)
    };
    let shift = failures.saturating_sub(1).min(5);
    let delay = Duration::from_secs(1u64 << shift).min(MOUNT_WATCH_MAX_RETRY);
    (failures, delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_event_bursts_coalesce_until_consumed() {
        let (sender, receiver) = async_channel::bounded(1);

        sender.try_send(()).expect("first event should wake the UI");
        assert!(sender.try_send(()).is_err(), "burst should stay bounded");
        receiver
            .try_recv()
            .expect("the queued wake should be available");
        sender
            .try_send(())
            .expect("a new event should queue after consumption");
    }

    #[test]
    fn filesystem_hints_bound_renames_and_retain_watch_failure() {
        use notify::event::{ModifyKind, RenameMode};

        let mut hints = FilesystemHints::default();
        for index in 0..(MAX_RENAME_HINTS + 5) {
            hints.record(Ok(notify::Event::new(notify::EventKind::Modify(
                ModifyKind::Name(RenameMode::Both),
            ))
            .add_path(PathBuf::from(format!("/old/{index}")))
            .add_path(PathBuf::from(format!("/new/{index}")))));
        }
        hints.record(Err(notify::Error::generic("watch failed")));

        assert_eq!(hints.renames.len(), MAX_RENAME_HINTS);
        assert!(hints.watch_error);
    }

    #[test]
    fn mount_watch_health_reports_only_real_outage_transitions() {
        let mut health = MountWatchHealth::default();

        assert_eq!(
            health.record(rmac_mounts::WatchEvent::Changed),
            MountWatchNotice::None
        );
        assert_eq!(
            health.record(rmac_mounts::WatchEvent::Unavailable),
            MountWatchNotice::Unavailable
        );
        assert_eq!(
            health.record(rmac_mounts::WatchEvent::Unavailable),
            MountWatchNotice::None
        );
        assert_eq!(
            health.record(rmac_mounts::WatchEvent::Changed),
            MountWatchNotice::Restored
        );
        assert_eq!(
            health.record(rmac_mounts::WatchEvent::Changed),
            MountWatchNotice::None
        );
    }

    #[test]
    fn mount_watch_restart_backoff_is_bounded_and_resets_after_stability() {
        let mut failures = 0;
        let mut delays = Vec::new();
        for _ in 0..7 {
            let retry = next_mount_watch_retry(failures, Duration::from_secs(1));
            failures = retry.0;
            delays.push(retry.1);
        }

        assert_eq!(
            delays,
            [
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
                Duration::from_secs(30),
                Duration::from_secs(30),
            ]
        );
        assert_eq!(
            next_mount_watch_retry(failures, MOUNT_WATCH_STABLE_PERIOD),
            (1, Duration::from_secs(1))
        );
    }
}
