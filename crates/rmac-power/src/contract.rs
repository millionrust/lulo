//! Shared [`PowerService`] contract, run against every implementation.
//!
//! `assert_power_service_is_observable` is read-only and safe to run
//! against [`SystemPowerService`] on real hardware (see `tests.rs`'s
//! `#[ignore]` live test). `assert_power_service_contract` additionally
//! switches the active power profile, so — matching the network and
//! Bluetooth contracts — it only ever runs against
//! [`crate::fake::FakePowerService`].

use super::*;
use crate::fake::PowerService;

/// Structural invariants that hold for any snapshot, live or fake.
pub fn assert_power_service_is_observable(service: &impl PowerService) {
    let snapshot = service.snapshot().expect("snapshot succeeds");
    if let Some(battery) = &snapshot.battery {
        assert!(
            battery.percentage <= 100,
            "battery percentage {} exceeds the 0-100 scale",
            battery.percentage
        );
    }
    if let Some(active) = snapshot.profiles.active {
        assert!(
            snapshot.profiles.supported.contains(&active),
            "the active profile is always one of the supported profiles"
        );
    }
}

/// The mutation contract: switching to a supported profile is visible in
/// the next snapshot. Only run this against disposable state (a fake, or
/// a throwaway fixture) — never against a shared machine's live power
/// service.
pub fn assert_power_service_contract(service: &impl PowerService) {
    assert_power_service_is_observable(service);

    let before = service.snapshot().unwrap();
    let Some(target) = before
        .profiles
        .supported
        .iter()
        .copied()
        .find(|profile| Some(*profile) != before.profiles.active)
    else {
        // No fixture provided an alternate supported profile; the
        // read-only invariants above are still exercised.
        return;
    };
    service
        .set_profile(target)
        .expect("switching to a supported profile succeeds");
    assert_eq!(
        service.snapshot().unwrap().profiles.active,
        Some(target),
        "set_profile() is visible in the next snapshot"
    );
}
