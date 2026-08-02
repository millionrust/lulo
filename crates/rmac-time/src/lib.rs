//! Platform-neutral date, time-zone, and synchronization state.

mod clock;
mod error;
mod model;
mod timezone;

pub use clock::{clock_readback_matches, ClockTarget};
pub use error::{Error, ErrorKind};
pub use model::{Service, Snapshot, WatchEvent};
pub use timezone::{normalize_timezones, validate_timezone_syntax, MAX_TIMEZONES};

#[cfg(test)]
mod tests;
