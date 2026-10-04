//! The next events across every enabled calendar, for the Calendar widget
//! (CAL-8; the design doc's "the widget shows the next events"). Two ways
//! to read it: [`load`] is one-shot, for Notification Centre's transient
//! panel (reads once per open, no background subscription); [`watch`] is
//! event-driven -- one EDS view per enabled calendar, blocking without
//! polling -- for the always-running desktop widget host
//! (`rmac-wallpaper`), which mirrors `rmac-calendar-agent`'s own
//! `linux::watch_calendar` architecture.

#[cfg(target_os = "linux")]
use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use rmac_calendar_store::{expand, Calendar as IcalCalendar};

#[derive(Clone, Debug, PartialEq)]
pub struct UpcomingEvent {
    pub calendar_uid: String,
    pub event_uid: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
}

/// How far ahead to look. Long enough that an idle weekend still shows
/// Monday's first meeting.
const HORIZON: Duration = Duration::days(14);

/// Pure: the next `limit` occurrences at or after `now` across
/// already-parsed calendars, soonest first. Separate from [`load`] so the
/// picking and ordering logic is unit-testable without EDS.
pub fn next_events(
    calendars: &[(String, IcalCalendar)],
    now: DateTime<Utc>,
    limit: usize,
) -> Vec<UpcomingEvent> {
    let mut events = Vec::new();
    for (calendar_uid, calendar) in calendars {
        let Ok(occurrences) = expand(calendar, now, now + HORIZON, chrono_tz::UTC, 2_000) else {
            continue;
        };
        for occurrence in occurrences {
            if occurrence.end <= now {
                continue;
            }
            events.push(UpcomingEvent {
                calendar_uid: calendar_uid.clone(),
                event_uid: occurrence.uid,
                title: occurrence.summary,
                start: occurrence.start,
                end: occurrence.end,
                all_day: occurrence.all_day,
            });
        }
    }
    events.sort_by_key(|event| event.start);
    events.truncate(limit);
    events
}

#[cfg(target_os = "linux")]
pub fn load(limit: usize) -> Result<Vec<UpcomingEvent>, String> {
    use rmac_calendar_eds::Eds;
    let eds = Eds::session().map_err(|_| "Calendar service unavailable".to_owned())?;
    eds.check_available()
        .map_err(|_| "Calendar service unavailable".to_owned())?;
    let mut calendars = Vec::new();
    for source in eds
        .sources()
        .map_err(|_| "Couldn't load calendars".to_owned())?
        .into_iter()
        .filter(|source| source.enabled)
    {
        let Ok(client) = eds.open(&source.uid) else {
            continue;
        };
        let Ok(objects) = client.object_list("#t") else {
            continue;
        };
        let mut ical = IcalCalendar { events: Vec::new() };
        for raw in objects {
            let wrapped = if raw.contains("BEGIN:VCALENDAR") {
                raw
            } else {
                format!("BEGIN:VCALENDAR\nVERSION:2.0\n{raw}\nEND:VCALENDAR")
            };
            if let Ok(parsed) = IcalCalendar::parse(&wrapped) {
                ical.events.extend(parsed.events);
            }
        }
        calendars.push((source.uid, ical));
    }
    Ok(next_events(&calendars, Utc::now(), limit))
}

#[cfg(not(target_os = "linux"))]
pub fn load(_limit: usize) -> Result<Vec<UpcomingEvent>, String> {
    Err("Calendar service unavailable".into())
}

/// Blocks, calling `on_update` with the current Up Next list every time it
/// could have changed (the first load, then any EDS view change on any
/// enabled calendar). Returns once there is nothing left to watch: no EDS,
/// no enabled calendar, a transport error, or `on_update` returning
/// `false`. A calendar source added, removed, enabled or disabled while
/// this runs is not picked up -- like the reminders agent, the caller
/// should simply call `watch` again (CAL-8 simplification; `linux.rs`'s own
/// doc comment explains why a live view watcher cannot be torn down
/// mid-process with EDS's blocking API).
#[cfg(target_os = "linux")]
pub fn watch(limit: usize, mut on_update: impl FnMut(Vec<UpcomingEvent>) -> bool) {
    use rmac_calendar_eds::Eds;
    use rmac_calendar_runtime::CalendarRuntime;
    use std::sync::mpsc;
    use std::thread;

    let Ok(eds) = Eds::session() else {
        return;
    };
    if eds.check_available().is_err() {
        return;
    }
    let mut runtime = CalendarRuntime::new(eds.clone());
    if runtime.reload_sources().is_err() {
        return;
    }
    let sources: Vec<_> = runtime
        .sources()
        .iter()
        .filter(|source| source.enabled)
        .cloned()
        .collect();
    if sources.is_empty() {
        on_update(Vec::new());
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, BTreeMap<String, String>)>();
    for source in &sources {
        let eds = eds.clone();
        let uid = source.uid.clone();
        let tx = tx.clone();
        thread::spawn(move || watch_one_calendar(eds, uid, tx));
    }
    drop(tx);
    let mut calendars: BTreeMap<String, IcalCalendar> = BTreeMap::new();
    while let Ok((uid, objects)) = rx.recv() {
        calendars.insert(uid, parse_objects(&objects));
        let snapshot: Vec<(String, IcalCalendar)> = calendars
            .iter()
            .map(|(uid, calendar)| (uid.clone(), calendar.clone()))
            .collect();
        if !on_update(next_events(&snapshot, Utc::now(), limit)) {
            return;
        }
    }
}

/// One thread per enabled calendar, same as `linux::watch_calendar`: sends
/// the initial snapshot, then blocks on EDS view changes with no polling.
#[cfg(target_os = "linux")]
fn watch_one_calendar(
    eds: rmac_calendar_eds::Eds,
    uid: String,
    tx: std::sync::mpsc::Sender<(String, BTreeMap<String, String>)>,
) {
    use rmac_calendar_runtime::CalendarRuntime;
    let mut runtime = CalendarRuntime::new(eds);
    if runtime.reload_sources().is_err() {
        return;
    }
    let objects = match runtime.open(&uid) {
        Ok(snapshot) => snapshot.objects.clone(),
        Err(_) => return,
    };
    if tx.send((uid.clone(), objects)).is_err() {
        return;
    }
    loop {
        match runtime.next_event(&uid) {
            Ok(Some(snapshot)) => {
                if tx.send((uid.clone(), snapshot.objects.clone())).is_err() {
                    return;
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

#[cfg(target_os = "linux")]
fn parse_objects(objects: &BTreeMap<String, String>) -> IcalCalendar {
    let mut events = Vec::new();
    for raw in objects.values() {
        let wrapped = if raw.contains("BEGIN:VCALENDAR") {
            raw.clone()
        } else {
            format!("BEGIN:VCALENDAR\nVERSION:2.0\n{raw}\nEND:VCALENDAR")
        };
        if let Ok(parsed) = IcalCalendar::parse(&wrapped) {
            events.extend(parsed.events);
        }
    }
    IcalCalendar { events }
}

#[cfg(not(target_os = "linux"))]
pub fn watch(_limit: usize, _on_update: impl FnMut(Vec<UpcomingEvent>) -> bool) {}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_calendar_store::Event as IcalEvent;

    fn event(uid: &str, start: &str, end: &str) -> IcalEvent {
        IcalEvent {
            uid: uid.into(),
            summary: uid.into(),
            start: rmac_calendar_store::TimeValue {
                local: start.parse::<DateTime<Utc>>().unwrap().naive_utc(),
                zone: rmac_calendar_store::Zone::Utc,
            },
            end: rmac_calendar_store::TimeValue {
                local: end.parse::<DateTime<Utc>>().unwrap().naive_utc(),
                zone: rmac_calendar_store::Zone::Utc,
            },
            rrules: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            recurrence_id: None,
            cancelled: false,
            other_properties: Vec::new(),
        }
    }

    #[test]
    fn next_events_sorts_across_calendars_and_truncates() {
        let now: DateTime<Utc> = "2026-10-10T09:00:00Z".parse().unwrap();
        let calendars = vec![
            (
                "work".to_owned(),
                IcalCalendar {
                    events: vec![event("b", "2026-10-10T14:00:00Z", "2026-10-10T15:00:00Z")],
                },
            ),
            (
                "home".to_owned(),
                IcalCalendar {
                    events: vec![
                        event("a", "2026-10-10T10:00:00Z", "2026-10-10T11:00:00Z"),
                        event("c", "2026-10-11T09:00:00Z", "2026-10-11T10:00:00Z"),
                    ],
                },
            ),
        ];
        let next = next_events(&calendars, now, 2);
        assert_eq!(next.len(), 2);
        assert_eq!(next[0].event_uid, "a");
        assert_eq!(next[0].calendar_uid, "home");
        assert_eq!(next[1].event_uid, "b");
    }

    #[test]
    fn next_events_skips_events_that_already_ended() {
        let now: DateTime<Utc> = "2026-10-10T09:00:00Z".parse().unwrap();
        let calendars = vec![(
            "work".to_owned(),
            IcalCalendar {
                events: vec![event(
                    "past",
                    "2026-10-10T07:00:00Z",
                    "2026-10-10T08:00:00Z",
                )],
            },
        )];
        assert!(next_events(&calendars, now, 10).is_empty());
    }
}
