//! Timers: the hr/min/sec entry and running countdowns. Times are Unix
//! milliseconds so a timer keeps running (and rings) after Clock quits.

use serde::{Deserialize, Serialize};

/// The three editable groups of the "00:15:00" entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    Hours,
    Minutes,
    Seconds,
}

/// The duration being set up; the Mac starts at 15 minutes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Entry {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            hours: 0,
            minutes: 15,
            seconds: 0,
        }
    }
}

impl Entry {
    fn limit(field: Field) -> u8 {
        match field {
            Field::Hours => 23,
            Field::Minutes | Field::Seconds => 59,
        }
    }

    fn slot(&mut self, field: Field) -> &mut u8 {
        match field {
            Field::Hours => &mut self.hours,
            Field::Minutes => &mut self.minutes,
            Field::Seconds => &mut self.seconds,
        }
    }

    /// Scroll or arrow-key step, wrapping within the field.
    pub fn step(&mut self, field: Field, delta: i32) {
        let modulus = i32::from(Self::limit(field)) + 1;
        let slot = self.slot(field);
        *slot = (i32::from(*slot) + delta).rem_euclid(modulus) as u8;
    }

    /// Typing a digit shifts it in from the right ("1" then "5" gives 15);
    /// a value past the field's limit starts over with the new digit.
    pub fn type_digit(&mut self, field: Field, digit: u8) {
        let limit = Self::limit(field);
        let slot = self.slot(field);
        let shifted = (*slot % 10) * 10 + digit.min(9);
        *slot = if shifted <= limit {
            shifted
        } else {
            digit.min(9)
        };
    }

    pub fn milliseconds(&self) -> u64 {
        (u64::from(self.hours) * 3600 + u64::from(self.minutes) * 60 + u64::from(self.seconds))
            * 1000
    }

    pub fn is_zero(&self) -> bool {
        self.milliseconds() == 0
    }

    pub fn text(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hours, self.minutes, self.seconds)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum RunState {
    Running { ends_at: u64 },
    Paused { remaining: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Countdown {
    pub id: u64,
    pub duration: u64,
    #[serde(default)]
    pub label: String,
    #[serde(flatten)]
    pub run: RunState,
}

impl Countdown {
    pub fn start(id: u64, duration: u64, now: u64) -> Self {
        Self {
            id,
            duration,
            label: String::new(),
            run: RunState::Running {
                ends_at: now + duration,
            },
        }
    }

    pub fn remaining(&self, now: u64) -> u64 {
        match self.run {
            RunState::Running { ends_at } => ends_at.saturating_sub(now),
            RunState::Paused { remaining } => remaining,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.run, RunState::Running { .. })
    }

    pub fn is_done(&self, now: u64) -> bool {
        self.is_running() && self.remaining(now) == 0
    }

    /// When it rings, if running.
    pub fn ends_at(&self) -> Option<u64> {
        match self.run {
            RunState::Running { ends_at } => Some(ends_at),
            RunState::Paused { .. } => None,
        }
    }

    pub fn pause(&mut self, now: u64) {
        if let RunState::Running { ends_at } = self.run {
            self.run = RunState::Paused {
                remaining: ends_at.saturating_sub(now),
            };
        }
    }

    pub fn resume(&mut self, now: u64) {
        if let RunState::Paused { remaining } = self.run {
            self.run = RunState::Running {
                ends_at: now + remaining,
            };
        }
    }

    /// Fraction of the ring still left, 1 → 0.
    pub fn fraction_left(&self, now: u64) -> f32 {
        if self.duration == 0 {
            return 0.0;
        }
        (self.remaining(now) as f64 / self.duration as f64) as f32
    }
}

/// Countdown text: "14:59" under an hour, "1:02:03" above; rounds up so a
/// timer never shows 0:00 while it still has time left.
pub fn remaining_text(milliseconds: u64) -> String {
    let seconds = milliseconds.div_ceil(1000);
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_defaults_to_fifteen_minutes() {
        let entry = Entry::default();
        assert_eq!(entry.text(), "00:15:00");
        assert_eq!(entry.milliseconds(), 900_000);
    }

    #[test]
    fn entry_steps_wrap_and_digits_shift_in() {
        let mut entry = Entry::default();
        entry.step(Field::Minutes, -16);
        assert_eq!(entry.minutes, 59);
        entry.step(Field::Hours, -1);
        assert_eq!(entry.hours, 23);
        entry.step(Field::Seconds, 61);
        assert_eq!(entry.seconds, 1);
        let mut entry = Entry {
            hours: 0,
            minutes: 0,
            seconds: 0,
        };
        assert!(entry.is_zero());
        entry.type_digit(Field::Minutes, 4);
        entry.type_digit(Field::Minutes, 5);
        assert_eq!(entry.minutes, 45);
        entry.type_digit(Field::Minutes, 7); // 57 fits
        assert_eq!(entry.minutes, 57);
        entry.type_digit(Field::Minutes, 9); // 79 does not: start over
        assert_eq!(entry.minutes, 9);
        entry.type_digit(Field::Hours, 3);
        entry.type_digit(Field::Hours, 5); // 35 > 23
        assert_eq!(entry.hours, 5);
    }

    #[test]
    fn countdown_pause_resume_and_finish() {
        let mut timer = Countdown::start(1, 60_000, 1_000);
        assert_eq!(timer.remaining(31_000), 30_000);
        assert!((timer.fraction_left(31_000) - 0.5).abs() < 1e-6);
        timer.pause(31_000);
        assert_eq!(timer.remaining(99_000), 30_000);
        assert_eq!(timer.ends_at(), None);
        timer.resume(100_000);
        assert_eq!(timer.ends_at(), Some(130_000));
        assert!(!timer.is_done(129_999));
        assert!(timer.is_done(130_000));
        assert_eq!(timer.remaining(200_000), 0);
    }

    #[test]
    fn remaining_text_rounds_up() {
        assert_eq!(remaining_text(900_000), "15:00");
        assert_eq!(remaining_text(899_001), "15:00");
        assert_eq!(remaining_text(899_000), "14:59");
        assert_eq!(remaining_text(1), "00:01");
        assert_eq!(remaining_text(0), "00:00");
        assert_eq!(remaining_text(3_723_000), "1:02:03");
    }

    #[test]
    fn countdown_serialises_flat() {
        let timer = Countdown::start(7, 5_000, 10);
        let json = serde_json::to_string(&timer).unwrap();
        assert!(json.contains("\"state\":\"running\""));
        assert!(json.contains("\"ends_at\":5010"));
        assert_eq!(serde_json::from_str::<Countdown>(&json).unwrap(), timer);
    }
}
