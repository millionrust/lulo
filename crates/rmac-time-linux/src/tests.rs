use crate::watch::{owner_change_reappeared, realtime_parts};

#[test]
fn idle_exit_is_ignored_but_reappearance_refreshes() {
    assert!(!owner_change_reappeared("org.freedesktop.timedate1", ""));
    assert!(owner_change_reappeared(
        "org.freedesktop.timedate1",
        ":1.42"
    ));
    assert!(!owner_change_reappeared("org.example.Other", ":1.42"));
}

#[test]
fn realtime_deadlines_preserve_nanosecond_precision() {
    let deadline = std::time::UNIX_EPOCH
        + std::time::Duration::from_secs(1_234)
        + std::time::Duration::from_nanos(567_890_123);
    assert_eq!(realtime_parts(deadline).unwrap(), (1_234, 567_890_123));
    assert!(realtime_parts(std::time::UNIX_EPOCH - std::time::Duration::from_nanos(1)).is_err());
}
