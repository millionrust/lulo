//! Alarm scheduling: when an alarm next rings and whether it is due now.
//!
//! Alarms are local wall-clock times. Conversions take the local zone's
//! offset function so the arithmetic stays pure and testable.

use serde::{Deserialize, Serialize};

use crate::format::WallTime;

/// How late (seconds) a missed ring may still sound — covers a machine
/// that wakes from sleep just after the alarm time.
pub const LATE_GRACE: i64 = 10 * 60;
/// How early (seconds) the ring process may treat an alarm as due, for
/// timer jitter.
pub const EARLY_GRACE: i64 = 30;
/// The Mac's snooze length.
pub const SNOOZE_SECONDS: i64 = 9 * 60;

/// Repeat days as a bit set, bit 0 = Monday … bit 6 = Sunday.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Days(pub u8);

impl Days {
    pub const NAMES: [&'static str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    pub const EVERY_DAY: Self = Self(0x7F);
    pub const WEEKDAYS: Self = Self(0x1F);
    pub const WEEKENDS: Self = Self(0x60);

    pub fn contains(self, weekday: u32) -> bool {
        weekday < 7 && self.0 & (1 << weekday) != 0
    }

    pub fn toggled(self, weekday: u32) -> Self {
        Self((self.0 ^ (1 << weekday.min(6))) & 0x7F)
    }

    pub fn is_empty(self) -> bool {
        self.0 & 0x7F == 0
    }

    /// The row subtitle: "Every day", "Weekdays", "Weekends", "Mon, Wed" or
    /// nothing for a one-time alarm.
    pub fn label(self) -> String {
        match Self(self.0 & 0x7F) {
            Self::EVERY_DAY => "Every day".into(),
            Self::WEEKDAYS => "Weekdays".into(),
            Self::WEEKENDS => "Weekends".into(),
            days => (0..7)
                .filter(|&day| days.contains(day))
                .map(|day| Self::NAMES[day as usize])
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    /// systemd calendar day list: "Mon,Wed" (empty when every day).
    pub fn calendar(self) -> String {
        if self.is_empty() || self.0 & 0x7F == 0x7F {
            return String::new();
        }
        (0..7)
            .filter(|&day| self.contains(day))
            .map(|day| Self::NAMES[day as usize])
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Alarm {
    pub id: u64,
    pub hour: u8,
    pub minute: u8,
    pub label: String,
    pub repeat: Days,
    pub enabled: bool,
    pub snooze: bool,
    /// Unix seconds a snoozed alarm rings again.
    pub snoozed_until: Option<i64>,
    /// Unix seconds of the last occurrence that rang.
    pub last_fired: Option<i64>,
}

impl Default for Alarm {
    fn default() -> Self {
        Self {
            id: 0,
            hour: 7,
            minute: 0,
            label: String::new(),
            repeat: Days::default(),
            enabled: true,
            snooze: true,
            snoozed_until: None,
            last_fired: None,
        }
    }
}

/// Why the ring process is sounding an alarm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ring {
    /// The scheduled occurrence at this Unix second.
    Scheduled(i64),
    /// A snooze that ends at this Unix second.
    Snoozed(i64),
}

impl Alarm {
    pub fn title(&self) -> &str {
        if self.label.trim().is_empty() {
            "Alarm"
        } else {
            self.label.trim()
        }
    }

    /// Unix seconds of this alarm's time on local day `days`.
    pub fn occurrence_on(&self, days: i64, offset_at: &impl Fn(i64) -> i32) -> i64 {
        let wall = days * 86_400 + i64::from(self.hour) * 3600 + i64::from(self.minute) * 60;
        local_to_utc(wall, offset_at)
    }

    fn rings_on(&self, days: i64) -> bool {
        self.repeat.is_empty()
            || self
                .repeat
                .contains(WallTime::at(days * 86_400, 0).weekday())
    }

    /// The next scheduled ring strictly after `now`, if enabled.
    pub fn next_occurrence(&self, now: i64, offset_at: &impl Fn(i64) -> i32) -> Option<i64> {
        if !self.enabled {
            return None;
        }
        let today = WallTime::at(now, offset_at(now)).days;
        let scheduled = (today - 1..=today + 8)
            .filter(|&days| self.rings_on(days))
            .map(|days| self.occurrence_on(days, offset_at))
            .find(|&time| time > now);
        match (scheduled, self.snoozed_until.filter(|&until| until > now)) {
            (Some(time), Some(until)) => Some(time.min(until)),
            (time, until) => time.or(until),
        }
    }

    /// Whether the alarm should ring at `now`, and why.
    pub fn due(&self, now: i64, offset_at: &impl Fn(i64) -> i32) -> Option<Ring> {
        if !self.enabled {
            return None;
        }
        let window = |time: i64| time <= now + EARLY_GRACE && time >= now - LATE_GRACE;
        if let Some(until) = self.snoozed_until.filter(|&until| window(until)) {
            return Some(Ring::Snoozed(until));
        }
        let today = WallTime::at(now, offset_at(now)).days;
        (today - 1..=today + 1)
            .filter(|&days| self.rings_on(days))
            .map(|days| self.occurrence_on(days, offset_at))
            .filter(|&time| window(time))
            .filter(|&time| self.last_fired.is_none_or(|fired| time > fired))
            .max()
            .map(Ring::Scheduled)
    }

    /// Record that `ring` sounded: one-time alarms switch off, snoozes clear.
    pub fn mark_rung(&mut self, ring: Ring) {
        if let Ring::Scheduled(time) = ring {
            self.last_fired = Some(time);
        }
        if self.repeat.is_empty() {
            self.enabled = false;
        }
        self.snoozed_until = None;
    }

    /// Snooze from `now`: ring again nine minutes later, even for a
    /// one-time alarm that has just switched itself off.
    pub fn snooze_from(&mut self, now: i64) {
        self.enabled = true;
        self.snoozed_until = Some(now + SNOOZE_SECONDS);
    }
}

/// Convert local wall seconds to UTC. A wall time repeated by a fall-back
/// change resolves to its first instant; one skipped by spring-forward
/// resolves to the instant after the jump that reads one gap later.
pub fn local_to_utc(wall: i64, offset_at: &impl Fn(i64) -> i32) -> i64 {
    let guess = wall - i64::from(offset_at(wall));
    let corrected = wall - i64::from(offset_at(guess));
    if corrected + i64::from(offset_at(corrected)) == wall {
        corrected
    } else if guess + i64::from(offset_at(guess)) == wall {
        guess
    } else {
        corrected.max(guess)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IST: i32 = 19_800;
    // 2026-09-23 (a Wednesday) 00:00 IST.
    const WEDNESDAY: i64 = 20_719 * 86_400 - IST as i64;

    fn ist(_: i64) -> i32 {
        IST
    }

    fn at(day_offset: i64, hour: i64, minute: i64) -> i64 {
        WEDNESDAY + day_offset * 86_400 + hour * 3600 + minute * 60
    }

    fn alarm(hour: u8, minute: u8, repeat: Days) -> Alarm {
        Alarm {
            id: 1,
            hour,
            minute,
            repeat,
            ..Alarm::default()
        }
    }

    #[test]
    fn day_labels_and_calendar() {
        assert_eq!(Days::EVERY_DAY.label(), "Every day");
        assert_eq!(Days::WEEKDAYS.label(), "Weekdays");
        assert_eq!(Days::WEEKENDS.label(), "Weekends");
        assert_eq!(Days(0b101).label(), "Mon, Wed");
        assert_eq!(Days(0).label(), "");
        assert_eq!(Days(0b101).calendar(), "Mon,Wed");
        assert_eq!(Days::EVERY_DAY.calendar(), "");
        assert_eq!(Days(0).toggled(6), Days::WEEKENDS.toggled(5));
    }

    #[test]
    fn one_time_alarm_rings_at_the_next_matching_time() {
        let seven = alarm(7, 0, Days(0));
        // At 12:14 on Wednesday the next 7:00 is Thursday.
        assert_eq!(
            seven.next_occurrence(at(0, 12, 14), &ist),
            Some(at(1, 7, 0))
        );
        // At 6:59 it is today.
        assert_eq!(seven.next_occurrence(at(0, 6, 59), &ist), Some(at(0, 7, 0)));
        let off = Alarm {
            enabled: false,
            ..seven
        };
        assert_eq!(off.next_occurrence(at(0, 6, 0), &ist), None);
    }

    #[test]
    fn repeating_alarm_skips_other_days() {
        // Weekends only: from Wednesday the next is Saturday.
        let weekend = alarm(9, 30, Days::WEEKENDS);
        assert_eq!(
            weekend.next_occurrence(at(0, 10, 0), &ist),
            Some(at(3, 9, 30))
        );
        // Monday only, from Wednesday: the Monday five days later.
        let monday = alarm(8, 0, Days(1));
        assert_eq!(
            monday.next_occurrence(at(0, 10, 0), &ist),
            Some(at(5, 8, 0))
        );
    }

    #[test]
    fn due_window_and_deduplication() {
        let mut seven = alarm(7, 0, Days(0));
        assert_eq!(seven.due(at(0, 6, 58), &ist), None);
        assert_eq!(
            seven.due(at(0, 6, 59) + 45, &ist),
            Some(Ring::Scheduled(at(0, 7, 0)))
        );
        assert_eq!(
            seven.due(at(0, 7, 9), &ist),
            Some(Ring::Scheduled(at(0, 7, 0)))
        );
        assert_eq!(seven.due(at(0, 7, 11), &ist), None); // too late
        seven.mark_rung(Ring::Scheduled(at(0, 7, 0)));
        assert!(!seven.enabled); // one-time alarms switch off
        let mut daily = alarm(7, 0, Days::EVERY_DAY);
        daily.mark_rung(Ring::Scheduled(at(0, 7, 0)));
        assert!(daily.enabled);
        assert_eq!(daily.due(at(0, 7, 1), &ist), None); // already rang
        assert_eq!(
            daily.due(at(1, 7, 0), &ist),
            Some(Ring::Scheduled(at(1, 7, 0)))
        );
    }

    #[test]
    fn alarm_just_after_midnight_is_found_across_the_day_boundary() {
        let late = alarm(23, 59, Days(0));
        assert_eq!(
            late.due(at(1, 0, 3), &ist),
            Some(Ring::Scheduled(at(0, 23, 59)))
        );
    }

    #[test]
    fn snooze_rings_nine_minutes_later() {
        let mut seven = alarm(7, 0, Days(0));
        seven.mark_rung(Ring::Scheduled(at(0, 7, 0)));
        seven.snooze_from(at(0, 7, 0));
        assert!(seven.enabled);
        assert_eq!(seven.next_occurrence(at(0, 7, 1), &ist), Some(at(0, 7, 9)));
        assert_eq!(
            seven.due(at(0, 7, 9), &ist),
            Some(Ring::Snoozed(at(0, 7, 9)))
        );
        seven.mark_rung(Ring::Snoozed(at(0, 7, 9)));
        assert_eq!(seven.snoozed_until, None);
        assert!(!seven.enabled); // a one-time alarm is done after its snooze
        let mut daily = alarm(7, 0, Days::EVERY_DAY);
        daily.snooze_from(at(0, 7, 0));
        daily.mark_rung(Ring::Snoozed(at(0, 7, 9)));
        assert!(daily.enabled);
    }

    #[test]
    fn dst_gap_rings_after_the_jump() {
        // A zone at +0 that springs to +1h at wall 02:00 (UTC 02:00).
        let jump = 2 * 3600;
        let offset = |utc: i64| if utc >= jump { 3600 } else { 0 };
        // 01:30 local exists: UTC 01:30.
        assert_eq!(local_to_utc(5_400, &offset), 5_400);
        // 02:30 local does not exist; ring at the first valid instant.
        let utc = local_to_utc(9_000, &offset);
        assert!(utc >= jump && utc <= jump + 3600);
        // 03:30 local is UTC 02:30.
        assert_eq!(local_to_utc(12_600, &offset), 9_000);
    }

    #[test]
    fn titles_and_persistence() {
        let mut seven = alarm(7, 0, Days::WEEKDAYS);
        assert_eq!(seven.title(), "Alarm");
        seven.label = "  Gym ".into();
        assert_eq!(seven.title(), "Gym");
        let json = serde_json::to_string(&seven).unwrap();
        assert_eq!(serde_json::from_str::<Alarm>(&json).unwrap(), seven);
        let partial: Alarm = serde_json::from_str(r#"{"id":3,"hour":6}"#).unwrap();
        assert_eq!((partial.id, partial.hour, partial.minute), (3, 6, 0));
        assert!(partial.enabled && partial.snooze);
    }
}
