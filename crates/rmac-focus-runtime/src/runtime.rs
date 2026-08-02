use rmac_focus::{ClockSample, Config, Engine, Evaluation, ManualActivation, ModeId, Status};
use rmac_focus_store::{Recovery, Store};
use rmac_notifications::{AppId, DeliveryPolicy};

use crate::{Error, PersistenceHealth, Projection, Update};

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

fn projection(status: &Status) -> Projection {
    Projection {
        enabled: status.active,
        mode_name: status.mode_name.clone(),
        ends_at_unix_ms: status.ends_at_unix_ms,
    }
}
