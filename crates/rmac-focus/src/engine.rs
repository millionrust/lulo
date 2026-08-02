use rmac_notifications::{AppId, DeliveryPolicy};

use crate::{
    ActivationSource, ClockSample, Config, Error, Evaluation, ManualActivation, ModeId, Status,
    Wake, CLOCK_JUMP_TOLERANCE_MS, MAX_MANUAL_DURATION_MS,
};

#[derive(Debug)]
pub struct Engine {
    config: Config,
    manual: Option<ManualActivation>,
    status: Status,
    last_clock: Option<ClockSample>,
}

impl Engine {
    pub fn new(config: Config, manual: Option<ManualActivation>) -> Result<Self, Error> {
        if manual
            .as_ref()
            .is_some_and(|manual| config.mode(&manual.mode).is_none())
        {
            return Err(Error::UnknownMode);
        }
        Ok(Self {
            config,
            manual,
            status: Status::default(),
            last_clock: None,
        })
    }

    pub fn manual_activation(&self) -> Option<&ManualActivation> {
        self.manual.as_ref()
    }

    pub fn activate_indefinitely(&mut self, mode: ModeId) -> Result<(), Error> {
        self.ensure_mode(&mode)?;
        self.manual = Some(ManualActivation {
            mode,
            until_unix_ms: None,
        });
        Ok(())
    }

    pub fn activate_for(
        &mut self,
        mode: ModeId,
        duration_ms: u64,
        now_unix_ms: u64,
    ) -> Result<(), Error> {
        if duration_ms == 0 || duration_ms > MAX_MANUAL_DURATION_MS {
            return Err(Error::InvalidDuration);
        }
        self.ensure_mode(&mode)?;
        self.manual = Some(ManualActivation {
            mode,
            until_unix_ms: Some(now_unix_ms.saturating_add(duration_ms)),
        });
        Ok(())
    }

    pub fn activate_until(
        &mut self,
        mode: ModeId,
        until_unix_ms: u64,
        now_unix_ms: u64,
    ) -> Result<(), Error> {
        let duration = until_unix_ms.saturating_sub(now_unix_ms);
        self.activate_for(mode, duration, now_unix_ms)
    }

    pub fn disable_manual(&mut self) {
        self.manual = None;
    }

    pub fn evaluate(&mut self, clock: ClockSample) -> Result<Evaluation, Error> {
        let clock = clock.validate()?;
        let wall_clock_jump = self
            .last_clock
            .is_some_and(|previous| clock_jump(previous, clock));
        self.last_clock = Some(clock);
        if self
            .manual
            .as_ref()
            .and_then(|manual| manual.until_unix_ms)
            .is_some_and(|until| until <= clock.unix_ms)
        {
            self.manual = None;
        }
        let status = self.resolve_status(clock);
        let changed = status != self.status;
        self.status = status.clone();
        Ok(Evaluation {
            status,
            changed,
            wall_clock_jump,
            wake: self.next_wake(clock),
        })
    }

    pub fn enforce(&self, app_id: &AppId, mut base: DeliveryPolicy) -> DeliveryPolicy {
        let Some(mode_id) = self.status.mode.as_ref() else {
            base.focus_active = false;
            return base;
        };
        let Some(mode) = self.config.mode(mode_id) else {
            base.focus_active = false;
            return base;
        };
        if mode.allowed_apps.contains(app_id) {
            base.focus_active = false;
        } else {
            base.focus_active = true;
            base.allow_urgent_through_focus &= mode.allow_urgent;
        }
        base
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    fn resolve_status(&self, clock: ClockSample) -> Status {
        let selected = if let Some(manual) = &self.manual {
            Some((
                manual.mode.clone(),
                ActivationSource::Manual,
                manual.until_unix_ms,
            ))
        } else {
            self.config
                .schedules
                .values()
                .filter(|schedule| schedule.active(clock.weekday, clock.minute_of_day))
                .max_by(|left, right| {
                    left.priority
                        .cmp(&right.priority)
                        .then_with(|| left.id.cmp(&right.id))
                })
                .map(|schedule| {
                    (
                        schedule.mode.clone(),
                        ActivationSource::Schedule(schedule.id.clone()),
                        None,
                    )
                })
        };
        let Some((mode_id, source, ends_at)) = selected else {
            return Status::default();
        };
        let mode = self
            .config
            .mode(&mode_id)
            .expect("validated mode reference");
        Status {
            active: true,
            mode: Some(mode_id),
            mode_name: Some(mode.name.clone()),
            source: Some(source),
            ends_at_unix_ms: ends_at,
        }
    }

    fn next_wake(&self, clock: ClockSample) -> Wake {
        let manual = self
            .manual
            .as_ref()
            .and_then(|manual| manual.until_unix_ms)
            .filter(|until| *until > clock.unix_ms);
        let schedule = (self.manual.is_none() && !self.config.schedules.is_empty())
            .then_some(clock.next_minute_unix_ms);
        manual
            .into_iter()
            .chain(schedule)
            .min()
            .map(Wake::AtUnixMs)
            .unwrap_or(Wake::None)
    }

    fn ensure_mode(&self, mode: &ModeId) -> Result<(), Error> {
        self.config.mode(mode).map(|_| ()).ok_or(Error::UnknownMode)
    }
}

fn clock_jump(previous: ClockSample, current: ClockSample) -> bool {
    let wall_delta = current.unix_ms.abs_diff(previous.unix_ms);
    let monotonic_delta = current.monotonic_ms.abs_diff(previous.monotonic_ms);
    wall_delta.abs_diff(monotonic_delta) > CLOCK_JUMP_TOLERANCE_MS
}

pub(crate) fn validate_text(value: &str, max: usize, allow_spaces: bool) -> Result<(), Error> {
    if value.trim().is_empty()
        || value.len() > max
        || value.chars().any(char::is_control)
        || (!allow_spaces && value.chars().any(char::is_whitespace))
    {
        return Err(Error::InvalidText);
    }
    Ok(())
}
