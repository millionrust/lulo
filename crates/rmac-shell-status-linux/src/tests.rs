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
    use crate::model::only_unshown_properties;

    let wireless = "org.freedesktop.NetworkManager.Device.Wireless";
    let battery = "org.freedesktop.UPower.Device";
    assert!(only_unshown_properties(wireless, &["Bitrate"], &[]));
    assert!(only_unshown_properties(battery, &["UpdateTime"], &[]));

    // Anything the bar shows, or anything unknown, still refreshes.
    assert!(!only_unshown_properties(
        battery,
        &["UpdateTime", "Percentage"],
        &[]
    ));
    assert!(!only_unshown_properties(
        wireless,
        &["ActiveAccessPoint"],
        &[]
    ));
    assert!(!only_unshown_properties(
        wireless,
        &["Bitrate"],
        &["Bitrate"]
    ));
    assert!(!only_unshown_properties(wireless, &[], &[]));
    assert!(!only_unshown_properties(
        "org.freedesktop.NetworkManager.AccessPoint",
        &["Strength"],
        &[]
    ));
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
