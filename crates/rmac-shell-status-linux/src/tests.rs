use super::*;

#[test]
fn source_sets_merge_without_losing_independent_services() {
    let mut sources = Sources::audio();
    sources.merge(Sources::system_bus());
    assert_eq!(sources, Sources::all());
    assert!(!sources.is_empty());
    assert!(Sources::empty().is_empty());
}

#[test]
fn bitrate_and_poll_time_changes_do_not_refresh() {
    use crate::model::{property_change, PropertyChange};

    let wireless = "org.freedesktop.NetworkManager.Device.Wireless";
    let battery = "org.freedesktop.UPower.Device";
    let access_point = "org.freedesktop.NetworkManager.AccessPoint";
    assert_eq!(
        property_change(wireless, &["Bitrate"], &[]),
        PropertyChange::Unshown
    );
    assert_eq!(
        property_change(battery, &["UpdateTime"], &[]),
        PropertyChange::Unshown
    );
    assert_eq!(
        property_change(access_point, &["Strength"], &[]),
        PropertyChange::SignalStrength
    );

    // Anything the bar shows, or anything unknown, still refreshes.
    for (interface, changed, invalidated) in [
        (battery, &["UpdateTime", "Percentage"][..], &[][..]),
        (wireless, &["ActiveAccessPoint"][..], &[][..]),
        (wireless, &["Bitrate"][..], &["Bitrate"][..]),
        (wireless, &[][..], &[][..]),
        (access_point, &["Strength", "Ssid"][..], &[][..]),
    ] {
        assert_eq!(
            property_change(interface, changed, invalidated),
            PropertyChange::Shown,
            "{interface} {changed:?} {invalidated:?}"
        );
    }
}

#[test]
fn signal_strength_rereads_the_network_at_most_every_30_seconds() {
    use crate::model::signal_strength_refresh_due;
    use std::time::Duration;

    assert!(signal_strength_refresh_due(None));
    assert!(!signal_strength_refresh_due(Some(Duration::from_secs(6))));
    assert!(signal_strength_refresh_due(Some(Duration::from_secs(30))));
}

#[test]
fn source_sets_scope_transport_failures() {
    assert!(!Sources::system_bus().audio);
    assert_eq!(
        Sources::audio(),
        Sources {
            audio: true,
            ..Sources::empty()
        }
    );
}
