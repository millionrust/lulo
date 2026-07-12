//! Bounded client-side keyboard repeat scheduling.

use std::fmt;
use std::time::{Duration, Instant};

pub const MAX_REPEAT_RATE: u32 = 100;
pub const MAX_REPEAT_DELAY_MS: u32 = 10_000;

pub struct RepeatScheduler<K> {
    config: RepeatConfig,
    held: Option<HeldKey<K>>,
    deadline: Option<Instant>,
}

impl<K> Default for RepeatScheduler<K> {
    fn default() -> Self {
        Self {
            config: RepeatConfig::default(),
            held: None,
            deadline: None,
        }
    }
}

impl<K: Copy + Eq> RepeatScheduler<K> {
    pub fn configure(&mut self, rate: u32, delay_ms: u32, now: Instant) -> RepeatConfig {
        self.config = RepeatConfig {
            rate: rate.min(MAX_REPEAT_RATE),
            delay_ms: delay_ms.min(MAX_REPEAT_DELAY_MS),
        };
        self.deadline = self
            .held
            .filter(|held| held.repeatable && self.config.rate > 0)
            .map(|_| now + self.config.delay());
        self.config
    }

    pub fn pressed(&mut self, key: K, repeatable: bool, now: Instant) {
        if !repeatable {
            return;
        }
        self.held = Some(HeldKey { key, repeatable });
        self.deadline = (repeatable && self.config.rate > 0).then(|| now + self.config.delay());
    }

    pub fn released(&mut self, key: K) {
        if self.held.is_some_and(|held| held.key == key) {
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.held = None;
        self.deadline = None;
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Return at most one repeat per pump iteration. Missed intervals are not
    /// replayed in a burst after the process is descheduled.
    pub fn take_due(&mut self, now: Instant) -> Option<K> {
        if self.deadline.is_none_or(|deadline| deadline > now) {
            return None;
        }
        let held = self.held?;
        if !held.repeatable || self.config.rate == 0 {
            self.clear();
            return None;
        }
        self.deadline = Some(now + self.config.interval());
        Some(held.key)
    }
}

impl<K> fmt::Debug for RepeatScheduler<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RepeatScheduler")
            .field("config", &self.config)
            .field("held_key", &self.held.as_ref().map(|_| "<redacted>"))
            .field("deadline", &self.deadline.map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Copy)]
struct HeldKey<K> {
    key: K,
    repeatable: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RepeatConfig {
    rate: u32,
    delay_ms: u32,
}

impl RepeatConfig {
    pub fn rate(self) -> u32 {
        self.rate
    }

    pub fn delay_ms(self) -> u32 {
        self.delay_ms
    }

    fn delay(self) -> Duration {
        Duration::from_millis(u64::from(self.delay_ms))
    }

    fn interval(self) -> Duration {
        debug_assert!(self.rate > 0);
        Duration::from_nanos(1_000_000_000 / u64::from(self.rate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_one_repeat_after_delay_then_at_bounded_rate() {
        let start = Instant::now();
        let mut repeat = RepeatScheduler::default();
        assert_eq!(repeat.configure(25, 400, start).rate(), 25);
        repeat.pressed(7_u32, true, start);
        assert_eq!(repeat.take_due(start + Duration::from_millis(399)), None);
        assert_eq!(repeat.take_due(start + Duration::from_millis(400)), Some(7));
        assert_eq!(repeat.take_due(start + Duration::from_millis(439)), None);
        assert_eq!(repeat.take_due(start + Duration::from_millis(440)), Some(7));
    }

    #[test]
    fn release_focus_loss_and_nonrepeatable_keys_cancel() {
        let start = Instant::now();
        let mut repeat = RepeatScheduler::default();
        repeat.configure(30, 1, start);
        repeat.pressed(4_u32, true, start);
        repeat.released(4);
        assert_eq!(repeat.take_due(start + Duration::from_secs(1)), None);

        repeat.pressed(5, false, start);
        assert_eq!(repeat.deadline(), None);
        repeat.clear();
        assert_eq!(repeat.take_due(start + Duration::from_secs(1)), None);
    }

    #[test]
    fn hostile_settings_are_clamped_and_never_catch_up_in_bursts() {
        let start = Instant::now();
        let mut repeat = RepeatScheduler::default();
        let config = repeat.configure(u32::MAX, u32::MAX, start);
        assert_eq!(config.rate(), MAX_REPEAT_RATE);
        assert_eq!(config.delay_ms(), MAX_REPEAT_DELAY_MS);
        repeat.pressed(9_u32, true, start);
        let late = start + Duration::from_secs(20);
        assert_eq!(repeat.take_due(late), Some(9));
        assert_eq!(repeat.take_due(late), None);
        assert!(!format!("{repeat:?}").contains('9'));
    }

    #[test]
    fn zero_rate_disables_and_reconfiguration_reschedules_held_key() {
        let start = Instant::now();
        let mut repeat = RepeatScheduler::default();
        repeat.configure(0, 200, start);
        repeat.pressed(3_u32, true, start);
        assert_eq!(repeat.deadline(), None);
        repeat.configure(20, 300, start);
        assert_eq!(repeat.take_due(start + Duration::from_millis(299)), None);
        assert_eq!(repeat.take_due(start + Duration::from_millis(300)), Some(3));
    }
}
