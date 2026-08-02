use std::time::Instant;

use chrono::{Datelike as _, Timelike as _};
use rmac_focus::{ClockSample, Wake, Weekday};

use crate::Update;

#[derive(Debug)]
pub struct ClockSampler {
    started: Instant,
}

impl Default for ClockSampler {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl ClockSampler {
    pub fn sample(&self) -> ClockSample {
        let now = chrono::Local::now();
        let unix_ms = now.timestamp_millis().max(0) as u64;
        ClockSample {
            unix_ms,
            monotonic_ms: self
                .started
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            weekday: weekday(now.weekday()),
            minute_of_day: (now.hour() * 60 + now.minute()) as u16,
            next_minute_unix_ms: unix_ms
                .div_euclid(60_000)
                .saturating_add(1)
                .saturating_mul(60_000),
        }
    }
}

fn weekday(day: chrono::Weekday) -> Weekday {
    match day {
        chrono::Weekday::Mon => Weekday::Monday,
        chrono::Weekday::Tue => Weekday::Tuesday,
        chrono::Weekday::Wed => Weekday::Wednesday,
        chrono::Weekday::Thu => Weekday::Thursday,
        chrono::Weekday::Fri => Weekday::Friday,
        chrono::Weekday::Sat => Weekday::Saturday,
        chrono::Weekday::Sun => Weekday::Sunday,
    }
}

pub fn wake_delay(update: &Update, now_unix_ms: u64) -> Option<std::time::Duration> {
    match update.evaluation.wake {
        Wake::None => None,
        Wake::AtUnixMs(wake) => Some(std::time::Duration::from_millis(
            wake.saturating_sub(now_unix_ms),
        )),
    }
}
