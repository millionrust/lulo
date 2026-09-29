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
    /// Mac style ("25 Sep 2026 at 12:00:26 PM", or "…at 12:00:26" with
    /// `twenty_four_hour`), not the locale-formatted long date the OS locale
    /// would otherwise pick (SET-78): the Mac's Date & Time pane always
    /// shows day, abbreviated month, year and seconds, regardless of locale.
    pub fn formatted_local_time(&self, twenty_four_hour: bool) -> String {
        self.formatted_local_time_at(self.time_usec, twenty_four_hour)
    }

    pub fn formatted_local_time_at(&self, time_usec: u64, twenty_four_hour: bool) -> String {
        use chrono::{Local, TimeZone as _};

        let seconds = (time_usec / 1_000_000).min(i64::MAX as u64) as i64;
        let nanoseconds = ((time_usec % 1_000_000) * 1_000) as u32;
        Local
            .timestamp_opt(seconds, nanoseconds)
            .single()
            .map(|time| format_mac_date_time(time, twenty_four_hour))
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

/// The Mac's Date & Time date/time string, independent of the ambient
/// `Local` timezone so it stays testable: day, abbreviated month, year, and
/// seconds, with the hour in either the 24-hour or the AM/PM style
/// (SET-78).
pub(crate) fn format_mac_date_time<Tz: chrono::TimeZone>(
    time: chrono::DateTime<Tz>,
    twenty_four_hour: bool,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let time_format = if twenty_four_hour {
        "%H:%M:%S"
    } else {
        "%-I:%M:%S %p"
    };
    let format = format!("%-d %b %Y at {time_format}");
    time.format(&format).to_string()
}
