use crate::system::{grouped_number, hour_cycle_from_time_format};
use crate::watch::owner_change_reappeared;

#[test]
fn idle_exit_is_ignored_but_reappearance_refreshes() {
    assert!(!owner_change_reappeared("org.freedesktop.locale1", ""));
    assert!(owner_change_reappeared("org.freedesktop.locale1", ":1.42"));
    assert!(!owner_change_reappeared("org.example.Other", ":1.42"));
}

#[test]
fn number_example_follows_locale_separators() {
    let number = grouped_number(" ", ",");
    assert_eq!(number, "1 234,56");
}

#[test]
fn hour_cycle_comes_from_the_locale_time_format() {
    assert_eq!(
        hour_cycle_from_time_format("%r"),
        Some(rmac_locale::HourCycle::TwelveHour)
    );
    assert_eq!(
        hour_cycle_from_time_format("%OH:%M:%S"),
        Some(rmac_locale::HourCycle::TwentyFourHour)
    );
    assert_eq!(hour_cycle_from_time_format("%% %Z"), None);
}
