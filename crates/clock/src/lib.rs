//! rmac Clock: World Clock, Alarms, Stopwatch and Timers. The pure time
//! arithmetic, persistence and ring scheduling live here, shared by the
//! window and the `--ring-due` process, and are unit-tested.

pub mod alarms;
pub mod changes;
pub mod cities;
pub mod countdown;
pub mod format;
pub mod map;
pub mod metrics;
pub mod schedule;
pub mod solar;
pub mod stopwatch;
pub mod store;
pub mod tz;

/// Unix time in milliseconds.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
