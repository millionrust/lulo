//! VALARM parsing and alarm-trigger computation.
//!
//! `rmac_calendar_store::Event` does not model VALARM (it keeps every
//! unrecognised VEVENT property, alarms included, as raw RFC 5545 lines in
//! `other_properties` -- see `crates/calendar/src/editing.rs`'s own alert
//! round trip). This module reads those raw lines back out and turns them
//! into concrete trigger times for a window of expanded occurrences. Pure
//! and allocation-light so it is cheap to re-run on every EDS change.

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use rmac_calendar_store::{expand, Calendar as IcalCalendar, CalendarError};

/// What a relative [`Trigger`] is measured from (RFC 5545 `TRIGGER;RELATED=`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Related {
    Start,
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trigger {
    Relative { offset: Duration, related: Related },
    Absolute(DateTime<Utc>),
}

/// One alarm this agent will notify for, tied to a specific occurrence (a
/// recurring event's alarm fires once per instance, never once per series).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveAlarm {
    pub calendar_uid: String,
    pub event_uid: String,
    pub occurrence_start: DateTime<Utc>,
    pub trigger_at: DateTime<Utc>,
    pub summary: String,
    pub all_day: bool,
}

/// Every alarm trigger on one VEVENT (or detached override) that this agent
/// notifies for. `ACTION:EMAIL` and `ACTION:PROCEDURE` are skipped on
/// purpose: Beta 1 only ever shows a Lulo notification, never sends mail or
/// runs a command from calendar data. A VALARM with no ACTION at all (not
/// standards-conformant, but EDS always writes one and a hand-edited .ics
/// might not) is treated the same as `DISPLAY`.
pub fn parse_valarms(other_properties: &[String]) -> Vec<Trigger> {
    let mut triggers = Vec::new();
    let mut in_alarm = false;
    let mut action: Option<&str> = None;
    let mut trigger: Option<Trigger> = None;
    for line in other_properties {
        match line.as_str() {
            "BEGIN:VALARM" => {
                in_alarm = true;
                action = None;
                trigger = None;
                continue;
            }
            "END:VALARM" => {
                if in_alarm {
                    let notifies = action
                        .map(|action| {
                            action.eq_ignore_ascii_case("DISPLAY")
                                || action.eq_ignore_ascii_case("AUDIO")
                        })
                        .unwrap_or(true);
                    if notifies {
                        if let Some(trigger) = trigger {
                            triggers.push(trigger);
                        }
                    }
                }
                in_alarm = false;
                continue;
            }
            _ => {}
        }
        if !in_alarm {
            continue;
        }
        let Some((name_and_params, value)) = line.split_once(':') else {
            continue;
        };
        let name = name_and_params.split(';').next().unwrap_or_default();
        match name {
            "ACTION" => action = Some(value.trim()),
            "TRIGGER" => trigger = parse_trigger(name_and_params, value),
            _ => {}
        }
    }
    triggers
}

fn parse_trigger(name_and_params: &str, value: &str) -> Option<Trigger> {
    let mut related_end = false;
    let mut absolute = false;
    for parameter in name_and_params.split(';').skip(1) {
        let (key, parameter_value) = parameter.split_once('=')?;
        match key {
            "RELATED" if parameter_value.eq_ignore_ascii_case("END") => related_end = true,
            "VALUE" if parameter_value.eq_ignore_ascii_case("DATE-TIME") => absolute = true,
            _ => {}
        }
    }
    if absolute {
        return parse_absolute(value).map(Trigger::Absolute);
    }
    parse_iso_duration(value).map(|offset| Trigger::Relative {
        offset,
        related: if related_end {
            Related::End
        } else {
            Related::Start
        },
    })
}

fn parse_absolute(value: &str) -> Option<DateTime<Utc>> {
    let raw = value.strip_suffix('Z')?;
    let naive = chrono::NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S").ok()?;
    Some(naive.and_utc())
}

/// A bounded RFC 5545 `dur-value`: optional sign, `P`, then weeks, or days
/// optionally followed by a `T` time part (hours/minutes/seconds). No
/// months -- RFC 5545 durations do not have one -- and no regex: the
/// grammar is small and fixed.
fn parse_iso_duration(value: &str) -> Option<Duration> {
    let (negative, rest) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let rest = rest.strip_prefix('P')?;
    let mut seconds: i64 = 0;
    let mut number = String::new();
    let mut in_time = false;
    let mut saw_any = false;
    for ch in rest.chars() {
        match ch {
            'T' if !in_time && number.is_empty() => in_time = true,
            '0'..='9' => number.push(ch),
            'W' | 'D' | 'H' | 'M' | 'S' => {
                if number.is_empty() {
                    return None;
                }
                let amount: i64 = number.parse().ok()?;
                number.clear();
                let unit_seconds = match ch {
                    'W' if !in_time => 7 * 24 * 3600,
                    'D' if !in_time => 24 * 3600,
                    'H' if in_time => 3600,
                    'M' if in_time => 60,
                    'S' if in_time => 1,
                    _ => return None,
                };
                seconds = seconds.checked_add(amount.checked_mul(unit_seconds)?)?;
                saw_any = true;
            }
            _ => return None,
        }
    }
    if !saw_any || !number.is_empty() {
        return None;
    }
    let seconds = if negative { -seconds } else { seconds };
    Some(Duration::seconds(seconds))
}

/// Every alarm in `calendar` (one EDS source) whose owning occurrence falls
/// in `[window_start, window_end)`. `calendar` holds every VEVENT the source
/// returned, masters and detached overrides alike, exactly as
/// `rmac_calendar_store::expand` expects.
pub fn alarms_for_calendar(
    calendar_uid: &str,
    calendar: &IcalCalendar,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    floating_zone: Tz,
) -> Result<Vec<ActiveAlarm>, CalendarError> {
    let mut alarms = Vec::new();
    for occurrence in expand(calendar, window_start, window_end, floating_zone, 10_000)? {
        let Some(master) = calendar
            .events
            .iter()
            .find(|event| event.uid == occurrence.uid && event.recurrence_id.is_none())
        else {
            continue;
        };
        let source_event = calendar
            .events
            .iter()
            .find(|event| {
                event.uid == occurrence.uid
                    && event
                        .recurrence_id
                        .and_then(|id| id.resolve(floating_zone).ok())
                        == Some(occurrence.recurrence_id)
            })
            .unwrap_or(master);
        for trigger in parse_valarms(&source_event.other_properties) {
            let trigger_at = match trigger {
                Trigger::Absolute(at) => at,
                Trigger::Relative { offset, related } => {
                    let anchor = match related {
                        Related::Start => occurrence.start,
                        Related::End => occurrence.end,
                    };
                    let Some(at) = anchor.checked_add_signed(offset) else {
                        continue;
                    };
                    at
                }
            };
            alarms.push(ActiveAlarm {
                calendar_uid: calendar_uid.to_owned(),
                event_uid: occurrence.uid.clone(),
                occurrence_start: occurrence.start,
                trigger_at,
                summary: occurrence.summary.clone(),
                all_day: occurrence.all_day,
            });
        }
    }
    Ok(alarms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_relative_and_absolute_triggers_and_skips_non_display_actions() {
        let triggers = parse_valarms(&[
            "BEGIN:VALARM".into(),
            "TRIGGER:-PT15M".into(),
            "ACTION:DISPLAY".into(),
            "END:VALARM".into(),
            "BEGIN:VALARM".into(),
            "TRIGGER;RELATED=END:PT5M".into(),
            "ACTION:AUDIO".into(),
            "END:VALARM".into(),
            "BEGIN:VALARM".into(),
            "TRIGGER;VALUE=DATE-TIME:20261006T090000Z".into(),
            "ACTION:DISPLAY".into(),
            "END:VALARM".into(),
            "BEGIN:VALARM".into(),
            "TRIGGER:-P1D".into(),
            "ACTION:EMAIL".into(),
            "END:VALARM".into(),
        ]);
        assert_eq!(
            triggers,
            vec![
                Trigger::Relative {
                    offset: Duration::minutes(-15),
                    related: Related::Start,
                },
                Trigger::Relative {
                    offset: Duration::minutes(5),
                    related: Related::End,
                },
                Trigger::Absolute(
                    chrono::NaiveDate::from_ymd_opt(2026, 10, 6)
                        .unwrap()
                        .and_hms_opt(9, 0, 0)
                        .unwrap()
                        .and_utc()
                ),
            ]
        );
    }

    #[test]
    fn treats_a_missing_action_as_display() {
        let triggers = parse_valarms(&[
            "BEGIN:VALARM".into(),
            "TRIGGER:-PT30M".into(),
            "END:VALARM".into(),
        ]);
        assert_eq!(triggers.len(), 1);
    }

    #[test]
    fn parses_durations_with_weeks_days_and_combined_time_parts() {
        assert_eq!(parse_iso_duration("PT0S"), Some(Duration::seconds(0)));
        assert_eq!(parse_iso_duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(parse_iso_duration("-P1D"), Some(Duration::days(-1)));
        assert_eq!(parse_iso_duration("-P1W"), Some(Duration::weeks(-1)));
        assert_eq!(parse_iso_duration("-PT15H"), Some(Duration::hours(-15)));
        assert_eq!(
            parse_iso_duration("P1DT2H30M"),
            Some(Duration::hours(26) + Duration::minutes(30))
        );
    }

    #[test]
    fn rejects_malformed_durations() {
        for bad in ["P", "PT", "P1M", "PT1X", "1D", "P-1D", "PTD"] {
            assert_eq!(parse_iso_duration(bad), None, "{bad}");
        }
    }

    const DAILY_EVENT: &str = "BEGIN:VEVENT\r\n\
UID:daily@local\r\n\
SUMMARY:Standup\r\n\
DTSTART:20261005T090000Z\r\n\
DTEND:20261005T091500Z\r\n\
RRULE:FREQ=DAILY;COUNT=3\r\n\
BEGIN:VALARM\r\n\
TRIGGER:-PT15M\r\n\
ACTION:DISPLAY\r\n\
END:VALARM\r\n\
END:VEVENT\r\n";

    fn parse(ical: &str) -> IcalCalendar {
        IcalCalendar::parse(&format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{ical}END:VCALENDAR\r\n"
        ))
        .unwrap()
    }

    #[test]
    fn computes_one_trigger_per_recurring_occurrence() {
        let calendar = parse(DAILY_EVENT);
        let start = chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let end = start + Duration::days(10);
        let alarms = alarms_for_calendar("local", &calendar, start, end, chrono_tz::UTC).unwrap();
        assert_eq!(alarms.len(), 3);
        assert_eq!(
            alarms[0].trigger_at,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 5)
                .unwrap()
                .and_hms_opt(8, 45, 0)
                .unwrap()
                .and_utc()
        );
        assert_eq!(alarms[0].calendar_uid, "local");
        assert_eq!(alarms[0].event_uid, "daily@local");
        assert!(!alarms[0].all_day);
    }

    #[test]
    fn related_end_triggers_from_the_occurrence_end() {
        let ical = "BEGIN:VEVENT\r\n\
UID:end@local\r\n\
SUMMARY:Review\r\n\
DTSTART:20261005T100000Z\r\n\
DTEND:20261005T110000Z\r\n\
BEGIN:VALARM\r\n\
TRIGGER;RELATED=END:PT10M\r\n\
ACTION:DISPLAY\r\n\
END:VALARM\r\n\
END:VEVENT\r\n";
        let calendar = parse(ical);
        let start = chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let end = start + Duration::days(10);
        let alarms = alarms_for_calendar("local", &calendar, start, end, chrono_tz::UTC).unwrap();
        assert_eq!(alarms.len(), 1);
        assert_eq!(
            alarms[0].trigger_at,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 5)
                .unwrap()
                .and_hms_opt(11, 10, 0)
                .unwrap()
                .and_utc()
        );
    }

    #[test]
    fn all_day_default_alert_fires_at_nine_am_the_day_before() {
        let ical = "BEGIN:VEVENT\r\n\
UID:birthday@local\r\n\
SUMMARY:Trip\r\n\
DTSTART;VALUE=DATE:20261010\r\n\
DTEND;VALUE=DATE:20261011\r\n\
BEGIN:VALARM\r\n\
TRIGGER:-PT15H\r\n\
ACTION:DISPLAY\r\n\
END:VALARM\r\n\
END:VEVENT\r\n";
        let calendar = parse(ical);
        let start = chrono::NaiveDate::from_ymd_opt(2026, 10, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let end = start + Duration::days(20);
        let alarms = alarms_for_calendar("local", &calendar, start, end, chrono_tz::UTC).unwrap();
        assert_eq!(alarms.len(), 1);
        assert!(alarms[0].all_day);
        assert_eq!(
            alarms[0].trigger_at,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 9)
                .unwrap()
                .and_hms_opt(9, 0, 0)
                .unwrap()
                .and_utc()
        );
    }
}
