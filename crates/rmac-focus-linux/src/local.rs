//! The Focus authority inside one process, for Windows (ADR 0023, "Focus
//! on Windows").
//!
//! On Lulo OS Focus is a session-bus service (`service`). Windows has no
//! session bus; there `lulo-shell` runs Control Centre, the menu bar and
//! Notification Centre in one process, so it [`install`]s the same
//! runtime (the same store, modes, schedules and delivery rules) here, and
//! every `client` call in that process is answered by it instead of the
//! bus. Notification Centre asks it what each Lulo notification may do
//! ([`Authority::enforce`]), so Focus silences Lulo's banners. Windows'
//! own Do Not Disturb has no public API, so other apps' toasts are
//! Windows' business.
//!
//! Nothing polls: a timed Focus ("for 1 hour") or a schedule boundary
//! wakes one sleeping thread at that moment, which evaluates again.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use async_channel::Sender;
use rmac_focus::{ActivationSource, Config, ModeId};
use rmac_focus_runtime::{wake_delay, ClockSampler, Projection, Runtime, Update};
use rmac_focus_store::Store;
use rmac_notifications::{AppId, DeliveryPolicy};

use crate::client::Error;
use crate::service::{self, WireState};

/// The mode a plain "Focus on" turns on (the service's default).
const DEFAULT_MODE: &str = "do-not-disturb";

pub struct Authority {
    runtime: Mutex<Runtime>,
    clock: ClockSampler,
    watchers: Mutex<Vec<Sender<Result<Projection, String>>>>,
    /// Bumped by every evaluation, so only the newest wake runs.
    wake: AtomicU64,
}

static AUTHORITY: OnceLock<Authority> = OnceLock::new();

/// Run Focus in this process from `store`. Once per process; later calls
/// keep the first.
pub fn install(store: Store) -> Result<(), Error> {
    let clock = ClockSampler::default();
    let (runtime, update) = Runtime::load(store, clock.sample()).map_err(|_| Error::Invalid)?;
    let installed = AUTHORITY.set(Authority {
        runtime: Mutex::new(runtime),
        clock,
        watchers: Mutex::new(Vec::new()),
        wake: AtomicU64::new(0),
    });
    if installed.is_ok() {
        if let Some(authority) = get() {
            authority.schedule(&update);
        }
    }
    Ok(())
}

/// This process's Focus authority, once installed.
pub fn get() -> Option<&'static Authority> {
    AUTHORITY.get()
}

impl Authority {
    fn runtime(&self) -> MutexGuard<'_, Runtime> {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn state(&self) -> WireState {
        service::wire_state(&self.runtime())
    }

    pub(crate) fn projection(&self) -> Projection {
        let runtime = self.runtime();
        let status = runtime.status();
        Projection {
            enabled: status.active,
            mode_name: status.mode_name.clone(),
            ends_at_unix_ms: status.ends_at_unix_ms,
        }
    }

    pub(crate) fn configuration(&self) -> Config {
        self.runtime().config().clone()
    }

    /// Change the state with `change`, then tell the watchers and set the
    /// next wake.
    fn mutate(
        &'static self,
        change: impl FnOnce(&mut Runtime, rmac_focus::ClockSample) -> Result<Update, Error>,
    ) -> Result<WireState, Error> {
        let (state, update) = {
            let mut runtime = self.runtime();
            let update = change(&mut runtime, self.clock.sample())?;
            (service::wire_update(&runtime, &update), update)
        };
        self.publish(&update);
        self.schedule(&update);
        Ok(state)
    }

    pub(crate) fn set_enabled(&'static self, enabled: bool) -> Result<WireState, Error> {
        if enabled {
            if self.runtime().status().active {
                return Ok(self.state());
            }
            let mode = ModeId::parse(DEFAULT_MODE).map_err(|_| Error::Invalid)?;
            return self.mutate(|runtime, clock| {
                runtime
                    .activate_indefinitely(mode, clock)
                    .map_err(|_| Error::Invalid)
            });
        }
        if matches!(
            self.runtime().status().source,
            Some(ActivationSource::Schedule(_))
        ) {
            return Err(Error::Scheduled);
        }
        self.disable()
    }

    pub(crate) fn activate(
        &'static self,
        mode_id: &str,
        duration_ms: u64,
    ) -> Result<WireState, Error> {
        let mode = ModeId::parse(mode_id).map_err(|_| Error::Invalid)?;
        self.mutate(|runtime, clock| {
            if duration_ms == 0 {
                runtime.activate_indefinitely(mode, clock)
            } else {
                runtime.activate_for(mode, duration_ms, clock)
            }
            .map_err(|_| Error::Invalid)
        })
    }

    pub(crate) fn disable(&'static self) -> Result<WireState, Error> {
        let state = self
            .mutate(|runtime, clock| runtime.disable_manual(clock).map_err(|_| Error::Invalid))?;
        // Off by hand while a schedule still holds Focus on.
        if service::activation_source(&state)
            .ok()
            .flatten()
            .is_some_and(|source| matches!(source, ActivationSource::Schedule(_)))
        {
            return Err(Error::Scheduled);
        }
        Ok(state)
    }

    pub(crate) fn replace_configuration(
        &'static self,
        configuration: &Config,
    ) -> Result<WireState, Error> {
        let configuration = configuration.clone();
        self.mutate(|runtime, clock| {
            runtime
                .replace_config(configuration, clock)
                .map_err(|_| Error::Invalid)
        })
    }

    /// What a notification from `app_id` may do while Focus is as it is.
    pub fn enforce(&self, app_id: &AppId, base: DeliveryPolicy) -> DeliveryPolicy {
        self.runtime().enforce(app_id, base)
    }

    /// Send `sender` the state now and on every change, until it closes.
    pub(crate) fn subscribe(&self, sender: Sender<Result<Projection, String>>) {
        if sender.try_send(Ok(self.projection())).is_ok() {
            self.watchers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(sender);
        }
    }

    fn publish(&self, update: &Update) {
        let mut watchers = self
            .watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        watchers.retain(|watcher| {
            !watcher.is_closed() && watcher.try_send(Ok(update.projection.clone())).is_ok()
        });
    }

    /// Wake once when Focus next changes by itself (a timed Focus ends, a
    /// schedule starts or stops), and evaluate again then.
    fn schedule(&'static self, update: &Update) {
        let generation = self.wake.fetch_add(1, Ordering::AcqRel) + 1;
        let now = self.clock.sample().unix_ms;
        let Some(delay) = wake_delay(update, now) else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("lulo-focus-wake".into())
            .spawn(move || {
                std::thread::sleep(delay.max(Duration::from_millis(1)));
                if self.wake.load(Ordering::Acquire) != generation {
                    return;
                }
                let update = {
                    let mut runtime = self.runtime();
                    match runtime.apply_clock(self.clock.sample()) {
                        Ok(update) => update,
                        Err(_) => return,
                    }
                };
                self.publish(&update);
                self.schedule(&update);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority(name: &str) -> &'static Authority {
        let path = std::env::temp_dir().join(format!(
            "lulo-focus-local-{name}-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let clock = ClockSampler::default();
        let (runtime, _) = Runtime::load(Store::at(path), clock.sample()).unwrap();
        Box::leak(Box::new(Authority {
            runtime: Mutex::new(runtime),
            clock,
            watchers: Mutex::new(Vec::new()),
            wake: AtomicU64::new(0),
        }))
    }

    #[test]
    fn focus_turns_on_and_off_and_tells_its_watchers() {
        let authority = authority("toggle");
        let (sender, receiver) = async_channel::unbounded();
        authority.subscribe(sender);
        assert!(!receiver.try_recv().unwrap().unwrap().enabled);
        let on = authority.set_enabled(true).unwrap();
        assert!(on.0);
        assert!(receiver.try_recv().unwrap().unwrap().enabled);
        let off = authority.set_enabled(false).unwrap();
        assert!(!off.0);
        assert!(!receiver.try_recv().unwrap().unwrap().enabled);
    }

    #[test]
    fn focus_silences_a_lulo_banner() {
        let authority = authority("enforce");
        let app = AppId::parse("org.rmac.Mail").unwrap();
        let base = DeliveryPolicy::default();
        assert!(!authority.enforce(&app, base).focus_active);
        authority.set_enabled(true).unwrap();
        assert!(authority.enforce(&app, base).focus_active);
    }

    #[test]
    fn a_timed_focus_reports_when_it_ends() {
        let authority = authority("timed");
        let state = authority.activate(DEFAULT_MODE, 60_000).unwrap();
        assert!(state.0);
        assert!(state.3 > 0);
    }
}
