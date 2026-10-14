pub(super) struct ChargeThresholdUpdate {
    pub(super) result: std::result::Result<rmac_power::Snapshot, rmac_power::Error>,
    pub(super) recovery: Option<rmac_power::Snapshot>,
}

pub(super) fn apply_charge_threshold(
    threshold: &rmac_power::ChargeThreshold,
    enabled: bool,
) -> ChargeThresholdUpdate {
    match rmac_power::set_charge_threshold(threshold, enabled) {
        Ok(snapshot) => ChargeThresholdUpdate {
            result: Ok(snapshot),
            recovery: None,
        },
        Err(error) => ChargeThresholdUpdate {
            result: Err(error),
            recovery: rmac_power::snapshot().ok(),
        },
    }
}

pub(super) fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours} hr {minutes} min")
    } else {
        format!("{minutes} min")
    }
}

pub(super) fn charge_threshold_description(threshold: &rmac_power::ChargeThreshold) -> String {
    match (threshold.start_percent, threshold.end_percent) {
        (Some(start), Some(end)) => {
            format!("Starts charging below {start}% and stops at {end}%")
        }
        (None, Some(end)) => format!("Stops charging at {end}%"),
        (Some(start), None) => format!("Starts charging below {start}%"),
        (None, None) if threshold.firmware_managed => {
            "Uses optimized limits selected by this computer's firmware".to_owned()
        }
        (None, None) => "Uses the charge limits reported by UPower".to_owned(),
    }
}

/// One bar of the Battery Level chart: the charge at the end of its slot,
/// and whether the battery charged during it (the Mac's charging marker).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LevelBar {
    pub(super) level: Option<u8>,
    pub(super) charging: bool,
}

fn is_charging(state: rmac_power::BatteryState) -> bool {
    matches!(
        state,
        rmac_power::BatteryState::Charging | rmac_power::BatteryState::PendingCharge
    )
}

/// `buckets` slots of `bucket_seconds` from `start`; a slot with no sample
/// carries the last known level forward once the record has begun.
pub(super) fn level_bars(
    points: &[rmac_power::BatteryHistoryPoint],
    start: u64,
    bucket_seconds: u64,
    buckets: usize,
) -> Vec<LevelBar> {
    let mut previous = points
        .iter()
        .take_while(|point| point.timestamp < start)
        .last()
        .map(|point| point.percentage);
    (0..buckets as u64)
        .map(|bucket| {
            let from = start + bucket * bucket_seconds;
            let to = from + bucket_seconds;
            let mut charging = false;
            let mut last = None;
            for point in points
                .iter()
                .filter(|point| point.timestamp >= from && point.timestamp < to)
            {
                charging |= is_charging(point.state);
                last = Some(point.percentage);
            }
            let level = last.or(previous);
            previous = level;
            LevelBar { level, charging }
        })
        .collect()
}

/// The last time the battery charged and the level it reached: the latest
/// charging (or fully charged) sample.
pub(super) fn last_charge(points: &[rmac_power::BatteryHistoryPoint]) -> Option<(u8, u64)> {
    points
        .iter()
        .rev()
        .find(|point| {
            is_charging(point.state) || point.state == rmac_power::BatteryState::FullyCharged
        })
        .map(|point| (point.percentage, point.timestamp))
}

/// "Today, 8:26 AM", "Yesterday, 10:20 PM" or "12 Oct, 9:05 AM", in local
/// time, as the Mac writes under "Last charged to".
pub(super) fn charged_when(timestamp: u64, now: u64) -> String {
    use chrono::{Local, TimeZone as _};
    let (Some(at), Some(today)) = (
        Local.timestamp_opt(timestamp as i64, 0).single(),
        Local.timestamp_opt(now as i64, 0).single(),
    ) else {
        return String::new();
    };
    let time = at.format("%-I:%M %p").to_string();
    match (today.date_naive() - at.date_naive()).num_days() {
        0 => format!("Today, {time}"),
        1 => format!("Yesterday, {time}"),
        _ => format!("{}, {time}", at.format("%-d %b")),
    }
}

/// The local midnight at or before `timestamp`.
pub(super) fn local_day_start(timestamp: u64) -> u64 {
    use chrono::{Local, TimeZone as _};
    Local
        .timestamp_opt(timestamp as i64, 0)
        .single()
        .and_then(|at| at.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|midnight| Local.from_local_datetime(&midnight).earliest())
        .map_or(timestamp - timestamp % 86_400, |midnight| {
            midnight.timestamp().max(0) as u64
        })
}

/// The Mac's hour axis: "12 A", "3", "6", "9", "12 P"…
pub(super) fn hour_tick(timestamp: u64) -> String {
    use chrono::{Local, TimeZone as _, Timelike as _};
    let Some(at) = Local.timestamp_opt(timestamp as i64, 0).single() else {
        return String::new();
    };
    match at.hour() {
        0 => "12 A".to_owned(),
        12 => "12 P".to_owned(),
        hour => (hour % 12).to_string(),
    }
}

/// Whether `timestamp` is a local whole hour divisible by three (the 24-hour
/// axis's ticks: 12 A, 3, 6, 9, 12 P…).
pub(super) fn is_three_hour_mark(timestamp: u64) -> bool {
    use chrono::{Local, TimeZone as _, Timelike as _};
    Local
        .timestamp_opt(timestamp as i64, 0)
        .single()
        .is_some_and(|at| at.minute() == 0 && at.second() == 0 && at.hour() % 3 == 0)
}

/// "13 Oct" for the day axis.
pub(super) fn day_tick(timestamp: u64) -> String {
    use chrono::{Local, TimeZone as _};
    Local
        .timestamp_opt(timestamp as i64, 0)
        .single()
        .map(|at| at.format("%-d %b").to_string())
        .unwrap_or_default()
}

pub(super) fn degradation_label(reason: &str) -> String {
    match reason {
        "lap-detected" => "Limited while the computer is on a lap".to_string(),
        "high-operating-temperature" => "Limited because of high temperature".to_string(),
        _ => "Limited by the system".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formats_hours_minutes_and_bare_minutes() {
        assert_eq!(format_duration(0), "0 min");
        assert_eq!(format_duration(59), "0 min");
        assert_eq!(format_duration(60), "1 min");
        assert_eq!(format_duration(45 * 60), "45 min");
        assert_eq!(format_duration(3600), "1 hr 0 min");
        assert_eq!(format_duration(3600 * 2 + 60 * 17), "2 hr 17 min");
    }

    #[test]
    fn level_bars_carry_the_charge_and_mark_charging_slots() {
        use rmac_power::BatteryState::*;
        let point = |timestamp, percentage, state| rmac_power::BatteryHistoryPoint {
            timestamp,
            percentage,
            state,
        };
        let points = [
            point(50, 90, Discharging),
            point(120, 80, Discharging),
            point(250, 85, Charging),
        ];
        let bars = level_bars(&points, 100, 100, 3);
        assert_eq!(
            bars[0],
            LevelBar {
                level: Some(80),
                charging: false
            }
        );
        assert_eq!(
            bars[1],
            LevelBar {
                level: Some(85),
                charging: true
            }
        );
        assert_eq!(
            bars[2],
            LevelBar {
                level: Some(85),
                charging: false
            }
        );
        assert_eq!(level_bars(&points[2..], 0, 100, 1)[0].level, None);
        assert_eq!(last_charge(&points), Some((85, 250)));
    }

    #[test]
    fn optimized_charging_explains_authoritative_limits() {
        let mut threshold = rmac_power::ChargeThreshold::default();
        threshold.start_percent = Some(40);
        threshold.end_percent = Some(80);
        assert_eq!(
            charge_threshold_description(&threshold),
            "Starts charging below 40% and stops at 80%"
        );
        threshold.start_percent = None;
        threshold.end_percent = None;
        threshold.firmware_managed = true;
        assert_eq!(
            charge_threshold_description(&threshold),
            "Uses optimized limits selected by this computer's firmware"
        );
    }
}
