//! Live Focus orchestration around persisted policy and authoritative clocks.

use std::fmt;
use std::time::Instant;

use chrono::{Datelike as _, Timelike as _};
use rmac_focus::{
    ClockSample, Config, Engine, Evaluation, ManualActivation, ModeId, Status, Wake, Weekday,
};
use rmac_focus_store::{Recovery, Store};
use rmac_notifications::{AppId, DeliveryPolicy};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceHealth {
    Healthy,
    RecoveredLastGood,
    RecoveredDefaults,
    SaveFailed,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Projection {
    pub enabled: bool,
    pub mode_name: Option<String>,
    pub ends_at_unix_ms: Option<u64>,
}

impl fmt::Debug for Projection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Projection")
            .field("enabled", &self.enabled)
            .field("mode_name", &self.mode_name.as_ref().map(|_| "<redacted>"))
            .field("ends_at_unix_ms", &self.ends_at_unix_ms)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Update {
    pub evaluation: Evaluation,
    pub projection: Projection,
    pub persistence: PersistenceHealth,
}

#[derive(Debug)]
pub struct Runtime {
    store: Store,
    config: Config,
    engine: Engine,
    persistence: PersistenceHealth,
}

impl Runtime {
    pub fn load(store: Store, clock: ClockSample) -> Result<(Self, Update), Error> {
        let snapshot = store.load().map_err(|_| Error::Load)?;
        let persistence = match snapshot.recovery {
            Recovery::None => PersistenceHealth::Healthy,
            Recovery::LastGood => PersistenceHealth::RecoveredLastGood,
            Recovery::Defaults => PersistenceHealth::RecoveredDefaults,
        };
        let config = snapshot.config;
        let mut engine =
            Engine::new(config.clone(), snapshot.manual).map_err(|_| Error::Invalid)?;
        let manual_before = engine.manual_activation().cloned();
        let evaluation = engine.evaluate(clock).map_err(|_| Error::Clock)?;
        let mut runtime = Self {
            store,
            config,
            engine,
            persistence,
        };
        if manual_before != runtime.engine.manual_activation().cloned() {
            runtime.persist();
        }
        let update = runtime.update(evaluation);
        Ok((runtime, update))
    }

    pub fn apply_clock(&mut self, clock: ClockSample) -> Result<Update, Error> {
        let manual_before = self.engine.manual_activation().cloned();
        let evaluation = self.engine.evaluate(clock).map_err(|_| Error::Clock)?;
        if manual_before != self.engine.manual_activation().cloned() {
            self.persist();
        }
        Ok(self.update(evaluation))
    }

    pub fn activate_indefinitely(
        &mut self,
        mode: ModeId,
        clock: ClockSample,
    ) -> Result<Update, Error> {
        self.engine
            .activate_indefinitely(mode)
            .map_err(|_| Error::Invalid)?;
        self.persist();
        self.apply_clock(clock)
    }

    pub fn activate_for(
        &mut self,
        mode: ModeId,
        duration_ms: u64,
        clock: ClockSample,
    ) -> Result<Update, Error> {
        self.engine
            .activate_for(mode, duration_ms, clock.unix_ms)
            .map_err(|_| Error::Invalid)?;
        self.persist();
        self.apply_clock(clock)
    }

    pub fn activate_until(
        &mut self,
        mode: ModeId,
        until_unix_ms: u64,
        clock: ClockSample,
    ) -> Result<Update, Error> {
        self.engine
            .activate_until(mode, until_unix_ms, clock.unix_ms)
            .map_err(|_| Error::Invalid)?;
        self.persist();
        self.apply_clock(clock)
    }

    pub fn disable_manual(&mut self, clock: ClockSample) -> Result<Update, Error> {
        self.engine.disable_manual();
        self.persist();
        self.apply_clock(clock)
    }

    pub fn replace_config(&mut self, config: Config, clock: ClockSample) -> Result<Update, Error> {
        let manual = self
            .engine
            .manual_activation()
            .filter(|manual| config.mode(&manual.mode).is_some())
            .cloned();
        self.engine = Engine::new(config.clone(), manual).map_err(|_| Error::Invalid)?;
        self.config = config;
        self.persist();
        self.apply_clock(clock)
    }

    pub fn enforce(&self, app_id: &AppId, base: DeliveryPolicy) -> DeliveryPolicy {
        self.engine.enforce(app_id, base)
    }

    pub fn status(&self) -> &Status {
        self.engine.status()
    }

    pub fn manual_activation(&self) -> Option<&ManualActivation> {
        self.engine.manual_activation()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn persistence_health(&self) -> PersistenceHealth {
        self.persistence
    }

    fn persist(&mut self) {
        self.persistence = if self
            .store
            .save(&self.config, self.engine.manual_activation())
            .is_ok()
        {
            PersistenceHealth::Healthy
        } else {
            PersistenceHealth::SaveFailed
        };
    }

    fn update(&self, evaluation: Evaluation) -> Update {
        Update {
            projection: projection(&evaluation.status),
            evaluation,
            persistence: self.persistence,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Load,
    Invalid,
    Clock,
}

fn projection(status: &Status) -> Projection {
    Projection {
        enabled: status.active,
        mode_name: status.mode_name.clone(),
        ends_at_unix_ms: status.ends_at_unix_ms,
    }
}

#[derive(Debug)]
pub struct ClockSampler {
    started: Instant,
}

impl Default for ClockSampler {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl ClockSampler {
    pub fn sample(&self) -> ClockSample {
        let now = chrono::Local::now();
        let unix_ms = now.timestamp_millis().max(0) as u64;
        ClockSample {
            unix_ms,
            monotonic_ms: self
                .started
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            weekday: weekday(now.weekday()),
            minute_of_day: (now.hour() * 60 + now.minute()) as u16,
            next_minute_unix_ms: unix_ms
                .div_euclid(60_000)
                .saturating_add(1)
                .saturating_mul(60_000),
        }
    }
}

fn weekday(day: chrono::Weekday) -> Weekday {
    match day {
        chrono::Weekday::Mon => Weekday::Monday,
        chrono::Weekday::Tue => Weekday::Tuesday,
        chrono::Weekday::Wed => Weekday::Wednesday,
        chrono::Weekday::Thu => Weekday::Thursday,
        chrono::Weekday::Fri => Weekday::Friday,
        chrono::Weekday::Sat => Weekday::Saturday,
        chrono::Weekday::Sun => Weekday::Sunday,
    }
}

pub fn wake_delay(update: &Update, now_unix_ms: u64) -> Option<std::time::Duration> {
    match update.evaluation.wake {
        Wake::None => None,
        Wake::AtUnixMs(wake) => Some(std::time::Duration::from_millis(
            wake.saturating_sub(now_unix_ms),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_focus_store::default_config;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!(
                "rmac-focus-runtime-{}-{label}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ))
            .join("focus.json")
    }

    fn clock(unix_ms: u64, monotonic_ms: u64, minute: u16) -> ClockSample {
        ClockSample {
            unix_ms,
            monotonic_ms,
            weekday: Weekday::Monday,
            minute_of_day: minute,
            next_minute_unix_ms: unix_ms + 60_000,
        }
    }

    #[test]
    fn mutation_persists_and_restart_expires_temporary_focus() {
        let path = path("restart");
        let store = Store::at(path.clone());
        let (mut runtime, _) = Runtime::load(store.clone(), clock(1_000, 1_000, 100)).unwrap();
        let update = runtime
            .activate_for(
                ModeId::parse("work").unwrap(),
                5_000,
                clock(1_000, 1_000, 100),
            )
            .unwrap();
        assert!(update.projection.enabled);
        assert_eq!(update.projection.ends_at_unix_ms, Some(6_000));
        drop(runtime);

        let (restarted, expired) = Runtime::load(store, clock(7_000, 2_000, 100)).unwrap();
        assert!(!expired.projection.enabled);
        assert!(restarted.manual_activation().is_none());
        assert_eq!(restarted.persistence_health(), PersistenceHealth::Healthy);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn replacing_config_drops_only_a_removed_manual_mode() {
        let path = path("replace");
        let store = Store::at(path.clone());
        let (mut runtime, _) = Runtime::load(store, clock(1_000, 1_000, 100)).unwrap();
        runtime
            .activate_indefinitely(ModeId::parse("work").unwrap(), clock(1_000, 1_000, 100))
            .unwrap();
        let do_not_disturb = default_config()
            .unwrap()
            .modes()
            .find(|mode| mode.id().as_str() == "do-not-disturb")
            .unwrap()
            .clone();
        let config = Config::new(vec![do_not_disturb], Vec::new()).unwrap();
        let update = runtime
            .replace_config(config, clock(2_000, 2_000, 100))
            .unwrap();
        assert!(!update.projection.enabled);
        assert!(runtime.manual_activation().is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn projection_and_debug_redact_mode_name() {
        let path = path("redaction");
        let store = Store::at(path.clone());
        let (mut runtime, _) = Runtime::load(store, clock(1_000, 1_000, 100)).unwrap();
        let update = runtime
            .activate_indefinitely(ModeId::parse("personal").unwrap(), clock(1_000, 1_000, 100))
            .unwrap();
        assert_eq!(update.projection.mode_name.as_deref(), Some("Personal"));
        assert!(!format!("{:?}", update.projection).contains("Personal"));
        assert_eq!(wake_delay(&update, 1_000), None);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn clock_sampler_returns_a_valid_local_minute_boundary() {
        let sample = ClockSampler::default().sample();
        assert!(sample.validate().is_ok());
        assert!(sample.next_minute_unix_ms > sample.unix_ms);
    }
}
