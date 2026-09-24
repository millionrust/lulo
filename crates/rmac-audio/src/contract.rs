//! Shared [`AudioService`] contract, run against every implementation.
//!
//! `assert_audio_service_is_observable` is read-only and safe to run
//! against [`SystemAudioService`] on real hardware (see `tests.rs`'s
//! `#[ignore]` live test). `assert_audio_service_contract` additionally
//! changes the volume, mute state and default device, so — matching the
//! network, Bluetooth and power contracts — it only ever runs against
//! [`crate::fake::FakeAudioService`].

use super::*;
use crate::fake::AudioService;

/// Structural invariants that hold for any snapshot, live or fake.
pub fn assert_audio_service_is_observable(service: &impl AudioService) {
    let snapshot = service.snapshot().expect("snapshot succeeds");
    if !snapshot.available {
        return;
    }
    assert!(snapshot.output.volume <= 100, "output volume exceeds 100");
    assert!(snapshot.input.volume <= 100, "input volume exceeds 100");
    assert!(
        snapshot
            .outputs
            .iter()
            .filter(|device| device.is_default)
            .count()
            <= 1,
        "at most one output is default at a time"
    );
    assert!(
        snapshot
            .inputs
            .iter()
            .filter(|device| device.is_default)
            .count()
            <= 1,
        "at most one input is default at a time"
    );
}

/// The mutation contract: setting the volume clamps to 0-100 and is
/// visible in the next snapshot, muting is visible in the next snapshot,
/// and switching the default output is visible in the next snapshot. Only
/// run this against disposable state — never a shared machine's live
/// audio graph.
pub fn assert_audio_service_contract(service: &impl AudioService) {
    assert_audio_service_is_observable(service);

    service
        .set_volume(DeviceKind::Output, 255)
        .expect("set_volume succeeds");
    assert_eq!(
        service.snapshot().unwrap().output.volume,
        100,
        "volume is clamped to 100"
    );

    service
        .set_muted(DeviceKind::Output, true)
        .expect("set_muted succeeds");
    assert!(service.snapshot().unwrap().output.muted, "mute is visible");
    service
        .set_muted(DeviceKind::Output, false)
        .expect("unmuting succeeds");
    assert!(
        !service.snapshot().unwrap().output.muted,
        "unmute is visible"
    );

    let before = service.snapshot().unwrap();
    let Some(target) = before
        .outputs
        .iter()
        .find(|device| !device.is_default)
        .cloned()
    else {
        // No fixture provided a second output to switch to; the
        // read-only invariants above are still exercised.
        return;
    };
    let after = service
        .set_default_device(DeviceKind::Output, &target)
        .expect("switching the default output succeeds");
    let switched = after
        .outputs
        .iter()
        .find(|device| device.id == target.id)
        .expect("the switched-to device is still reported");
    assert!(switched.is_default, "set_default_device() is visible");
}
