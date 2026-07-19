//! Event-driven process boundary for the rmac top bar.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use chrono::{DateTime, FixedOffset, Local};
use futures_util::FutureExt as _;

const FALLBACK_HOUR_CYCLE: rmac_top_bar::LocaleHourCycle =
    rmac_top_bar::LocaleHourCycle::TwentyFourHour;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    operation: &'static str,
}

impl Error {
    fn new(operation: &'static str) -> Self {
        Self { operation }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}", self.operation)
    }
}

impl std::error::Error for Error {}

/// Pure reconciliation state. Wall-clock samples are injected so clock jumps,
/// locale changes, and timer behavior can be verified without sleeping.
#[derive(Default)]
pub struct Coordinator {
    presentation: rmac_top_bar::State,
    status: Option<rmac_shell_status::Snapshot>,
    locale_hour_cycle: Option<rmac_top_bar::LocaleHourCycle>,
    time_source_ready: bool,
}

impl Coordinator {
    pub fn apply_shell(
        &mut self,
        update: rmac_shell_runtime::Update,
        now: DateTime<FixedOffset>,
    ) -> Option<rmac_top_bar::Update> {
        self.status = Some(update.snapshot.status);
        self.evaluate(now)
    }

    pub fn apply_locale(
        &mut self,
        hour_cycle: Result<rmac_locale::HourCycle, ()>,
        now: DateTime<FixedOffset>,
    ) -> Option<rmac_top_bar::Update> {
        match hour_cycle {
            Ok(hour_cycle) => self.locale_hour_cycle = Some(top_bar_hour_cycle(hour_cycle)),
            Err(()) if self.locale_hour_cycle.is_none() => {
                self.locale_hour_cycle = Some(FALLBACK_HOUR_CYCLE);
            }
            Err(()) => {}
        }
        self.evaluate(now)
    }

    pub fn apply_time_signal(
        &mut self,
        now: DateTime<FixedOffset>,
    ) -> Option<rmac_top_bar::Update> {
        self.time_source_ready = true;
        self.evaluate(now)
    }

    pub fn clock_deadline(&mut self, now: DateTime<FixedOffset>) -> Option<rmac_top_bar::Update> {
        self.evaluate(now)
    }

    fn evaluate(&mut self, now: DateTime<FixedOffset>) -> Option<rmac_top_bar::Update> {
        let status = self.status.as_ref()?;
        let locale_hour_cycle = self.locale_hour_cycle?;
        self.time_source_ready
            .then(|| self.presentation.apply(status, now, locale_hour_cycle))
    }
}

fn top_bar_hour_cycle(value: rmac_locale::HourCycle) -> rmac_top_bar::LocaleHourCycle {
    match value {
        rmac_locale::HourCycle::TwelveHour => rmac_top_bar::LocaleHourCycle::TwelveHour,
        rmac_locale::HourCycle::TwentyFourHour => rmac_top_bar::LocaleHourCycle::TwentyFourHour,
    }
}

enum Input {
    Shell(Box<rmac_shell_runtime::Update>),
    Locale(rmac_locale::WatchEvent),
    Time,
    ClockDeadline,
    OutputClosed,
}

/// Publish renderer-visible top-bar projections until the receiver closes.
///
/// The runtime waits for shell, locale, and clock subscriptions before its
/// first publication. Afterwards it wakes only for authority events or the
/// exact next visible clock boundary.
pub async fn watch(sender: Sender<rmac_top_bar::Update>) -> Result<(), Error> {
    let (shell_tx, shell_rx) = async_channel::bounded(8);
    let (locale_tx, locale_rx) = async_channel::bounded(4);
    let (time_tx, time_rx) = async_channel::bounded(4);

    let shell = async {
        rmac_shell_runtime::watch(shell_tx)
            .await
            .map_err(|_| Error::new("watch shell status"))
    };
    let locale = async {
        rmac_locale_linux::watch(locale_tx)
            .await
            .map_err(|_| Error::new("watch system locale"))
    };
    let time = async {
        rmac_time_linux::watch(time_tx)
            .await
            .map_err(|_| Error::new("watch system clock"))
    };
    let consumer = consume(sender, shell_rx, locale_rx, time_rx);
    let (_, _, _, _) = futures_util::try_join!(shell, locale, time, consumer)?;
    Ok(())
}

async fn consume(
    sender: Sender<rmac_top_bar::Update>,
    shell: Receiver<rmac_shell_runtime::Update>,
    locale: Receiver<rmac_locale::WatchEvent>,
    time: Receiver<rmac_time::WatchEvent>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut deadline = None;
    loop {
        let input = next_input(&sender, &shell, &locale, &time, deadline).await?;
        let now = Local::now().fixed_offset();
        let update = match input {
            Input::Shell(update) => coordinator.apply_shell(*update, now),
            Input::Locale(rmac_locale::WatchEvent::Changed) => {
                let hour_cycle = blocking::unblock(rmac_locale_linux::hour_cycle)
                    .await
                    .map_err(|_| ());
                coordinator.apply_locale(hour_cycle, now)
            }
            Input::Locale(rmac_locale::WatchEvent::Unavailable) => {
                coordinator.apply_locale(Err(()), now)
            }
            Input::Time => coordinator.apply_time_signal(now),
            Input::ClockDeadline => coordinator.clock_deadline(now),
            Input::OutputClosed => return Ok(()),
        };
        if let Some(update) = update {
            deadline = Some(realtime_deadline(now, update.next_clock_update)?);
            if update.redraw && sender.send(update).await.is_err() {
                return Ok(());
            }
        }
    }
}

async fn next_input(
    sender: &Sender<rmac_top_bar::Update>,
    shell: &Receiver<rmac_shell_runtime::Update>,
    locale: &Receiver<rmac_locale::WatchEvent>,
    time: &Receiver<rmac_time::WatchEvent>,
    deadline: Option<SystemTime>,
) -> Result<Input, Error> {
    let shell = shell.recv().fuse();
    let locale = locale.recv().fuse();
    let time = time.recv().fuse();
    let closed = sender.closed().fuse();
    let timer = async move {
        match deadline {
            Some(deadline) => wait_for_deadline(deadline).await,
            None => futures_util::future::pending::<()>().await,
        }
    }
    .fuse();
    futures_util::pin_mut!(shell, locale, time, closed, timer);
    futures_util::select! {
        update = shell => update.map(|update| Input::Shell(Box::new(update))).map_err(|_| Error::new("receive shell status")),
        event = locale => event.map(Input::Locale).map_err(|_| Error::new("receive locale state")),
        event = time => event.map(|_| Input::Time).map_err(|_| Error::new("receive clock state")),
        _ = timer => Ok(Input::ClockDeadline),
        _ = closed => Ok(Input::OutputClosed),
    }
}

fn realtime_deadline(now: DateTime<FixedOffset>, delay: Duration) -> Result<SystemTime, Error> {
    let epoch_millis = u64::try_from(now.timestamp_millis())
        .map_err(|_| Error::new("calculate the top bar clock deadline"))?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(epoch_millis))
        .and_then(|now| now.checked_add(delay))
        .ok_or_else(|| Error::new("calculate the top bar clock deadline"))
}

#[cfg(target_os = "linux")]
async fn wait_for_deadline(deadline: SystemTime) {
    if rmac_time_linux::wait_until_realtime(deadline)
        .await
        .is_err()
    {
        // Retain a bounded, non-spinning fallback if timerfd is unavailable.
        // The live clock-change watcher still replaces this sleep when it can.
        wait_with_monotonic_fallback(deadline).await;
    }
}

#[cfg(not(target_os = "linux"))]
async fn wait_for_deadline(deadline: SystemTime) {
    wait_with_monotonic_fallback(deadline).await;
}

async fn wait_with_monotonic_fallback(deadline: SystemTime) {
    let delay = deadline
        .duration_since(SystemTime::now())
        .unwrap_or(Duration::ZERO);
    async_io::Timer::after(delay).await;
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn at(hour: u32, minute: u32, second: u32, millis: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(5 * 3600 + 30 * 60)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 19, hour, minute, second)
            .single()
            .unwrap()
            + chrono::Duration::milliseconds(i64::from(millis))
    }

    fn shell_update(settings: rmac_shell_settings::ClockSettings) -> rmac_shell_runtime::Update {
        let mut snapshot = rmac_shell_runtime::Snapshot::default();
        snapshot.status.clock = settings;
        rmac_shell_runtime::Update {
            snapshot,
            visible: true,
            quick_settings_visible: false,
        }
    }

    #[test]
    fn first_projection_waits_for_every_authority() {
        let mut coordinator = Coordinator::default();
        let now = at(21, 7, 8, 250);
        assert!(coordinator
            .apply_shell(shell_update(Default::default()), now)
            .is_none());
        assert!(coordinator
            .apply_locale(Ok(rmac_locale::HourCycle::TwelveHour), now)
            .is_none());
        let update = coordinator.apply_time_signal(now).unwrap();
        assert!(update.redraw);
        assert!(update.projection.content.clock.visible.contains("9:07 PM"));
    }

    #[test]
    fn locale_failure_uses_then_preserves_a_last_known_good_cycle() {
        let mut coordinator = Coordinator::default();
        let now = at(21, 7, 8, 0);
        coordinator.apply_shell(shell_update(Default::default()), now);
        coordinator.apply_time_signal(now);
        let fallback = coordinator.apply_locale(Err(()), now).unwrap();
        assert!(fallback.projection.content.clock.visible.contains("21:07"));

        let twelve = coordinator
            .apply_locale(Ok(rmac_locale::HourCycle::TwelveHour), now)
            .unwrap();
        assert!(twelve.redraw);
        assert!(twelve.projection.content.clock.visible.contains("9:07 PM"));
        let unavailable = coordinator.apply_locale(Err(()), now).unwrap();
        assert!(!unavailable.redraw);
        assert_eq!(unavailable.projection, twelve.projection);
    }

    #[test]
    fn clock_deadlines_rearm_exactly_without_idle_frames() {
        let mut coordinator = Coordinator::default();
        let now = at(21, 7, 8, 250);
        coordinator.apply_shell(shell_update(Default::default()), now);
        coordinator.apply_locale(Ok(rmac_locale::HourCycle::TwentyFourHour), now);
        coordinator.apply_time_signal(now);

        let duplicate = coordinator.clock_deadline(now).unwrap();
        assert!(!duplicate.redraw);
        assert_eq!(duplicate.next_clock_update, Duration::from_millis(51_750));
        let next_minute = coordinator.clock_deadline(at(21, 8, 0, 0)).unwrap();
        assert!(next_minute.redraw);
        assert_eq!(next_minute.next_clock_update, Duration::from_secs(60));
    }

    #[test]
    fn enabling_seconds_replaces_the_minute_deadline() {
        let mut coordinator = Coordinator::default();
        let now = at(21, 7, 8, 250);
        coordinator.apply_locale(Ok(rmac_locale::HourCycle::TwentyFourHour), now);
        coordinator.apply_time_signal(now);
        let initial = coordinator
            .apply_shell(shell_update(Default::default()), now)
            .unwrap();
        assert_eq!(initial.next_clock_update, Duration::from_millis(51_750));

        let settings = rmac_shell_settings::ClockSettings {
            show_seconds: true,
            ..Default::default()
        };
        let seconds = coordinator
            .apply_shell(shell_update(settings), now)
            .unwrap();
        assert!(seconds.redraw);
        assert_eq!(seconds.next_clock_update, Duration::from_millis(750));
    }

    #[test]
    fn realtime_target_is_the_exact_visible_boundary() {
        let target = realtime_deadline(at(21, 7, 8, 250), Duration::from_millis(51_750)).unwrap();
        let target_millis = target.duration_since(UNIX_EPOCH).unwrap().as_millis();
        assert_eq!(target_millis % 60_000, 0);
    }
}
