//! Focused power service contracts.

use super::*;

#[test]
fn upower_states_and_percentages_are_normalized() {
    assert_eq!(battery_state_from_upower(1), BatteryState::Charging);
    assert_eq!(battery_state_from_upower(4), BatteryState::FullyCharged);
    assert_eq!(battery_state_from_upower(99), BatteryState::Unknown);
    assert_eq!(percent(101.0), 100);
    assert_eq!(percent(-1.0), 0);
    assert_eq!(threshold_percent(Some(80)), Some(80));
    assert_eq!(threshold_percent(Some(u32::MAX)), None);
}

#[test]
fn charge_history_is_valid_ordered_unique_and_bounded() {
    let mut raw = (1..=110)
        .rev()
        .map(|timestamp| (timestamp, f64::from(timestamp % 101), 2))
        .collect::<Vec<_>>();
    raw.push((110, 42.0, 1));
    raw.push((0, 50.0, 2));
    raw.push((111, f64::NAN, 2));
    raw.push((112, 101.0, 2));

    let points = normalize_history(raw);
    assert_eq!(points.len(), HISTORY_POINT_LIMIT);
    assert_eq!(points.first().map(|point| point.timestamp), Some(15));
    assert_eq!(points.last().map(|point| point.timestamp), Some(110));
    assert!(points
        .windows(2)
        .all(|points| points[0].timestamp < points[1].timestamp));
}

#[test]
fn threshold_debug_output_redacts_private_identity() {
    let threshold = ChargeThreshold {
        availability: ChargeThresholdAvailability::Available,
        enabled: true,
        start_percent: Some(40),
        end_percent: Some(80),
        firmware_managed: false,
        identity: Some(ChargeThresholdIdentity {
            service_owner: ":1.42".to_owned(),
            object_path: "/org/freedesktop/UPower/devices/battery_BAT0".to_owned(),
            native_path: "/private/device/path".to_owned(),
            serial: "private-serial".to_owned(),
        }),
    };
    let output = format!("{threshold:?}");
    assert!(threshold.can_change());
    assert!(output.contains("has_identity: true"));
    assert!(!output.contains("private"));
    assert!(!output.contains("BAT0"));
}

#[test]
fn profile_order_is_stable_and_duplicates_are_removed() {
    assert_eq!(parse_profile("balanced"), Some(PowerProfile::Balanced));
    assert_eq!(parse_profile("unsupported"), None);
    let mut profiles = vec![
        PowerProfile::Performance,
        PowerProfile::Balanced,
        PowerProfile::PowerSaver,
        PowerProfile::Balanced,
    ];
    normalize_profiles(&mut profiles);
    assert_eq!(
        profiles,
        vec![
            PowerProfile::PowerSaver,
            PowerProfile::Balanced,
            PowerProfile::Performance
        ]
    );
}

#[test]
fn bus_driver_signals_are_not_power_changes() {
    assert!(sent_by_service(Some(":1.42")));
    assert!(!sent_by_service(Some("org.freedesktop.DBus")));
    assert!(!sent_by_service(None));
}

#[test]
fn owner_events_distinguish_upower_outages_from_optional_profile_changes() {
    assert_eq!(
        power_owner_event("org.freedesktop.UPower", ""),
        Some(PowerOwnerEvent::Upower(false))
    );
    assert_eq!(
        power_owner_event("org.freedesktop.UPower", ":1.42"),
        Some(PowerOwnerEvent::Upower(true))
    );
    assert_eq!(
        power_owner_event("org.freedesktop.UPower.PowerProfiles", ""),
        Some(PowerOwnerEvent::Profiles)
    );
    assert_eq!(
        power_owner_event("net.hadess.PowerProfiles", ":1.43"),
        Some(PowerOwnerEvent::Profiles)
    );
    assert_eq!(power_owner_event("org.example.Other", ":1.44"), None);
}

#[test]
fn macos_fixture_preserves_charge_health_and_time() {
    let battery = parse_macos_battery(
            "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t63%; discharging; 2:15 remaining present: true",
            "\"CycleCount\" = 242\n\"AppleRawMaxCapacity\" = 4500\n\"DesignCapacity\" = 5000",
        )
        .unwrap();
    assert_eq!(battery.percentage, 63);
    assert_eq!(battery.state, BatteryState::Discharging);
    assert_eq!(battery.seconds_remaining, Some(8100));
    assert_eq!(battery.capacity, Some(90));
    assert_eq!(battery.charge_cycles, Some(242));
    assert!(battery.on_battery);
}

#[test]
fn errors_keep_operation_context() {
    let error = Error::new("read battery state", "service unavailable");
    assert_eq!(
        error.to_string(),
        "could not read battery state: service unavailable"
    );
}
