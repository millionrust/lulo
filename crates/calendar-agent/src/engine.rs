//! Pure alarm-evaluation core.
//!
//! Given "now" and every open calendar's events, decide what to show right
//! now and when to wake next. No I/O here at all: `crate::linux` is the only
//! caller in production, and unit tests drive this directly with whatever
//! `now` they like -- a fake clock is just a `DateTime<Utc>` value, nothing
//! more (ADR 0022 §6, CAL-7).

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use rmac_calendar_store::{Calendar as IcalCalendar, CalendarError};

use crate::alarms::{alarms_for_calendar, ActiveAlarm};
use crate::state::{self, AlarmRecord, AlarmStatus, AlertState};

/// How far ahead occurrences are expanded to find the next alarm. Generous
/// enough for a "1 week before" all-day default or a yearly birthday-style
/// RRULE; an alarm further out than this is picked up once something nearer
/// re-triggers a scan (an EDS change, a snooze, or this horizon itself on
/// the next restart).
pub const HORIZON: Duration = Duration::days(400);
/// How far an alarm's trigger may lead its occurrence. Bounds the
/// expansion window's start so a lead longer than any default (a custom
/// alert further out) is still found.
pub const MAX_LEAD: Duration = Duration::days(14);

/// A source of the current time, for the one piece of runtime glue
/// (`crate::linux`) that is not itself a pure function of an explicit `now`.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Evaluation {
    /// Alarms to notify for right now, in no particular order.
    pub due: Vec<ActiveAlarm>,
    /// The next real-time deadline to re-arm the timer for, if anything is
    /// still pending.
    pub next_wake: Option<DateTime<Utc>>,
}

/// Recompute what is due and when to wake next, updating `state` in place:
/// newly-due alarms become `Shown`, alarms too stale to show become
/// `Closed`, and nothing already `Closed` or already `Shown` for its exact
/// trigger resurfaces. Idempotent: calling this again with the same `now`
/// and no new calendar data returns an empty `due` list.
pub fn evaluate(
    now: DateTime<Utc>,
    calendars: &BTreeMap<String, IcalCalendar>,
    floating_zone: Tz,
    state: &mut AlertState,
) -> Result<Evaluation, CalendarError> {
    let window_start = now - MAX_LEAD - state::MAX_MISSED_AGE;
    let window_end = now + HORIZON;
    let mut evaluation = Evaluation::default();
    for (calendar_uid, calendar) in calendars {
        for alarm in alarms_for_calendar(
            calendar_uid,
            calendar,
            window_start,
            window_end,
            floating_zone,
        )? {
            let key = state::key(
                calendar_uid,
                &alarm.event_uid,
                alarm.occurrence_start,
                alarm.trigger_at,
            );
            let record = state.alarms.get(&key).cloned();
            if matches!(
                record,
                Some(AlarmRecord {
                    status: AlarmStatus::Closed,
                    ..
                })
            ) {
                continue;
            }
            let already_shown = matches!(
                record,
                Some(AlarmRecord {
                    status: AlarmStatus::Shown,
                    ..
                })
            );
            let effective_at = match &record {
                Some(AlarmRecord {
                    status: AlarmStatus::Snoozed,
                    snoozed_until: Some(until),
                    ..
                }) => *until,
                _ => alarm.trigger_at,
            };
            if effective_at <= now - state::MAX_MISSED_AGE {
                state.alarms.insert(
                    key,
                    AlarmRecord {
                        trigger_at: alarm.trigger_at,
                        status: AlarmStatus::Closed,
                        snoozed_until: None,
                    },
                );
                continue;
            }
            if effective_at <= now {
                if !already_shown {
                    state.alarms.insert(
                        key,
                        AlarmRecord {
                            trigger_at: alarm.trigger_at,
                            status: AlarmStatus::Shown,
                            snoozed_until: None,
                        },
                    );
                    evaluation.due.push(alarm);
                }
            } else {
                evaluation.next_wake = Some(
                    evaluation
                        .next_wake
                        .map_or(effective_at, |wake| wake.min(effective_at)),
                );
            }
        }
    }
    state.prune(now);
    Ok(evaluation)
}

/// Snooze one alarm (the notification's "Snooze" action): its effective
/// trigger becomes `until`, superseding the natural one until it fires or
/// the alarm is closed.
pub fn snooze(
    state: &mut AlertState,
    calendar_uid: &str,
    event_uid: &str,
    occurrence_start: DateTime<Utc>,
    trigger_at: DateTime<Utc>,
    until: DateTime<Utc>,
) {
    let key = state::key(calendar_uid, event_uid, occurrence_start, trigger_at);
    state.alarms.insert(
        key,
        AlarmRecord {
            trigger_at,
            status: AlarmStatus::Snoozed,
            snoozed_until: Some(until),
        },
    );
}

/// Dismiss one alarm (the notification's "Close" action, or clicking it to
/// open the event): it never resurfaces.
pub fn close(
    state: &mut AlertState,
    calendar_uid: &str,
    event_uid: &str,
    occurrence_start: DateTime<Utc>,
    trigger_at: DateTime<Utc>,
) {
    let key = state::key(calendar_uid, event_uid, occurrence_start, trigger_at);
    state.alarms.insert(
        key,
        AlarmRecord {
            trigger_at,
            status: AlarmStatus::Closed,
            snoozed_until: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn calendar_with(ical: &str) -> IcalCalendar {
        IcalCalendar::parse(&format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{ical}END:VCALENDAR\r\n"
        ))
        .unwrap()
    }

    fn single_event(start: DateTime<Utc>, minutes_before: i64) -> BTreeMap<String, IcalCalendar> {
        let ical = format!(
            "BEGIN:VEVENT\r\nUID:a@local\r\nSUMMARY:Standup\r\nDTSTART:{}\r\nDTEND:{}\r\n\
             BEGIN:VALARM\r\nTRIGGER:-PT{minutes_before}M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\n",
            start.format("%Y%m%dT%H%M%SZ"),
            (start + Duration::minutes(15)).format("%Y%m%dT%H%M%SZ"),
        );
        BTreeMap::from([("local".to_owned(), calendar_with(&ical))])
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    #[test]
    fn a_future_trigger_only_sets_the_next_wake() {
        let start = at(10_000);
        let calendars = single_event(start, 15);
        let mut state = AlertState::default();
        let evaluation = evaluate(at(0), &calendars, chrono_tz::UTC, &mut state).unwrap();
        assert!(evaluation.due.is_empty());
        assert_eq!(evaluation.next_wake, Some(start - Duration::minutes(15)));
    }

    #[test]
    fn a_due_trigger_fires_once_and_never_refires_on_rescan() {
        let start = at(10_000);
        let calendars = single_event(start, 15);
        let mut state = AlertState::default();
        let trigger_at = start - Duration::minutes(15);

        let first = evaluate(trigger_at, &calendars, chrono_tz::UTC, &mut state).unwrap();
        assert_eq!(first.due.len(), 1);
        assert_eq!(first.due[0].trigger_at, trigger_at);

        let second = evaluate(trigger_at, &calendars, chrono_tz::UTC, &mut state).unwrap();
        assert!(second.due.is_empty());
        let third = evaluate(
            trigger_at + Duration::minutes(1),
            &calendars,
            chrono_tz::UTC,
            &mut state,
        )
        .unwrap();
        assert!(third.due.is_empty());
    }

    #[test]
    fn an_alarm_older_than_the_missed_window_is_closed_without_firing() {
        let start = at(10_000);
        let calendars = single_event(start, 15);
        let mut state = AlertState::default();
        let trigger_at = start - Duration::minutes(15);
        let now = trigger_at + state::MAX_MISSED_AGE + Duration::minutes(1);
        let evaluation = evaluate(now, &calendars, chrono_tz::UTC, &mut state).unwrap();
        assert!(evaluation.due.is_empty());
        assert!(evaluation.next_wake.is_none());
    }

    #[test]
    fn snoozing_defers_the_next_fire_and_closing_suppresses_it() {
        let start = at(10_000);
        let calendars = single_event(start, 15);
        let trigger_at = start - Duration::minutes(15);

        let mut state = AlertState::default();
        evaluate(trigger_at, &calendars, chrono_tz::UTC, &mut state).unwrap();
        let until = trigger_at + Duration::minutes(5);
        snooze(&mut state, "local", "a@local", start, trigger_at, until);
        let snoozed = evaluate(
            trigger_at + Duration::seconds(1),
            &calendars,
            chrono_tz::UTC,
            &mut state,
        )
        .unwrap();
        assert!(snoozed.due.is_empty());
        assert_eq!(snoozed.next_wake, Some(until));
        let woken = evaluate(until, &calendars, chrono_tz::UTC, &mut state).unwrap();
        assert_eq!(woken.due.len(), 1);

        let mut closed_state = AlertState::default();
        evaluate(trigger_at, &calendars, chrono_tz::UTC, &mut closed_state).unwrap();
        close(&mut closed_state, "local", "a@local", start, trigger_at);
        let after_close = evaluate(
            trigger_at + Duration::hours(1),
            &calendars,
            chrono_tz::UTC,
            &mut closed_state,
        )
        .unwrap();
        assert!(after_close.due.is_empty());
        assert!(after_close.next_wake.is_none());
    }

    #[test]
    fn multiple_alarms_on_one_occurrence_do_not_collide() {
        let start = at(100_000);
        let ical = format!(
            "BEGIN:VEVENT\r\nUID:a@local\r\nSUMMARY:Review\r\nDTSTART:{}\r\nDTEND:{}\r\n\
             BEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
             BEGIN:VALARM\r\nTRIGGER:-P1D\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\n",
            start.format("%Y%m%dT%H%M%SZ"),
            (start + Duration::minutes(30)).format("%Y%m%dT%H%M%SZ"),
        );
        let calendars = BTreeMap::from([("local".to_owned(), calendar_with(&ical))]);
        let mut state = AlertState::default();
        let day_before = evaluate(
            start - Duration::days(1),
            &calendars,
            chrono_tz::UTC,
            &mut state,
        )
        .unwrap();
        assert_eq!(day_before.due.len(), 1);
        let fifteen_before = evaluate(
            start - Duration::minutes(15),
            &calendars,
            chrono_tz::UTC,
            &mut state,
        )
        .unwrap();
        assert_eq!(fifteen_before.due.len(), 1);
        assert_ne!(
            day_before.due[0].trigger_at,
            fifteen_before.due[0].trigger_at
        );
    }
}
