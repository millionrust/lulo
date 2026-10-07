//! The menu bar clock, formatted as the Mac's ("Tue 7 Oct  14:05").

use chrono::{Datelike, NaiveDateTime, Timelike};

/// How the user's Windows locale writes dates and times.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClockStyle {
    /// "Tue Oct 7" rather than "Tue 7 Oct".
    pub month_first: bool,
    /// "14:05" rather than "2:05 PM".
    pub twenty_four_hour: bool,
}

impl ClockStyle {
    /// The style for a Windows locale name and its short-time pattern
    /// (`LOCALE_STIMEFORMAT`, such as "HH:mm" or "h:mm tt").
    pub fn from_locale(locale: &str, time_format: &str) -> Self {
        Self {
            month_first: locale.eq_ignore_ascii_case("en-US"),
            twenty_four_hour: time_format.contains('H'),
        }
    }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The clock text for `now`.
pub fn clock_text(now: NaiveDateTime, style: ClockStyle) -> String {
    let weekday = WEEKDAYS[now.weekday().num_days_from_monday() as usize];
    let month = MONTHS[now.month0() as usize];
    let date = if style.month_first {
        format!("{weekday} {month} {}", now.day())
    } else {
        format!("{weekday} {} {month}", now.day())
    };
    let time = if style.twenty_four_hour {
        format!("{:02}:{:02}", now.hour(), now.minute())
    } else {
        let (pm, hour) = now.hour12();
        format!(
            "{hour}:{:02} {}",
            now.minute(),
            if pm { "PM" } else { "AM" }
        )
    };
    // The Mac sets the time two spaces after the date.
    format!("{date}  {time}")
}

/// Milliseconds from `now` to the start of the next minute, when the clock
/// next changes; the menu bar sleeps until then.
pub fn millis_to_next_minute(now: NaiveDateTime) -> u64 {
    let into_minute = u64::from(now.second()) * 1000 + u64::from(now.nanosecond() / 1_000_000);
    60_000u64.saturating_sub(into_minute.min(59_999))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(hour: u32, minute: u32, second: u32, milli: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 7)
            .unwrap()
            .and_hms_milli_opt(hour, minute, second, milli)
            .unwrap()
    }

    #[test]
    fn the_clock_reads_like_the_mac_in_both_locales() {
        let uk = ClockStyle::from_locale("en-GB", "HH:mm");
        assert_eq!(clock_text(at(14, 5, 0, 0), uk), "Wed 7 Oct  14:05");
        let us = ClockStyle::from_locale("en-US", "h:mm tt");
        assert_eq!(clock_text(at(14, 5, 0, 0), us), "Wed Oct 7  2:05 PM");
        assert_eq!(clock_text(at(0, 9, 0, 0), us), "Wed Oct 7  12:09 AM");
    }

    #[test]
    fn the_clock_sleeps_until_the_minute_changes() {
        assert_eq!(millis_to_next_minute(at(9, 0, 0, 0)), 60_000);
        assert_eq!(millis_to_next_minute(at(9, 0, 59, 500)), 500);
        assert_eq!(millis_to_next_minute(at(9, 0, 30, 250)), 29_750);
    }
}
