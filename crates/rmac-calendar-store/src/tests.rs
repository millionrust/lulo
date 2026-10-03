use super::*;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn fixture(properties: &str) -> Calendar {
    Calendar::parse(&format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:test\r\nSUMMARY:Meeting\r\n{properties}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )).unwrap()
}

fn occurrences(properties: &str, from: &str, to: &str) -> Vec<Occurrence> {
    expand(
        &fixture(properties),
        utc(from),
        utc(to),
        chrono_tz::UTC,
        1000,
    )
    .unwrap()
}

#[test]
fn rfc5545_daily_count_ten() {
    // RFC 5545 section 3.8.5.3, "Daily for 10 occurrences".
    let result = occurrences("DTSTART;TZID=America/New_York:19970902T090000\r\nDTEND;TZID=America/New_York:19970902T100000\r\nRRULE:FREQ=DAILY;COUNT=10", "1997-09-01T00:00:00Z", "1997-09-13T00:00:00Z");
    assert_eq!(result.len(), 10);
    assert_eq!(result.first().unwrap().start, utc("1997-09-02T13:00:00Z"));
    assert_eq!(result.last().unwrap().start, utc("1997-09-11T13:00:00Z"));
}

#[test]
fn rfc5545_every_ten_days_five_times() {
    let result = occurrences("DTSTART;TZID=America/New_York:19970902T090000\r\nDTEND;TZID=America/New_York:19970902T100000\r\nRRULE:FREQ=DAILY;INTERVAL=10;COUNT=5", "1997-09-01T00:00:00Z", "1997-10-14T00:00:00Z");
    let dates: Vec<_> = result
        .iter()
        .map(|event| {
            event
                .start
                .with_timezone(&chrono_tz::America::New_York)
                .date_naive()
                .to_string()
        })
        .collect();
    assert_eq!(
        dates,
        [
            "1997-09-02",
            "1997-09-12",
            "1997-09-22",
            "1997-10-02",
            "1997-10-12"
        ]
    );
}

#[test]
fn rfc5545_daily_until_december_24() {
    let result = occurrences("DTSTART;TZID=America/New_York:19970902T090000\r\nDTEND;TZID=America/New_York:19970902T100000\r\nRRULE:FREQ=DAILY;UNTIL=19971224T000000Z", "1997-09-01T00:00:00Z", "1997-12-26T00:00:00Z");
    assert_eq!(result.len(), 113);
    assert_eq!(result.last().unwrap().start, utc("1997-12-23T14:00:00Z"));
    assert!(result.iter().all(|event| event
        .start
        .with_timezone(&chrono_tz::America::New_York)
        .hour()
        == 9));
}

#[test]
fn rfc5545_tuesday_thursday_for_five_weeks() {
    let result = occurrences("DTSTART;TZID=America/New_York:19970902T090000\r\nDTEND;TZID=America/New_York:19970902T100000\r\nRRULE:FREQ=WEEKLY;COUNT=10;WKST=SU;BYDAY=TU,TH", "1997-09-01T00:00:00Z", "1997-10-05T00:00:00Z");
    let days: Vec<_> = result.iter().map(|event| event.start.day()).collect();
    assert_eq!(days, [2, 4, 9, 11, 16, 18, 23, 25, 30, 2]);
}

#[test]
fn rfc5545_last_weekday_of_month() {
    let result = occurrences("DTSTART;TZID=America/New_York:19970930T090000\r\nDTEND;TZID=America/New_York:19970930T100000\r\nRRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=4", "1997-09-01T00:00:00Z", "1998-01-02T00:00:00Z");
    let dates: Vec<_> = result
        .iter()
        .map(|event| {
            event
                .start
                .with_timezone(&chrono_tz::America::New_York)
                .date_naive()
                .to_string()
        })
        .collect();
    assert_eq!(
        dates,
        ["1997-09-30", "1997-10-31", "1997-11-28", "1997-12-31"]
    );
}

#[test]
fn count_applies_before_window_filter() {
    let result = occurrences(
        "DTSTART:20250101T090000Z\r\nDTEND:20250101T100000Z\r\nRRULE:FREQ=DAILY;COUNT=3",
        "2025-01-03T00:00:00Z",
        "2025-01-10T00:00:00Z",
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].start, utc("2025-01-03T09:00:00Z"));
}

#[test]
fn daily_interval_count_property() {
    let start = utc("2025-01-01T09:00:00Z");
    for interval in 1..=7 {
        for count in 1..=20 {
            let rule = format!("DTSTART:20250101T090000Z\r\nDTEND:20250101T100000Z\r\nRRULE:FREQ=DAILY;INTERVAL={interval};COUNT={count}");
            let calendar = fixture(&rule);
            let found = expand(
                &calendar,
                utc("2025-01-01T00:00:00Z"),
                utc("2025-07-01T00:00:00Z"),
                chrono_tz::UTC,
                100,
            )
            .unwrap();
            assert_eq!(found.len(), count as usize);
            for (index, occurrence) in found.iter().enumerate() {
                assert_eq!(
                    occurrence.start,
                    start + chrono::Duration::days((index as i64) * interval)
                );
                assert_eq!(
                    occurrence.end - occurrence.start,
                    chrono::Duration::hours(1)
                );
            }
        }
    }
}

#[test]
fn exdate_and_rdate_form_a_set() {
    let result = occurrences("DTSTART:20250101T090000Z\r\nDTEND:20250101T100000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEXDATE:20250102T090000Z\r\nRDATE:20250104T090000Z,20250104T090000Z", "2025-01-01T00:00:00Z", "2025-01-05T00:00:00Z");
    let days: Vec<_> = result.iter().map(|event| event.start.day()).collect();
    assert_eq!(days, [1, 3, 4]);
}

#[test]
fn detached_instance_moves_and_changes_summary() {
    let mut calendar =
        fixture("DTSTART:20250101T090000Z\r\nDTEND:20250101T100000Z\r\nRRULE:FREQ=DAILY;COUNT=3");
    let detached = fixture(
        "RECURRENCE-ID:20250102T090000Z\r\nDTSTART:20250105T120000Z\r\nDTEND:20250105T130000Z",
    );
    calendar.events.extend(detached.events);
    let result = expand(
        &calendar,
        utc("2025-01-01T00:00:00Z"),
        utc("2025-01-06T00:00:00Z"),
        chrono_tz::UTC,
        10,
    )
    .unwrap();
    assert_eq!(
        result
            .iter()
            .map(|event| event.start.day())
            .collect::<Vec<_>>(),
        [1, 3, 5]
    );
    assert_eq!(result[2].recurrence_id, utc("2025-01-02T09:00:00Z"));
}

#[test]
fn cancelled_detached_instance_is_omitted() {
    let mut calendar =
        fixture("DTSTART:20250101T090000Z\r\nDTEND:20250101T100000Z\r\nRRULE:FREQ=DAILY;COUNT=2");
    calendar.events.extend(fixture("RECURRENCE-ID:20250102T090000Z\r\nDTSTART:20250102T090000Z\r\nDTEND:20250102T100000Z\r\nSTATUS:CANCELLED").events);
    let result = expand(
        &calendar,
        utc("2025-01-01T00:00:00Z"),
        utc("2025-01-04T00:00:00Z"),
        chrono_tz::UTC,
        10,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
}

#[test]
fn new_york_recurrence_stays_at_nine_across_dst() {
    let result = occurrences("DTSTART;TZID=America/New_York:20250308T090000\r\nDTEND;TZID=America/New_York:20250308T100000\r\nRRULE:FREQ=DAILY;COUNT=3", "2025-03-08T00:00:00Z", "2025-03-12T00:00:00Z");
    assert_eq!(
        result
            .iter()
            .map(|event| event.start.hour())
            .collect::<Vec<_>>(),
        [14, 13, 13]
    );
    assert!(result
        .iter()
        .all(|event| event.end - event.start == chrono::Duration::hours(1)));
}

#[test]
fn floating_time_uses_supplied_zone() {
    let calendar = fixture("DTSTART:20250310T090000\r\nDTEND:20250310T100000");
    let result = expand(
        &calendar,
        utc("2025-03-10T00:00:00Z"),
        utc("2025-03-11T00:00:00Z"),
        chrono_tz::America::New_York,
        10,
    )
    .unwrap();
    assert_eq!(result[0].start, utc("2025-03-10T13:00:00Z"));
}

#[test]
fn all_day_exclusive_end_and_dst() {
    let calendar = fixture("DTSTART;VALUE=DATE:20250309\r\nDTEND;VALUE=DATE:20250310");
    let result = expand(
        &calendar,
        utc("2025-03-09T00:00:00Z"),
        utc("2025-03-11T00:00:00Z"),
        chrono_tz::America::New_York,
        10,
    )
    .unwrap();
    assert_eq!(result[0].end - result[0].start, chrono::Duration::hours(23));
    assert!(result[0].all_day);
}

#[test]
fn fall_back_all_day_occurrence_overlaps_next_utc_day() {
    let calendar = fixture(
        "DTSTART;VALUE=DATE:20251102\r\nDTEND;VALUE=DATE:20251103\r\nRRULE:FREQ=DAILY;COUNT=2",
    );
    let result = expand(
        &calendar,
        utc("2025-11-03T04:30:00Z"),
        utc("2025-11-03T06:00:00Z"),
        chrono_tz::America::New_York,
        10,
    )
    .unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].end - result[0].start, chrono::Duration::hours(25));
}

#[test]
fn parse_serialise_round_trip_folded_unicode_and_alarm() {
    let source = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:u1\r\nSUMMARY:Crème\\, tea\\nroom\r\nDTSTART;TZID=Europe/Paris:20250102T090000\r\nDTEND;TZID=Europe/Paris:20250102T100000\r\nDESCRIPTION:Something long\r\n and folded\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let parsed = Calendar::parse(source).unwrap();
    assert_eq!(parsed.events[0].summary, "Crème, tea\nroom");
    assert_eq!(
        parsed.events[0].other_properties,
        [
            "DESCRIPTION:Something longand folded",
            "BEGIN:VALARM",
            "ACTION:DISPLAY",
            "END:VALARM"
        ]
    );
    let saved = parsed.to_ical();
    assert!(saved
        .lines()
        .all(|line| line.trim_end_matches('\r').len() <= 75));
    assert_eq!(Calendar::parse(&saved).unwrap(), parsed);
}

#[test]
fn serialization_folds_utf8_without_splitting_codepoints() {
    let mut calendar = fixture("DTSTART:20250101T090000Z\r\nDTEND:20250101T100000Z");
    calendar.events[0].summary = "Crème café 日本語 ".repeat(12);
    let saved = calendar.to_ical();
    assert!(saved.contains("\r\n "));
    assert!(saved
        .lines()
        .all(|line| line.trim_end_matches('\r').len() <= 75));
    assert_eq!(Calendar::parse(&saved).unwrap(), calendar);
}

#[test]
fn invalid_external_data_returns_errors() {
    for source in [
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VEVENT\r\nEND:VCALENDAR",
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=Mars/Olympus:20250101T090000\r\nEND:VEVENT\r\nEND:VCALENDAR",
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20250101T090000Z\r\nDTEND:20241231T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nUID:y\r\nDTSTART:20250101T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR",
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20250101T090000Z\r\nDURATION:PT1H\r\nEND:VEVENT\r\nEND:VCALENDAR",
        " BEGIN:VCALENDAR",
    ] {
        assert!(Calendar::parse(source).is_err(), "{source}");
    }
}

#[test]
fn empty_calendar_is_valid() {
    let calendar = Calendar::parse("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n").unwrap();
    assert!(calendar.events.is_empty());
    assert_eq!(Calendar::parse(&calendar.to_ical()).unwrap(), calendar);
}

#[test]
fn spring_gap_uses_pre_transition_offset() {
    let calendar = fixture("DTSTART;TZID=America/New_York:20250309T023000\r\nDTEND;TZID=America/New_York:20250309T033000");
    assert_eq!(
        calendar.events[0].start.resolve(chrono_tz::UTC).unwrap(),
        utc("2025-03-09T07:30:00Z")
    );
}

#[test]
fn fall_ambiguous_time_uses_first_occurrence() {
    let calendar = fixture("DTSTART;TZID=America/New_York:20251102T013000\r\nDTEND;TZID=America/New_York:20251102T023000");
    assert_eq!(
        calendar.events[0].start.resolve(chrono_tz::UTC).unwrap(),
        utc("2025-11-02T05:30:00Z")
    );
}

#[test]
fn occurrence_limit_reports_error() {
    let calendar =
        fixture("DTSTART:20250101T090000Z\r\nDTEND:20250101T100000Z\r\nRRULE:FREQ=DAILY;COUNT=3");
    assert!(expand(
        &calendar,
        utc("2025-01-01T00:00:00Z"),
        utc("2025-01-05T00:00:00Z"),
        chrono_tz::UTC,
        2
    )
    .is_err());
}

#[test]
fn malformed_rrule_returns_error() {
    let calendar =
        fixture("DTSTART:20250101T090000Z\r\nDTEND:20250101T100000Z\r\nRRULE:FREQ=GLORP");
    assert!(expand(
        &calendar,
        utc("2025-01-01T00:00:00Z"),
        utc("2025-01-05T00:00:00Z"),
        chrono_tz::UTC,
        10
    )
    .is_err());
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 1, 6).unwrap()
}
fn event(id: &str, start: u32, end: u32) -> LayoutEvent {
    LayoutEvent {
        id: id.into(),
        start: Utc.with_ymd_and_hms(2025, 1, 6, start, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2025, 1, 6, end, 0, 0).unwrap(),
    }
}

#[test]
fn layout_golden_three_way_overlap() {
    let result = layout_day(
        &[event("a", 9, 11), event("b", 9, 10), event("c", 9, 10)],
        day(),
        chrono_tz::UTC,
    )
    .unwrap();
    let golden: Vec<_> = result
        .iter()
        .map(|slot| {
            (
                slot.id.as_str(),
                slot.start_second,
                slot.end_second,
                slot.column,
                slot.columns,
            )
        })
        .collect();
    assert_eq!(
        golden,
        [
            ("a", 32400, 39600, 0, 3),
            ("b", 32400, 36000, 1, 3),
            ("c", 32400, 36000, 2, 3)
        ]
    );
}

#[test]
fn layout_golden_touching_groups_and_split_midnight() {
    let long = LayoutEvent {
        id: "night".into(),
        start: utc("2025-01-05T23:00:00Z"),
        end: utc("2025-01-06T02:00:00Z"),
    };
    let result = layout_day(
        &[long, event("a", 9, 10), event("b", 10, 11)],
        day(),
        chrono_tz::UTC,
    )
    .unwrap();
    let golden: Vec<_> = result
        .iter()
        .map(|slot| {
            (
                slot.id.as_str(),
                slot.start_second,
                slot.end_second,
                slot.column,
                slot.columns,
            )
        })
        .collect();
    assert_eq!(
        golden,
        [
            ("night", 0, 7200, 0, 1),
            ("a", 32400, 36000, 0, 1),
            ("b", 36000, 39600, 0, 1)
        ]
    );
}

#[test]
fn layout_property_overlapping_intervals_never_share_column() {
    let mut seed = 1u64;
    for _ in 0..100 {
        let mut events = Vec::new();
        for index in 0..20 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let start = ((seed >> 32) % 23) as u32;
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let end = (start + 1 + ((seed >> 32) % 3) as u32).min(23);
            events.push(event(&index.to_string(), start, end));
        }
        let slots = layout_day(&events, day(), chrono_tz::UTC).unwrap();
        for (index, a) in slots.iter().enumerate() {
            assert!(a.column < a.columns);
            for b in slots.iter().skip(index + 1) {
                if a.start_second < b.end_second && b.start_second < a.end_second {
                    assert_ne!(a.column, b.column);
                }
            }
        }
    }
}

#[test]
fn layout_dst_day_has_23_hours() {
    let date = NaiveDate::from_ymd_opt(2025, 3, 9).unwrap();
    let whole = LayoutEvent {
        id: "whole".into(),
        start: utc("2025-03-09T05:00:00Z"),
        end: utc("2025-03-10T04:00:00Z"),
    };
    let slots = layout_day(&[whole], date, Tz::America__New_York).unwrap();
    assert_eq!(slots[0].end_second, 23 * 3600);
}

#[test]
fn layout_week_golden_splits_overnight_event() {
    let start = NaiveDate::from_ymd_opt(2025, 1, 6).unwrap();
    let overnight = LayoutEvent {
        id: "night".into(),
        start: utc("2025-01-06T23:00:00Z"),
        end: utc("2025-01-07T02:00:00Z"),
    };
    let slots = layout_week(&[overnight], start, chrono_tz::UTC).unwrap();
    let golden: Vec<_> = slots
        .iter()
        .map(|slot| {
            (
                slot.day.to_string(),
                slot.start_second,
                slot.end_second,
                slot.column,
                slot.columns,
            )
        })
        .collect();
    assert_eq!(
        golden,
        [
            ("2025-01-06".into(), 82800, 86400, 0, 1),
            ("2025-01-07".into(), 0, 7200, 0, 1)
        ]
    );
}
