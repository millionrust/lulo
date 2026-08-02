use std::collections::BTreeSet;

use rmac_focus::{Config, Mode, ModeId, Schedule, ScheduleId, Weekday};
use rmac_notifications::AppId;

use super::*;

fn config_with_schedule() -> Config {
    rmac_focus::Config::new(
        vec![Mode::new(
            ModeId::parse("work").unwrap(),
            "Work",
            BTreeSet::new(),
            false,
        )
        .unwrap()],
        vec![Schedule {
            id: ScheduleId::parse("weekday").unwrap(),
            mode: ModeId::parse("work").unwrap(),
            days: BTreeSet::from([Weekday::Monday, Weekday::Tuesday]),
            start_minute: 9 * 60,
            end_minute: 17 * 60,
            priority: 1,
            enabled: true,
        }],
    )
    .unwrap()
}

#[test]
fn mode_edits_preserve_unrelated_configuration() {
    let config = config_with_schedule();
    let work = ModeId::parse("work").unwrap();
    let app = AppId::parse("org.example.Editor").unwrap();
    let allowed = set_allowed_app(&config, &work, app.clone(), true).unwrap();
    assert!(allowed.mode(&work).unwrap().allowed_apps().contains(&app));
    assert_eq!(
        allowed.schedules().collect::<Vec<_>>(),
        config.schedules().collect::<Vec<_>>()
    );

    let urgent = set_mode_urgent(&allowed, &work, true).unwrap();
    assert!(urgent.mode(&work).unwrap().allow_urgent());
    assert!(urgent.mode(&work).unwrap().allowed_apps().contains(&app));

    let removed = set_allowed_app(&urgent, &work, app.clone(), false).unwrap();
    assert!(!removed.mode(&work).unwrap().allowed_apps().contains(&app));
}

#[test]
fn schedule_edits_validate_the_whole_graph() {
    let config = config_with_schedule();
    let id = ScheduleId::parse("weekday").unwrap();
    let disabled = set_schedule_enabled(&config, &id, false).unwrap();
    assert!(!disabled.schedules().next().unwrap().enabled);

    let mut replacement = disabled.schedules().next().unwrap().clone();
    replacement.start_minute = 8 * 60;
    let updated = upsert_schedule(&disabled, replacement).unwrap();
    assert_eq!(updated.schedules().next().unwrap().start_minute, 8 * 60);

    let without_monday = set_schedule_day(&updated, &id, Weekday::Monday, false).unwrap();
    assert!(!without_monday
        .schedules()
        .next()
        .unwrap()
        .days
        .contains(&Weekday::Monday));

    let moved_start = set_schedule_start(&without_monday, &id, 10 * 60).unwrap();
    let moved_end = set_schedule_end(&moved_start, &id, 18 * 60).unwrap();
    let schedule = moved_end.schedules().next().unwrap();
    assert_eq!(schedule.start_minute, 10 * 60);
    assert_eq!(schedule.end_minute, 18 * 60);

    let removed = remove_schedule(&moved_end, &id).unwrap();
    assert_eq!(removed.schedules().count(), 0);
    assert_eq!(remove_schedule(&removed, &id), Err(Error::UnknownSchedule));
}

#[test]
fn schedule_creation_is_immediately_editable_and_collision_safe() {
    let config = Config::new(
        config_with_schedule().modes().cloned().collect(),
        Vec::new(),
    )
    .unwrap();
    let work = ModeId::parse("work").unwrap();
    let (first, first_id) = create_schedule(&config, &work).unwrap();
    let (second, second_id) = create_schedule(&first, &work).unwrap();
    assert_ne!(first_id, second_id);
    assert_eq!(second.schedules().count(), 2);
    let created = first.schedules().next().unwrap();
    assert_eq!(created.start_minute, 9 * 60);
    assert_eq!(created.end_minute, 17 * 60);
    assert!(created.days.contains(&Weekday::Monday));
    assert!(!created.days.contains(&Weekday::Sunday));
}

#[test]
fn a_schedule_cannot_have_zero_days_or_equal_times() {
    let config = config_with_schedule();
    let id = ScheduleId::parse("weekday").unwrap();
    let tuesday_only = set_schedule_day(&config, &id, Weekday::Monday, false).unwrap();
    assert_eq!(
        set_schedule_day(&tuesday_only, &id, Weekday::Tuesday, false),
        Err(Error::Invalid)
    );
    assert_eq!(set_schedule_end(&config, &id, 9 * 60), Err(Error::Invalid));
    assert_eq!(set_schedule_start(&config, &id, 1_440), Err(Error::Invalid));
}

#[test]
fn unknown_targets_fail_without_mutating_configuration() {
    let config = config_with_schedule();
    assert_eq!(
        set_mode_urgent(&config, &ModeId::parse("missing").unwrap(), true),
        Err(Error::UnknownMode)
    );
    assert_eq!(
        set_schedule_enabled(&config, &ScheduleId::parse("missing").unwrap(), false,),
        Err(Error::UnknownSchedule)
    );
}
