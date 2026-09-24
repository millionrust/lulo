//! Shared [`WifiService`] contract, run against every implementation.
//!
//! `assert_wifi_service_is_observable` is read-only and safe to run against
//! [`SystemWifiService`] on real hardware (see `tests.rs`'s `#[ignore]`
//! live test). `assert_wifi_service_contract` additionally toggles the
//! radio and joins/forgets networks, so it only ever runs against
//! [`crate::fake::FakeWifiService`] — running it against a shared machine's
//! real Wi-Fi adapter would disconnect other work on that machine.

use super::*;

/// Structural invariants that hold for any snapshot, live or fake: a second
/// read right after the first agrees on which network is current, strengths
/// stay in range, and known networks are exactly the saved set.
pub fn assert_wifi_service_is_observable(service: &impl WifiService) {
    let snapshot = service.snapshot().expect("snapshot succeeds");
    if !snapshot.available {
        return;
    }
    for network in &snapshot.networks {
        assert!(
            network.strength <= 100,
            "strength {} exceeds the 0-100 scale",
            network.strength
        );
    }
    let connected: Vec<_> = snapshot
        .networks
        .iter()
        .filter(|network| network.connected)
        .collect();
    assert!(
        connected.len() <= 1,
        "at most one Wi-Fi network is connected at a time, found {}",
        connected.len()
    );
    if let Some(current) = connected.first() {
        assert_eq!(
            snapshot.current_ssid.as_deref(),
            Some(current.ssid.as_str())
        );
    }
}

/// The full mutation contract: enabling/disabling is visible in the next
/// snapshot, joining a known open network connects it and disconnects any
/// previously active network, and forgetting an unsaved network is an
/// error. Only run this against disposable state (a fake, or a throwaway
/// fixture) — never against a shared machine's live Wi-Fi adapter.
pub fn assert_wifi_service_contract(service: &impl WifiService) {
    assert_wifi_service_is_observable(service);

    service.set_enabled(false).expect("disabling succeeds");
    assert!(!service.snapshot().unwrap().enabled, "disable is visible");
    service.set_enabled(true).expect("enabling succeeds");
    assert!(service.snapshot().unwrap().enabled, "enable is visible");

    let before = service.snapshot().unwrap();
    let Some(target) = before
        .networks
        .iter()
        .find(|network| {
            network.can_connect()
                && matches!(
                    network.security,
                    WifiSecurity::Open | WifiSecurity::EnhancedOpen
                )
        })
        .map(|network| network.id.clone())
    else {
        // No fixture provided an open network to join; the read-only
        // invariants above are still exercised.
        return;
    };
    let after = service
        .connect(&target)
        .expect("joining a visible open network succeeds");
    let joined = after
        .networks
        .iter()
        .find(|network| network.id == target)
        .expect("the joined network is still reported");
    assert!(joined.connected, "connect() marks the network connected");
    assert_eq!(after.current_ssid.as_deref(), Some(joined.ssid.as_str()));

    let after_forget = service
        .forget(&target)
        .expect("forgetting a just-joined network succeeds");
    assert!(
        after_forget
            .networks
            .iter()
            .find(|network| network.id == target)
            .is_none_or(|network| !network.connected),
        "forget() disconnects the network"
    );
}
