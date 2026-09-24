//! Shared [`BluetoothService`] contract, run against every implementation.
//!
//! `assert_bluetooth_service_is_observable` is read-only and safe to run
//! against [`SystemBluetoothService`] on real hardware (see `tests.rs`'s
//! `#[ignore]` live test). `assert_bluetooth_service_contract` additionally
//! powers the adapter and connects/pairs devices, so it only ever runs
//! against [`crate::fake::FakeBluetoothService`] — running it against a
//! shared machine's real adapter would drop other work's Bluetooth
//! connections.

use super::*;

/// Structural invariants that hold for any snapshot, live or fake.
pub fn assert_bluetooth_service_is_observable(service: &impl BluetoothService) {
    let snapshot = service.snapshot().expect("snapshot succeeds");
    if !snapshot.available {
        return;
    }
    if !snapshot.powered {
        assert!(
            snapshot.devices.iter().all(|device| !device.connected),
            "no device stays connected while the adapter is off"
        );
    }
    for device in &snapshot.devices {
        assert!(
            !device.connected || device.paired,
            "device {} is connected but not paired",
            device.id
        );
    }
}

/// The full mutation contract: powering off disconnects every device,
/// connecting an unpaired device is rejected, and pairing a canceled
/// session fails without leaving the device paired. Only run this against
/// disposable state (a fake, or a throwaway fixture) — never against a
/// shared machine's live Bluetooth adapter.
pub fn assert_bluetooth_service_contract(service: &impl BluetoothService) {
    assert_bluetooth_service_is_observable(service);

    service.set_powered(true).expect("powering on succeeds");
    assert!(service.snapshot().unwrap().powered, "power-on is visible");

    let before = service.snapshot().unwrap();
    let Some(target) = before
        .devices
        .iter()
        .find(|device| !device.paired)
        .map(|device| device.id.clone())
    else {
        // No fixture provided an unpaired device; the read-only invariants
        // above are still exercised.
        return;
    };

    assert!(
        service.set_connected(&target, true).is_err(),
        "connecting an unpaired device is rejected"
    );

    let (session, _events) = PairingSession::new();
    session.cancel();
    assert!(
        service.pair(&target, &session).is_err(),
        "pairing a canceled session fails"
    );
    let after_cancel = service.snapshot().unwrap();
    let device = after_cancel
        .devices
        .iter()
        .find(|device| device.id == target)
        .expect("the device is still reported");
    assert!(
        !device.paired,
        "a canceled pairing leaves the device unpaired"
    );

    service.set_powered(false).expect("powering off succeeds");
    let after_power_off = service.snapshot().unwrap();
    assert!(
        after_power_off
            .devices
            .iter()
            .all(|device| !device.connected),
        "powering off disconnects every device"
    );
}
