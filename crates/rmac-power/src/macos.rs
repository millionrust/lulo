//! macOS development power backend and fixture parsers.

use super::*;

#[cfg(target_os = "macos")]
pub(super) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    sender
        .send(WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch power changes", "the event consumer closed"))
}

#[cfg(target_os = "macos")]
pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
    let pmset = command("pmset", &["-g", "batt"], "read battery state")?;
    let ioreg = command(
        "ioreg",
        &["-rn", "AppleSmartBattery"],
        "read battery health",
    )
    .unwrap_or_default();
    Ok(Snapshot {
        battery: parse_macos_battery(&pmset, &ioreg),
        profiles: Profiles::default(),
    })
}

#[cfg(target_os = "macos")]
pub(super) fn system_set_profile(_: PowerProfile) -> Result<(), Error> {
    Err(Error::new(
        "change the power profile",
        "no supported macOS power-profile adapter is available",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn system_set_charge_threshold(_: &ChargeThreshold, _: bool) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change optimized charging",
        "no supported macOS charge-threshold adapter is available",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn command(
    program: &'static str,
    arguments: &[&str],
    operation: &'static str,
) -> Result<String, Error> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| Error::new(operation, error.to_string()))?;
    if !output.status.success() {
        return Err(Error::new(
            operation,
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn parse_macos_battery(pmset: &str, ioreg: &str) -> Option<Battery> {
    let on_battery = pmset
        .lines()
        .next()
        .is_some_and(|line| line.contains("Battery Power"));
    let line = pmset.lines().find(|line| line.contains('%'))?;
    if line.contains("present: false") {
        return None;
    }
    let parts = line.split(';').collect::<Vec<_>>();
    let percentage = parts
        .first()?
        .split_whitespace()
        .find_map(|field| field.strip_suffix('%'))?
        .parse::<u8>()
        .ok()?
        .min(100);
    let state = match parts.get(1).map(|state| state.trim()) {
        Some("charging") => BatteryState::Charging,
        Some("discharging") => BatteryState::Discharging,
        Some("charged") => BatteryState::FullyCharged,
        Some("finishing charge") => BatteryState::PendingCharge,
        _ => BatteryState::Unknown,
    };
    let seconds_remaining = parts.get(2).and_then(|part| parse_macos_time(part));
    let charge_cycles = ioreg_field(ioreg, "\"CycleCount\"").and_then(|value| value.parse().ok());
    let raw_max =
        ioreg_field(ioreg, "\"AppleRawMaxCapacity\"").and_then(|value| value.parse::<f64>().ok());
    let design =
        ioreg_field(ioreg, "\"DesignCapacity\"").and_then(|value| value.parse::<f64>().ok());
    let capacity = raw_max
        .zip(design)
        .filter(|(_, design)| *design > 0.0)
        .map(|(full, design)| percent(full / design * 100.0));
    Some(Battery {
        percentage,
        state,
        on_battery,
        seconds_remaining,
        capacity,
        charge_cycles,
        energy_rate_watts: None,
        model: None,
        charge_threshold: ChargeThreshold::default(),
        history: BatteryHistory::default(),
    })
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn parse_macos_time(value: &str) -> Option<u64> {
    let time = value.replace("remaining", "");
    let time = time.split_whitespace().next()?;
    let (hours, minutes) = time.split_once(':')?;
    Some(hours.parse::<u64>().ok()? * 3600 + minutes.parse::<u64>().ok()? * 60)
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn ioreg_field(contents: &str, key: &str) -> Option<String> {
    let needle = format!("{key} = ");
    contents
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&needle))
        .map(str::trim)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}
