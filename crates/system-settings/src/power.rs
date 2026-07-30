pub(super) struct ChargeThresholdUpdate {
    pub(super) result: std::result::Result<rmac_power::Snapshot, rmac_power::Error>,
    pub(super) recovery: Option<rmac_power::Snapshot>,
}

pub(super) fn apply_profile(
    profile: rmac_power::PowerProfile,
) -> std::result::Result<rmac_power::Snapshot, rmac_power::Error> {
    rmac_power::set_profile(profile)?;
    rmac_power::snapshot()
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

pub(super) fn sample_history(
    points: &[rmac_power::BatteryHistoryPoint],
    limit: usize,
) -> Vec<rmac_power::BatteryHistoryPoint> {
    if points.len() <= limit {
        return points.to_vec();
    }
    if limit == 0 {
        return Vec::new();
    }
    if limit == 1 {
        return points.last().copied().into_iter().collect();
    }
    (0..limit)
        .map(|index| points[index * (points.len() - 1) / (limit - 1)])
        .collect()
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
    fn history_sampling_preserves_the_time_range() {
        let points = (0..100)
            .map(|timestamp| rmac_power::BatteryHistoryPoint {
                timestamp,
                percentage: timestamp as u8,
                state: rmac_power::BatteryState::Discharging,
            })
            .collect::<Vec<_>>();
        let samples = sample_history(&points, 12);
        assert_eq!(samples.len(), 12);
        assert_eq!(samples.first().map(|point| point.timestamp), Some(0));
        assert_eq!(samples.last().map(|point| point.timestamp), Some(99));
        assert!(samples
            .windows(2)
            .all(|points| points[0].timestamp < points[1].timestamp));
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
