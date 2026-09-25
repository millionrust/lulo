use rmac_updates::WatchEvent;

use crate::api::{
    is_progress_change_member, FILTER_NONE, FLAG_ONLY_DOWNLOAD, FLAG_ONLY_TRUSTED, FLAG_SIMULATE,
    OFFLINE_ACTION_REBOOT, PACKAGEKIT_DESTINATION, ROLE_UPDATE_PACKAGES,
};
use crate::transaction::{details_download_size, offline_update_flags};
use crate::watch::packagekit_owner_event;

#[test]
fn modern_packagekit_bitfields_match_the_upstream_enum_contract() {
    assert_eq!(FILTER_NONE, 2);
    assert_eq!(FLAG_ONLY_TRUSTED, 2);
    assert_eq!(FLAG_SIMULATE, 4);
    assert_eq!(ROLE_UPDATE_PACKAGES, 1 << 22);
    // PK_TRANSACTION_FLAG_ENUM_ONLY_DOWNLOAD = 3.
    assert_eq!(FLAG_ONLY_DOWNLOAD, 8);
    assert_eq!(OFFLINE_ACTION_REBOOT, "reboot");
}

#[test]
fn offline_preparation_downloads_trusted_packages_only() {
    // pk_transaction_obtain_authorization() skips polkit for ONLY_DOWNLOAD,
    // and ONLY_TRUSTED is never dropped.
    assert_eq!(offline_update_flags(), 2 | 8);
}

#[test]
fn details_prefer_the_download_size_and_ignore_zero() {
    assert_eq!(details_download_size(Some(4_000), Some(9_000)), Some(4_000));
    assert_eq!(details_download_size(None, Some(9_000)), Some(9_000));
    assert_eq!(details_download_size(Some(0), Some(0)), None);
    assert_eq!(details_download_size(Some(0), Some(9_000)), Some(9_000));
    assert_eq!(details_download_size(None, None), None);
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
