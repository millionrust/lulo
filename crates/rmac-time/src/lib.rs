//! Platform-neutral date, time-zone, and synchronization state.

use std::fmt;

pub const MAX_TIMEZONES: usize = 1024;
const MAX_TIMEZONE_BYTES: usize = 128;
const MAX_ERROR_BYTES: usize = 512;
const MAX_CLOCK_INPUT_BYTES: usize = 32;
const CLOCK_READBACK_TOLERANCE_USEC: u64 = 5_000_000;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockTarget {
    time_usec: u64,
    display: String,
}

impl ClockTarget {
    pub fn parse(value: &str) -> Result<Self, Error> {
        use chrono::Datelike as _;

        if value != value.trim()
            || value.is_empty()
            || value.len() > MAX_CLOCK_INPUT_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(invalid_clock_error());
        }
        let parsed = chrono::DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S %:z")
            .map_err(|_| invalid_clock_error())?;
        if !(1970..=2261).contains(&parsed.year()) {
            return Err(Error::new(
                ErrorKind::InvalidTime,
                "the system clock must be between 1970 and 2261",
            ));
        }
        let display = parsed.format("%Y-%m-%d %H:%M:%S %:z").to_string();
        if display != value {
            return Err(invalid_clock_error());
        }
        let timestamp = parsed.timestamp_micros();
        let time_usec = u64::try_from(timestamp).map_err(|_| invalid_clock_error())?;
        if time_usec > i64::MAX as u64 {
            return Err(Error::new(
                ErrorKind::InvalidTime,
                "the requested system time is outside timedated's supported range",
            ));
        }
        Ok(Self { time_usec, display })
    }

    pub fn time_usec(&self) -> u64 {
        self.time_usec
    }

    pub fn display(&self) -> &str {
        &self.display
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidTimezone,
    InvalidTime,
    Unavailable,
    Authorization,
    Conflict,
    Mutation,
    Mismatch,
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            kind,
            detail: bounded_text(&detail),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for Error {}

pub trait Service {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_ntp(&self, enabled: bool) -> Result<Snapshot, Error>;
    fn set_timezone(&self, timezone: &str) -> Result<Snapshot, Error>;
    fn set_time(&self, target: &ClockTarget) -> Result<Snapshot, Error>;
}

pub fn clock_readback_matches(
    target_usec: u64,
    observed_usec: u64,
    elapsed: std::time::Duration,
) -> bool {
    let elapsed_usec = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    let expected = target_usec.saturating_add(elapsed_usec);
    expected.abs_diff(observed_usec) <= CLOCK_READBACK_TOLERANCE_USEC
}

pub fn normalize_timezones(values: Vec<String>) -> (Vec<String>, bool) {
    let mut timezones = values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim();
            (validate_timezone_syntax(value).is_ok()).then(|| value.to_string())
        })
        .collect::<Vec<_>>();
    timezones.sort_unstable();
    timezones.dedup();
    let truncated = timezones.len() > MAX_TIMEZONES;
    timezones.truncate(MAX_TIMEZONES);
    (timezones, truncated)
}

pub fn validate_timezone_syntax(timezone: &str) -> Result<(), Error> {
    if timezone.is_empty() || timezone.len() > MAX_TIMEZONE_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidTimezone,
            "enter a time zone such as Asia/Kolkata",
        ));
    }
    if timezone.starts_with('/')
        || timezone.starts_with('.')
        || timezone.ends_with('/')
        || timezone.contains("..")
        || timezone.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '/' | '_' | '-' | '+'))
        })
    {
        return Err(Error::new(
            ErrorKind::InvalidTimezone,
            "use a system time-zone name such as Asia/Kolkata",
        ));
    }
    Ok(())
}

fn invalid_clock_error() -> Error {
    Error::new(
        ErrorKind::InvalidTime,
        "enter date, time, and UTC offset as 2026-07-18 11:30:00 +05:30",
    )
}

fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_ERROR_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakeService {
        snapshot: Snapshot,
        mutation: Result<(), Error>,
    }

    impl Service for FakeService {
        fn snapshot(&self) -> Result<Snapshot, Error> {
            Ok(self.snapshot.clone())
        }

        fn set_ntp(&self, enabled: bool) -> Result<Snapshot, Error> {
            self.mutation.clone()?;
            let mut snapshot = self.snapshot.clone();
            snapshot.ntp_enabled = enabled;
            Ok(snapshot)
        }

        fn set_timezone(&self, timezone: &str) -> Result<Snapshot, Error> {
            self.snapshot.validate_timezone(timezone)?;
            self.mutation.clone()?;
            let mut snapshot = self.snapshot.clone();
            snapshot.timezone = timezone.into();
            Ok(snapshot)
        }

        fn set_time(&self, target: &ClockTarget) -> Result<Snapshot, Error> {
            self.mutation.clone()?;
            if self.snapshot.ntp_enabled {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "turn off automatic time before setting the clock manually",
                ));
            }
            let mut snapshot = self.snapshot.clone();
            snapshot.time_usec = target.time_usec();
            Ok(snapshot)
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            timezone: "UTC".into(),
            can_ntp: true,
            ntp_enabled: true,
            synchronized: true,
            timezones: vec!["Asia/Kolkata".into(), "UTC".into()],
            ..Snapshot::default()
        }
    }

    #[test]
    fn timezone_validation_rejects_paths_and_unknown_values() {
        let snapshot = snapshot();
        assert!(snapshot.validate_timezone("Asia/Kolkata").is_ok());
        assert_eq!(
            snapshot
                .validate_timezone("../etc/passwd")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidTimezone
        );
        assert_eq!(
            snapshot
                .validate_timezone("Mars/Olympus")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidTimezone
        );
    }

    #[test]
    fn timezone_inventory_is_sorted_deduplicated_and_bounded() {
        let mut values = (0..=MAX_TIMEZONES)
            .map(|index| format!("Etc/Zone_{index}"))
            .collect::<Vec<_>>();
        values.push("UTC".into());
        values.push("UTC".into());
        values.push("../invalid".into());

        let (timezones, truncated) = normalize_timezones(values);

        assert_eq!(timezones.len(), MAX_TIMEZONES);
        assert!(truncated);
        assert!(timezones.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn fake_mutations_return_refreshed_authoritative_state() {
        let service = FakeService {
            snapshot: snapshot(),
            mutation: Ok(()),
        };

        assert!(!service.set_ntp(false).unwrap().ntp_enabled);
        assert_eq!(
            service.set_timezone("Asia/Kolkata").unwrap().timezone,
            "Asia/Kolkata"
        );
    }

    #[test]
    fn failed_fake_mutation_preserves_previous_snapshot() {
        let before = snapshot();
        let service = FakeService {
            snapshot: before.clone(),
            mutation: Err(Error::new(
                ErrorKind::Authorization,
                "authorization was cancelled",
            )),
        };

        assert!(service.set_ntp(false).is_err());
        assert_eq!(service.snapshot().unwrap(), before);
    }

    #[test]
    fn clock_targets_are_unambiguous_bounded_and_canonical() {
        let target = ClockTarget::parse("2026-07-18 11:30:00 +05:30").unwrap();
        assert_eq!(target.display(), "2026-07-18 11:30:00 +05:30");
        assert_eq!(target.time_usec(), 1_784_354_400_000_000);
        assert_eq!(
            ClockTarget::parse("2026-07-18 11:30:00")
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidTime
        );
        assert!(ClockTarget::parse("1969-12-31 23:59:59 +00:00").is_err());
        assert!(ClockTarget::parse("2262-01-01 00:00:00 +00:00").is_err());
        assert!(ClockTarget::parse("2026-07-18 11:30:00 +05:30\nprivate").is_err());
        assert!(ClockTarget::parse(" 2026-07-18 11:30:00 +05:30").is_err());
        assert!(ClockTarget::parse("2026-07-18 11:30:00 +05:30\n").is_err());
    }

    #[test]
    fn clock_readback_accounts_for_transaction_elapsed_time() {
        let target = 1_700_000_000_000_000;
        assert!(clock_readback_matches(
            target,
            target + 2_500_000,
            std::time::Duration::from_millis(2_500)
        ));
        assert!(!clock_readback_matches(
            target,
            target + 30_000_000,
            std::time::Duration::from_secs(1)
        ));
    }

    #[test]
    fn manual_clock_requires_automatic_time_to_be_off() {
        let service = FakeService {
            snapshot: snapshot(),
            mutation: Ok(()),
        };
        let target = ClockTarget::parse("2026-07-18 11:30:00 +05:30").unwrap();
        assert_eq!(
            service.set_time(&target).unwrap_err().kind(),
            ErrorKind::Conflict
        );

        let mut manual_snapshot = snapshot();
        manual_snapshot.ntp_enabled = false;
        let service = FakeService {
            snapshot: manual_snapshot,
            mutation: Ok(()),
        };
        assert_eq!(
            service.set_time(&target).unwrap().time_usec,
            target.time_usec()
        );
    }
}
