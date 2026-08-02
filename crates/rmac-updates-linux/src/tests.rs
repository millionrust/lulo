use rmac_updates::WatchEvent;

use crate::api::{
    is_progress_change_member, FILTER_NONE, FLAG_ONLY_TRUSTED, FLAG_SIMULATE,
    PACKAGEKIT_DESTINATION, ROLE_UPDATE_PACKAGES,
};
use crate::watch::packagekit_owner_event;

#[test]
fn modern_packagekit_bitfields_match_the_upstream_enum_contract() {
    assert_eq!(FILTER_NONE, 2);
    assert_eq!(FLAG_ONLY_TRUSTED, 2);
    assert_eq!(FLAG_SIMULATE, 4);
    assert_eq!(ROLE_UPDATE_PACKAGES, 1 << 22);
}

#[test]
fn modern_and_legacy_progress_notifications_are_recognized() {
    assert!(is_progress_change_member("PropertiesChanged"));
    assert!(is_progress_change_member("Changed"));
    assert!(!is_progress_change_member("Package"));
}

#[test]
fn idle_daemon_exit_is_not_reported_as_an_outage() {
    assert_eq!(packagekit_owner_event(PACKAGEKIT_DESTINATION, ""), None);
    assert_eq!(
        packagekit_owner_event(PACKAGEKIT_DESTINATION, ":1.42"),
        Some(WatchEvent::Changed)
    );
    assert_eq!(packagekit_owner_event("org.example.Other", ":1.42"), None);
}
