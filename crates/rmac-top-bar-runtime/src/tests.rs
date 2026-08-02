use std::time::{Duration, UNIX_EPOCH};

use chrono::{DateTime, FixedOffset, TimeZone as _};

use super::*;
use crate::watch::realtime_deadline;

fn at(hour: u32, minute: u32, second: u32, millis: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(5 * 3600 + 30 * 60)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 19, hour, minute, second)
        .single()
        .unwrap()
        + chrono::Duration::milliseconds(i64::from(millis))
}

fn shell_update(settings: rmac_shell_settings::ClockSettings) -> rmac_shell_runtime::Update {
    let mut snapshot = rmac_shell_runtime::Snapshot::default();
    snapshot.status.clock = settings;
    rmac_shell_runtime::Update {
        snapshot,
        visible: true,
        quick_settings_visible: false,
    }
}

#[test]
fn first_projection_waits_for_every_authority() {
    let mut coordinator = Coordinator::default();
    let now = at(21, 7, 8, 250);
    assert!(coordinator
        .apply_shell(shell_update(Default::default()), now)
        .is_none());
    assert!(coordinator
        .apply_locale(Ok(rmac_locale::HourCycle::TwelveHour), now)
        .is_none());
    let update = coordinator.apply_time_signal(now).unwrap();
    assert!(update.redraw);
    assert!(update.projection.content.clock.visible.contains("9:07 PM"));
}

#[test]
fn locale_failure_uses_then_preserves_a_last_known_good_cycle() {
    let mut coordinator = Coordinator::default();
    let now = at(21, 7, 8, 0);
    coordinator.apply_shell(shell_update(Default::default()), now);
    coordinator.apply_time_signal(now);
    let fallback = coordinator.apply_locale(Err(()), now).unwrap();
    assert!(fallback.projection.content.clock.visible.contains("21:07"));

    let twelve = coordinator
        .apply_locale(Ok(rmac_locale::HourCycle::TwelveHour), now)
        .unwrap();
    assert!(twelve.redraw);
    assert!(twelve.projection.content.clock.visible.contains("9:07 PM"));
    let unavailable = coordinator.apply_locale(Err(()), now).unwrap();
    assert!(!unavailable.redraw);
    assert_eq!(unavailable.projection, twelve.projection);
}

#[test]
fn clock_deadlines_rearm_exactly_without_idle_frames() {
    let mut coordinator = Coordinator::default();
    let now = at(21, 7, 8, 250);
    coordinator.apply_shell(shell_update(Default::default()), now);
    coordinator.apply_locale(Ok(rmac_locale::HourCycle::TwentyFourHour), now);
    coordinator.apply_time_signal(now);

    let duplicate = coordinator.clock_deadline(now).unwrap();
    assert!(!duplicate.redraw);
    assert_eq!(duplicate.next_clock_update, Duration::from_millis(51_750));
    let next_minute = coordinator.clock_deadline(at(21, 8, 0, 0)).unwrap();
    assert!(next_minute.redraw);
    assert_eq!(next_minute.next_clock_update, Duration::from_secs(60));
}

#[test]
fn enabling_seconds_replaces_the_minute_deadline() {
    let mut coordinator = Coordinator::default();
    let now = at(21, 7, 8, 250);
    coordinator.apply_locale(Ok(rmac_locale::HourCycle::TwentyFourHour), now);
    coordinator.apply_time_signal(now);
    let initial = coordinator
        .apply_shell(shell_update(Default::default()), now)
        .unwrap();
    assert_eq!(initial.next_clock_update, Duration::from_millis(51_750));

    let settings = rmac_shell_settings::ClockSettings {
        show_seconds: true,
        ..Default::default()
    };
    let seconds = coordinator
        .apply_shell(shell_update(settings), now)
        .unwrap();
    assert!(seconds.redraw);
    assert_eq!(seconds.next_clock_update, Duration::from_millis(750));
}

#[test]
fn realtime_target_is_the_exact_visible_boundary() {
    let target = realtime_deadline(at(21, 7, 8, 250), Duration::from_millis(51_750)).unwrap();
    let target_millis = target.duration_since(UNIX_EPOCH).unwrap().as_millis();
    assert_eq!(target_millis % 60_000, 0);
}
