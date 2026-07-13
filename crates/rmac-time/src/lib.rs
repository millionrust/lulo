//! Platform-neutral date, time-zone, and synchronization state.

use std::fmt;

pub const MAX_TIMEZONES: usize = 1024;
const MAX_TIMEZONE_BYTES: usize = 128;

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
        use chrono::{Local, TimeZone as _};

        let seconds = (self.time_usec / 1_000_000).min(i64::MAX as u64) as i64;
        let nanoseconds = ((self.time_usec % 1_000_000) * 1_000) as u32;
        Local
            .timestamp_opt(seconds, nanoseconds)
            .single()
            .map(|time| time.format("%A, %B %-d, %Y at %-I:%M:%S %p").to_string())
            .unwrap_or_else(|| "Unavailable".into())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidTimezone,
    Unavailable,
    Authorization,
    Mutation,
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    detail: String,
}

impl Error {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
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
}
