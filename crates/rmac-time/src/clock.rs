use crate::{Error, ErrorKind};

const MAX_CLOCK_INPUT_BYTES: usize = 32;
const CLOCK_READBACK_TOLERANCE_USEC: u64 = 5_000_000;

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

pub fn clock_readback_matches(
    target_usec: u64,
    observed_usec: u64,
    elapsed: std::time::Duration,
) -> bool {
    let elapsed_usec = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    let expected = target_usec.saturating_add(elapsed_usec);
    expected.abs_diff(observed_usec) <= CLOCK_READBACK_TOLERANCE_USEC
}

fn invalid_clock_error() -> Error {
    Error::new(
        ErrorKind::InvalidTime,
        "enter date, time, and UTC offset as 2026-07-18 11:30:00 +05:30",
    )
}
