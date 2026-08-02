use super::*;
use rmac_notifications::{AppId, BannerPolicy, DeliveryPolicy, HistoryPolicy};

fn mode(id: &str, apps: &[&str], urgent: bool) -> Mode {
    Mode::new(
        ModeId::parse(id).unwrap(),
        format!("Private {id}"),
        apps.iter().map(|app| AppId::parse(*app).unwrap()).collect(),
        urgent,
    )
    .unwrap()
}

fn schedule(
    id: &str,
    mode: &str,
    days: &[Weekday],
    start: u16,
    end: u16,
    priority: u8,
) -> Schedule {
    Schedule {
        id: ScheduleId::parse(id).unwrap(),
        mode: ModeId::parse(mode).unwrap(),
        days: days.iter().copied().collect(),
        start_minute: start,
        end_minute: end,
        priority,
        enabled: true,
    }
}

fn clock(unix_ms: u64, monotonic_ms: u64, weekday: Weekday, minute: u16) -> ClockSample {
    ClockSample {
        unix_ms,
        monotonic_ms,
        weekday,
        minute_of_day: minute,
        next_minute_unix_ms: unix_ms + 60_000,
    }
}

fn base() -> DeliveryPolicy {
    DeliveryPolicy {
        enabled: true,
        banner: BannerPolicy::Allow,
        sounds: true,
        history: HistoryPolicy::Allow,
        allow_urgent_through_focus: true,
        focus_active: false,
    }
}

#[test]
fn overnight_schedule_uses_previous_start_day_after_midnight() {
    let config = Config::new(
        vec![mode("sleep", &[], false)],
        vec![schedule(
            "weeknight",
            "sleep",
            &[Weekday::Monday],
            22 * 60,
            7 * 60,
            1,
        )],
    )
    .unwrap();
    let mut engine = Engine::new(config, None).unwrap();
    assert!(
        engine
            .evaluate(clock(1_000, 1_000, Weekday::Monday, 23 * 60))
            .unwrap()
            .status
            .active
    );
    assert!(
        engine
            .evaluate(clock(2_000, 2_000, Weekday::Tuesday, 6 * 60))
            .unwrap()
            .status
            .active
    );
    assert!(
        !engine
            .evaluate(clock(3_000, 3_000, Weekday::Tuesday, 8 * 60))
            .unwrap()
            .status
            .active
    );
}

#[test]
fn manual_duration_overrides_schedule_then_expires_after_restart() {
    let config = Config::new(
        vec![mode("work", &[], false), mode("sleep", &[], false)],
        vec![schedule(
            "always-now",
            "sleep",
            &[Weekday::Monday],
            0,
            1_439,
            1,
        )],
    )
    .unwrap();
    let manual = ManualActivation {
        mode: ModeId::parse("work").unwrap(),
        until_unix_ms: Some(10_000),
    };
    let mut restarted = Engine::new(config, Some(manual)).unwrap();
    let active = restarted
        .evaluate(clock(9_000, 100, Weekday::Monday, 100))
        .unwrap();
    assert_eq!(active.status.mode.unwrap().as_str(), "work");
    assert_eq!(active.wake, Wake::AtUnixMs(10_000));
    let expired = restarted
        .evaluate(clock(10_000, 1_100, Weekday::Monday, 100))
        .unwrap();
    assert_eq!(expired.status.mode.unwrap().as_str(), "sleep");
    assert!(restarted.manual_activation().is_none());
}

#[test]
fn higher_priority_overlap_wins_deterministically() {
    let config = Config::new(
        vec![mode("low", &[], false), mode("high", &[], false)],
        vec![
            schedule("low-rule", "low", &[Weekday::Friday], 60, 180, 1),
            schedule("high-rule", "high", &[Weekday::Friday], 60, 180, 9),
        ],
    )
    .unwrap();
    let mut engine = Engine::new(config, None).unwrap();
    assert_eq!(
        engine
            .evaluate(clock(1_000, 1_000, Weekday::Friday, 90))
            .unwrap()
            .status
            .mode
            .unwrap()
            .as_str(),
        "high"
    );
}

#[test]
fn allowed_apps_bypass_focus_while_urgent_policy_is_composed() {
    let config = Config::new(
        vec![mode("work", &["org.example.Allowed"], false)],
        Vec::new(),
    )
    .unwrap();
    let mut engine = Engine::new(config, None).unwrap();
    engine
        .activate_indefinitely(ModeId::parse("work").unwrap())
        .unwrap();
    engine
        .evaluate(clock(1_000, 1_000, Weekday::Monday, 100))
        .unwrap();
    let allowed = engine.enforce(&AppId::parse("org.example.Allowed").unwrap(), base());
    assert!(!allowed.focus_active);
    let blocked = engine.enforce(&AppId::parse("org.example.Blocked").unwrap(), base());
    assert!(blocked.focus_active);
    assert!(!blocked.allow_urgent_through_focus);
}

#[test]
fn wall_clock_jump_forces_recomputation_without_stale_mode() {
    let config = Config::new(
        vec![mode("work", &[], false)],
        vec![schedule(
            "morning",
            "work",
            &[Weekday::Monday],
            8 * 60,
            9 * 60,
            1,
        )],
    )
    .unwrap();
    let mut engine = Engine::new(config, None).unwrap();
    assert!(
        engine
            .evaluate(clock(1_000_000, 1_000, Weekday::Monday, 8 * 60 + 30))
            .unwrap()
            .status
            .active
    );
    let jumped = engine
        .evaluate(clock(4_600_000, 2_000, Weekday::Monday, 9 * 60 + 30))
        .unwrap();
    assert!(jumped.wall_clock_jump);
    assert!(!jumped.status.active);
    assert!(jumped.changed);
}

#[test]
fn invalid_durations_schedules_and_duplicate_ids_fail_closed() {
    let duplicate = Config::new(
        vec![mode("same", &[], false), mode("same", &[], true)],
        Vec::new(),
    );
    assert_eq!(duplicate, Err(Error::Duplicate));
    let invalid = Config::new(
        vec![mode("work", &[], false)],
        vec![schedule("bad", "work", &[Weekday::Monday], 100, 100, 1)],
    );
    assert_eq!(invalid, Err(Error::InvalidSchedule));

    let config = Config::new(vec![mode("work", &[], false)], Vec::new()).unwrap();
    let mut engine = Engine::new(config, None).unwrap();
    assert_eq!(
        engine.activate_for(ModeId::parse("work").unwrap(), 0, 1_000),
        Err(Error::InvalidDuration)
    );
}

#[test]
fn debug_output_redacts_mode_names_ids_and_apps() {
    let config = Config::new(
        vec![mode("private-mode-8472", &["org.private.App8472"], false)],
        Vec::new(),
    )
    .unwrap();
    let debug = format!("{config:?}");
    assert!(!debug.contains("private-mode-8472"));
    assert!(!debug.contains("App8472"));
}
