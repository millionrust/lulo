use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rmac_notifications::AppId;

use crate::engine::validate_text;

const MAX_MODES: usize = 32;
const MAX_SCHEDULES: usize = 64;
const MAX_ALLOWED_APPS: usize = 256;
const MAX_ID_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 256;
pub(crate) const MAX_MANUAL_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
pub(crate) const CLOCK_JUMP_TOLERANCE_MS: u64 = 2_000;

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
    pub(crate) id: ModeId,
    pub(crate) name: String,
    pub(crate) allowed_apps: BTreeSet<AppId>,
    pub(crate) allow_urgent: bool,
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
    pub(crate) modes: BTreeMap<ModeId, Mode>,
    pub(crate) schedules: BTreeMap<ScheduleId, Schedule>,
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
