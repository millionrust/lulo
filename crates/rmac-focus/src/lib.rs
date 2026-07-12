//! Framework-neutral Focus modes, schedules, and notification enforcement.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rmac_notifications::{AppId, DeliveryPolicy};

const MAX_MODES: usize = 32;
const MAX_SCHEDULES: usize = 64;
const MAX_ALLOWED_APPS: usize = 256;
const MAX_ID_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 256;
const MAX_MANUAL_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const CLOCK_JUMP_TOLERANCE_MS: u64 = 2_000;

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ModeId(String);

impl ModeId {
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        validate_text(&value, MAX_ID_BYTES, false)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ModeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ModeId(<redacted>)")
    }
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ScheduleId(String);

impl ScheduleId {
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        validate_text(&value, MAX_ID_BYTES, false)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ScheduleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ScheduleId(<redacted>)")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Mode {
    id: ModeId,
    name: String,
    allowed_apps: BTreeSet<AppId>,
    allow_urgent: bool,
}

impl Mode {
    pub fn new(
        id: ModeId,
        name: impl Into<String>,
        allowed_apps: BTreeSet<AppId>,
        allow_urgent: bool,
    ) -> Result<Self, Error> {
        let name = name.into();
        validate_text(&name, MAX_NAME_BYTES, true)?;
        if allowed_apps.len() > MAX_ALLOWED_APPS {
            return Err(Error::Limit);
        }
        Ok(Self {
            id,
            name,
            allowed_apps,
            allow_urgent,
        })
    }

    pub fn id(&self) -> &ModeId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn allowed_apps(&self) -> &BTreeSet<AppId> {
        &self.allowed_apps
    }

    pub fn allow_urgent(&self) -> bool {
        self.allow_urgent
    }
}

impl fmt::Debug for Mode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Mode")
            .field("id", &self.id)
            .field("name", &"<redacted>")
            .field(
                "allowed_apps",
                &format_args!("<{} redacted apps>", self.allowed_apps.len()),
            )
            .field("allow_urgent", &self.allow_urgent)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    fn previous(self) -> Self {
        match self {
            Self::Monday => Self::Sunday,
            Self::Tuesday => Self::Monday,
            Self::Wednesday => Self::Tuesday,
            Self::Thursday => Self::Wednesday,
            Self::Friday => Self::Thursday,
            Self::Saturday => Self::Friday,
            Self::Sunday => Self::Saturday,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Schedule {
    pub id: ScheduleId,
    pub mode: ModeId,
    pub days: BTreeSet<Weekday>,
    /// Inclusive local start minute, 0 through 1439.
    pub start_minute: u16,
    /// Exclusive local end minute, 0 through 1439.
    pub end_minute: u16,
    pub priority: u8,
    pub enabled: bool,
}

impl fmt::Debug for Schedule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Schedule")
            .field("id", &self.id)
            .field("mode", &self.mode)
            .field("days", &self.days)
            .field("start_minute", &self.start_minute)
            .field("end_minute", &self.end_minute)
            .field("priority", &self.priority)
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl Schedule {
    pub fn validate(&self) -> Result<(), Error> {
        if self.days.is_empty()
            || self.start_minute >= 1_440
            || self.end_minute >= 1_440
            || self.start_minute == self.end_minute
        {
            return Err(Error::InvalidSchedule);
        }
        Ok(())
    }

    pub fn active(&self, weekday: Weekday, minute: u16) -> bool {
        if !self.enabled {
            return false;
        }
        if self.start_minute < self.end_minute {
            self.days.contains(&weekday) && (self.start_minute..self.end_minute).contains(&minute)
        } else {
            (self.days.contains(&weekday) && minute >= self.start_minute)
                || (self.days.contains(&weekday.previous()) && minute < self.end_minute)
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Config {
    modes: BTreeMap<ModeId, Mode>,
    schedules: BTreeMap<ScheduleId, Schedule>,
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field(
                "modes",
                &format_args!("<{} redacted modes>", self.modes.len()),
            )
            .field(
                "schedules",
                &format_args!("<{} redacted schedules>", self.schedules.len()),
            )
            .finish()
    }
}

impl Config {
    pub fn new(modes: Vec<Mode>, schedules: Vec<Schedule>) -> Result<Self, Error> {
        if modes.len() > MAX_MODES || schedules.len() > MAX_SCHEDULES {
            return Err(Error::Limit);
        }
        let mode_count = modes.len();
        let schedule_count = schedules.len();
        let modes: BTreeMap<_, _> = modes
            .into_iter()
            .map(|mode| (mode.id.clone(), mode))
            .collect();
        if modes.is_empty() {
            return Err(Error::InvalidMode);
        }
        let schedules: BTreeMap<_, _> = schedules
            .into_iter()
            .map(|schedule| (schedule.id.clone(), schedule))
            .collect();
        if modes.len() != mode_count || schedules.len() != schedule_count {
            return Err(Error::Duplicate);
        }
        for schedule in schedules.values() {
            schedule.validate()?;
            if !modes.contains_key(&schedule.mode) {
                return Err(Error::UnknownMode);
            }
        }
        Ok(Self { modes, schedules })
    }

    pub fn modes(&self) -> impl Iterator<Item = &Mode> {
        self.modes.values()
    }

    pub fn schedules(&self) -> impl Iterator<Item = &Schedule> {
        self.schedules.values()
    }

    pub fn mode(&self, id: &ModeId) -> Option<&Mode> {
        self.modes.get(id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManualActivation {
    pub mode: ModeId,
    pub until_unix_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockSample {
    pub unix_ms: u64,
    pub monotonic_ms: u64,
    pub weekday: Weekday,
    pub minute_of_day: u16,
    /// Exact next local minute boundary supplied by the timezone-aware adapter.
    pub next_minute_unix_ms: u64,
}

impl ClockSample {
    pub fn validate(self) -> Result<Self, Error> {
        if self.minute_of_day >= 1_440
            || self.next_minute_unix_ms <= self.unix_ms
            || self.next_minute_unix_ms.saturating_sub(self.unix_ms) > 120_000
        {
            return Err(Error::InvalidClock);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivationSource {
    Manual,
    Schedule(ScheduleId),
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct Status {
    pub active: bool,
    pub mode: Option<ModeId>,
    pub mode_name: Option<String>,
    pub source: Option<ActivationSource>,
    /// Exact only for a manual temporary activation. Scheduled end remains a
    /// local-time rule and is recomputed from every authoritative clock sample.
    pub ends_at_unix_ms: Option<u64>,
}

impl fmt::Debug for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Status")
            .field("active", &self.active)
            .field("mode", &self.mode)
            .field("mode_name", &self.mode_name.as_ref().map(|_| "<redacted>"))
            .field("source", &self.source)
            .field("ends_at_unix_ms", &self.ends_at_unix_ms)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wake {
    None,
    AtUnixMs(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evaluation {
    pub status: Status,
    pub changed: bool,
    pub wall_clock_jump: bool,
    pub wake: Wake,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidText,
    InvalidMode,
    InvalidSchedule,
    InvalidClock,
    InvalidDuration,
    UnknownMode,
    Duplicate,
    Limit,
}

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

fn validate_text(value: &str, max: usize, allow_spaces: bool) -> Result<(), Error> {
    if value.trim().is_empty()
        || value.len() > max
        || value.chars().any(char::is_control)
        || (!allow_spaces && value.chars().any(char::is_whitespace))
    {
        return Err(Error::InvalidText);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notifications::{BannerPolicy, HistoryPolicy};

    fn mode(id: &str, apps: &[&str], urgent: bool) -> Mode {
        Mode::new(
            ModeId::parse(id).unwrap(),
            format!("Private {id}"),
            apps.iter().map(|app| AppId::parse(*app).unwrap()).collect(),
            urgent,
        )
        .unwrap()
    }

    fn schedule(
        id: &str,
        mode: &str,
        days: &[Weekday],
        start: u16,
        end: u16,
        priority: u8,
    ) -> Schedule {
        Schedule {
            id: ScheduleId::parse(id).unwrap(),
            mode: ModeId::parse(mode).unwrap(),
            days: days.iter().copied().collect(),
            start_minute: start,
            end_minute: end,
            priority,
            enabled: true,
        }
    }

    fn clock(unix_ms: u64, monotonic_ms: u64, weekday: Weekday, minute: u16) -> ClockSample {
        ClockSample {
            unix_ms,
            monotonic_ms,
            weekday,
            minute_of_day: minute,
            next_minute_unix_ms: unix_ms + 60_000,
        }
    }

    fn base() -> DeliveryPolicy {
        DeliveryPolicy {
            enabled: true,
            banner: BannerPolicy::Allow,
            sounds: true,
            history: HistoryPolicy::Allow,
            allow_urgent_through_focus: true,
            focus_active: false,
        }
    }

    #[test]
    fn overnight_schedule_uses_previous_start_day_after_midnight() {
        let config = Config::new(
            vec![mode("sleep", &[], false)],
            vec![schedule(
                "weeknight",
                "sleep",
                &[Weekday::Monday],
                22 * 60,
                7 * 60,
                1,
            )],
        )
        .unwrap();
        let mut engine = Engine::new(config, None).unwrap();
        assert!(
            engine
                .evaluate(clock(1_000, 1_000, Weekday::Monday, 23 * 60))
                .unwrap()
                .status
                .active
        );
        assert!(
            engine
                .evaluate(clock(2_000, 2_000, Weekday::Tuesday, 6 * 60))
                .unwrap()
                .status
                .active
        );
        assert!(
            !engine
                .evaluate(clock(3_000, 3_000, Weekday::Tuesday, 8 * 60))
                .unwrap()
                .status
                .active
        );
    }

    #[test]
    fn manual_duration_overrides_schedule_then_expires_after_restart() {
        let config = Config::new(
            vec![mode("work", &[], false), mode("sleep", &[], false)],
            vec![schedule(
                "always-now",
                "sleep",
                &[Weekday::Monday],
                0,
                1_439,
                1,
            )],
        )
        .unwrap();
        let manual = ManualActivation {
            mode: ModeId::parse("work").unwrap(),
            until_unix_ms: Some(10_000),
        };
        let mut restarted = Engine::new(config, Some(manual)).unwrap();
        let active = restarted
            .evaluate(clock(9_000, 100, Weekday::Monday, 100))
            .unwrap();
        assert_eq!(active.status.mode.unwrap().as_str(), "work");
        assert_eq!(active.wake, Wake::AtUnixMs(10_000));
        let expired = restarted
            .evaluate(clock(10_000, 1_100, Weekday::Monday, 100))
            .unwrap();
        assert_eq!(expired.status.mode.unwrap().as_str(), "sleep");
        assert!(restarted.manual_activation().is_none());
    }

    #[test]
    fn higher_priority_overlap_wins_deterministically() {
        let config = Config::new(
            vec![mode("low", &[], false), mode("high", &[], false)],
            vec![
                schedule("low-rule", "low", &[Weekday::Friday], 60, 180, 1),
                schedule("high-rule", "high", &[Weekday::Friday], 60, 180, 9),
            ],
        )
        .unwrap();
        let mut engine = Engine::new(config, None).unwrap();
        assert_eq!(
            engine
                .evaluate(clock(1_000, 1_000, Weekday::Friday, 90))
                .unwrap()
                .status
                .mode
                .unwrap()
                .as_str(),
            "high"
        );
    }

    #[test]
    fn allowed_apps_bypass_focus_while_urgent_policy_is_composed() {
        let config = Config::new(
            vec![mode("work", &["org.example.Allowed"], false)],
            Vec::new(),
        )
        .unwrap();
        let mut engine = Engine::new(config, None).unwrap();
        engine
            .activate_indefinitely(ModeId::parse("work").unwrap())
            .unwrap();
        engine
            .evaluate(clock(1_000, 1_000, Weekday::Monday, 100))
            .unwrap();
        let allowed = engine.enforce(&AppId::parse("org.example.Allowed").unwrap(), base());
        assert!(!allowed.focus_active);
        let blocked = engine.enforce(&AppId::parse("org.example.Blocked").unwrap(), base());
        assert!(blocked.focus_active);
        assert!(!blocked.allow_urgent_through_focus);
    }

    #[test]
    fn wall_clock_jump_forces_recomputation_without_stale_mode() {
        let config = Config::new(
            vec![mode("work", &[], false)],
            vec![schedule(
                "morning",
                "work",
                &[Weekday::Monday],
                8 * 60,
                9 * 60,
                1,
            )],
        )
        .unwrap();
        let mut engine = Engine::new(config, None).unwrap();
        assert!(
            engine
                .evaluate(clock(1_000_000, 1_000, Weekday::Monday, 8 * 60 + 30))
                .unwrap()
                .status
                .active
        );
        let jumped = engine
            .evaluate(clock(4_600_000, 2_000, Weekday::Monday, 9 * 60 + 30))
            .unwrap();
        assert!(jumped.wall_clock_jump);
        assert!(!jumped.status.active);
        assert!(jumped.changed);
    }

    #[test]
    fn invalid_durations_schedules_and_duplicate_ids_fail_closed() {
        let duplicate = Config::new(
            vec![mode("same", &[], false), mode("same", &[], true)],
            Vec::new(),
        );
        assert_eq!(duplicate, Err(Error::Duplicate));
        let invalid = Config::new(
            vec![mode("work", &[], false)],
            vec![schedule("bad", "work", &[Weekday::Monday], 100, 100, 1)],
        );
        assert_eq!(invalid, Err(Error::InvalidSchedule));

        let config = Config::new(vec![mode("work", &[], false)], Vec::new()).unwrap();
        let mut engine = Engine::new(config, None).unwrap();
        assert_eq!(
            engine.activate_for(ModeId::parse("work").unwrap(), 0, 1_000),
            Err(Error::InvalidDuration)
        );
    }

    #[test]
    fn debug_output_redacts_mode_names_ids_and_apps() {
        let config = Config::new(
            vec![mode("private-mode-8472", &["org.private.App8472"], false)],
            Vec::new(),
        )
        .unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("private-mode-8472"));
        assert!(!debug.contains("App8472"));
    }
}
