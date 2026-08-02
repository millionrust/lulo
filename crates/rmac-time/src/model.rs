use crate::{validate_timezone_syntax, ClockTarget, Error, ErrorKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub timezone: String,
    pub local_rtc: bool,
    pub can_ntp: bool,
    pub ntp_enabled: bool,
    pub synchronized: bool,
    pub time_usec: u64,
    pub timezones: Vec<String>,
    pub timezones_truncated: bool,
}

impl Snapshot {
    pub fn formatted_local_time(&self) -> String {
        self.formatted_local_time_at(self.time_usec)
    }

    pub fn formatted_local_time_at(&self, time_usec: u64) -> String {
        use chrono::{Local, TimeZone as _};

        let seconds = (time_usec / 1_000_000).min(i64::MAX as u64) as i64;
        let nanoseconds = ((time_usec % 1_000_000) * 1_000) as u32;
        Local
            .timestamp_opt(seconds, nanoseconds)
            .single()
            .map(|time| time.format("%A, %B %-d, %Y at %-I:%M %p").to_string())
            .unwrap_or_else(|| "Unavailable".into())
    }

    pub fn clock_input(&self) -> Option<String> {
        self.clock_input_at(self.time_usec)
    }

    pub fn clock_input_at(&self, time_usec: u64) -> Option<String> {
        use chrono::{Local, TimeZone as _};

        let seconds = i64::try_from(time_usec / 1_000_000).ok()?;
        let nanoseconds = ((time_usec % 1_000_000) * 1_000) as u32;
        Local
            .timestamp_opt(seconds, nanoseconds)
            .single()
            .map(|time| time.format("%Y-%m-%d %H:%M:%S %:z").to_string())
    }

    pub fn validate_timezone(&self, timezone: &str) -> Result<(), Error> {
        validate_timezone_syntax(timezone)?;
        if self.timezones.iter().any(|candidate| candidate == timezone) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidTimezone,
                "the time zone is not known to this system",
            ))
        }
    }
}

pub trait Service {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_ntp(&self, enabled: bool) -> Result<Snapshot, Error>;
    fn set_timezone(&self, timezone: &str) -> Result<Snapshot, Error>;
    fn set_time(&self, target: &ClockTarget) -> Result<Snapshot, Error>;
}
