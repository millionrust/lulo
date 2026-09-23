//! Text for clock faces, cards and pins.

use crate::tz::civil_from_days;

/// Wall-clock fields of an instant at a fixed offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WallTime {
    /// Days since 1970-01-01 in that zone.
    pub days: i64,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl WallTime {
    pub fn at(utc: i64, offset: i32) -> Self {
        let local = utc + i64::from(offset);
        let seconds = local.rem_euclid(86_400);
        Self {
            days: local.div_euclid(86_400),
            hour: (seconds / 3600) as u32,
            minute: (seconds % 3600 / 60) as u32,
            second: (seconds % 60) as u32,
        }
    }

    /// 0 = Monday … 6 = Sunday.
    pub fn weekday(&self) -> u32 {
        // 1970-01-01 was a Thursday (index 3).
        (self.days + 3).rem_euclid(7) as u32
    }

    pub fn date(&self) -> (i64, u32, u32) {
        civil_from_days(self.days)
    }
}

/// "12:14 PM", the way the Mac's World Clock and menu bar write times.
pub fn time_12h(hour: u32, minute: u32) -> String {
    let (display, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{display}:{minute:02} {suffix}")
}

/// "Today", "Yesterday" or "Tomorrow" for a city relative to here.
pub fn relative_day(city_days: i64, local_days: i64) -> &'static str {
    match city_days - local_days {
        ..=-1 => "Yesterday",
        0 => "Today",
        _ => "Tomorrow",
    }
}

/// "+0 HRS", "+1 HR", "-4 HRS", "+5:30 HRS": the city's offset from here.
pub fn offset_difference(city_offset: i32, local_offset: i32) -> String {
    let difference = city_offset - local_offset;
    let sign = if difference < 0 { '-' } else { '+' };
    let magnitude = difference.unsigned_abs();
    let hours = magnitude / 3600;
    let minutes = magnitude % 3600 / 60;
    let unit = if hours == 1 && minutes == 0 {
        "HR"
    } else {
        "HRS"
    };
    if minutes == 0 {
        format!("{sign}{hours} {unit}")
    } else {
        format!("{sign}{hours}:{minutes:02} {unit}")
    }
}

/// Hand angles in degrees clockwise from 12 for an analogue face; the hour
/// and minute hands sweep, the second hand ticks.
pub fn hand_angles(time: WallTime) -> (f32, f32, f32) {
    let seconds = time.second as f32;
    let minutes = time.minute as f32 + seconds / 60.0;
    let hours = (time.hour % 12) as f32 + minutes / 60.0;
    (hours * 30.0, minutes * 6.0, seconds * 6.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_time_and_weekday() {
        // 2026-09-23 06:44:30 UTC is 12:14:30 in New Delhi, a Wednesday.
        let utc = 20_719 * 86_400 + 6 * 3600 + 44 * 60 + 30;
        let delhi = WallTime::at(utc, 19_800);
        assert_eq!((delhi.hour, delhi.minute, delhi.second), (12, 14, 30));
        assert_eq!(delhi.weekday(), 2);
        assert_eq!(delhi.date(), (2026, 9, 23));
        // Honolulu is still on the 22nd.
        let honolulu = WallTime::at(utc, -36_000);
        assert_eq!(honolulu.days, 20_718);
        assert_eq!(relative_day(honolulu.days, delhi.days), "Yesterday");
        assert_eq!(relative_day(delhi.days + 1, delhi.days), "Tomorrow");
        assert_eq!(relative_day(delhi.days, delhi.days), "Today");
    }

    #[test]
    fn twelve_hour_times() {
        assert_eq!(time_12h(0, 5), "12:05 AM");
        assert_eq!(time_12h(9, 0), "9:00 AM");
        assert_eq!(time_12h(12, 14), "12:14 PM");
        assert_eq!(time_12h(23, 59), "11:59 PM");
    }

    #[test]
    fn offset_differences() {
        assert_eq!(offset_difference(19_800, 19_800), "+0 HRS");
        assert_eq!(offset_difference(23_400, 19_800), "+1 HR");
        assert_eq!(offset_difference(-14_400, 19_800), "-9:30 HRS");
        assert_eq!(offset_difference(19_800, 0), "+5:30 HRS");
        assert_eq!(offset_difference(-7_200, 0), "-2 HRS");
    }

    #[test]
    fn hands_sweep() {
        let time = WallTime {
            days: 0,
            hour: 15,
            minute: 30,
            second: 15,
        };
        let (hour, minute, second) = hand_angles(time);
        assert!((hour - 105.125).abs() < 1e-3);
        assert!((minute - 181.5).abs() < 1e-3);
        assert_eq!(second, 90.0);
    }
}
