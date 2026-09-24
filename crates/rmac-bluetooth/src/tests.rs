use super::*;
use crate::macos::{parse_macos_snapshot, sort_devices};
use crate::watch::bluez_owner_availability;

/// Runs the read-only half of the [`BluetoothService`] contract against
/// the real BlueZ backend. `#[ignore]`d because it needs a D-Bus session
/// and a Bluetooth adapter; run it deliberately on the reference laptop
/// with `cargo test -p rmac-bluetooth -- --ignored`. It never mutates
/// state (see [`contract::assert_bluetooth_service_contract`]'s doc
/// comment for why the mutating half only runs against the fake).
#[test]
#[ignore = "needs a live BlueZ session; run on the reference laptop"]
fn system_bluetooth_service_snapshot_is_well_formed() {
    contract::assert_bluetooth_service_is_observable(&SystemBluetoothService);
}

#[test]
fn macos_fixture_parses_and_sorts_known_devices() {
    let snapshot = parse_macos_snapshot(
            "Bluetooth Controller:\n  State: On\n  Connected:\n    Keyboard:\n      Address: AA-BB\n      Minor Type: Keyboard\n  Not Connected:\n    Headphones:\n      Address: CC-DD\n      Minor Type: Headphones\n",
        );

    assert!(snapshot.available);
    assert!(snapshot.powered);
    assert_eq!(snapshot.devices.len(), 2);
    assert_eq!(snapshot.devices[0].name, "Keyboard");
    assert!(snapshot.devices[0].connected);
    assert_eq!(snapshot.devices[1].kind, "Headphones");
}

#[test]
fn errors_preserve_operation_context() {
    let error = Error::new("read Bluetooth state", "service unavailable");
    assert_eq!(
        error.to_string(),
        "could not read Bluetooth state: service unavailable"
    );
}

#[test]
fn bluez_owner_changes_only_match_the_bluez_service() {
    assert_eq!(bluez_owner_availability("org.bluez", ":1.42"), Some(true));
    assert_eq!(bluez_owner_availability("org.bluez", ""), Some(false));
    assert_eq!(bluez_owner_availability("org.example.Other", ""), None);
}

#[test]
fn pairing_errors_keep_user_outcomes_typed() {
    let (session, _events) = PairingSession::new();
    session.cancel();
    let error = Error::pairing("pair Bluetooth device", "D-Bus failure", &session);
    assert!(error.is_canceled());
    assert!(!error.is_rejected());
    assert!(!error.is_timed_out());
}

#[test]
fn device_order_prefers_connected_then_paired() {
    let mut devices = vec![
        Device {
            id: "nearby".into(),
            name: "Nearby".into(),
            address: String::new(),
            kind: String::new(),
            paired: false,
            trusted: false,
            connected: false,
        },
        Device {
            id: "paired".into(),
            name: "Paired".into(),
            address: String::new(),
            kind: String::new(),
            paired: true,
            trusted: true,
            connected: false,
        },
        Device {
            id: "connected".into(),
            name: "Connected".into(),
            address: String::new(),
            kind: String::new(),
            paired: true,
            trusted: true,
            connected: true,
        },
    ];

    sort_devices(&mut devices);

    assert_eq!(devices[0].id, "connected");
    assert_eq!(devices[1].id, "paired");
    assert_eq!(devices[2].id, "nearby");
}
