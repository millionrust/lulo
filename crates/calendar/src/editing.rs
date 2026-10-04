//! Event persistence and undo commands. Call `load` and `apply` only on workers.

use crate::WeekSnapshot;
#[cfg(target_os = "linux")]
use crate::{Calendar, CalendarColor, Event};
#[cfg(target_os = "linux")]
use chrono::Datelike;
use chrono::{DateTime, Duration, NaiveDate, Utc};
#[cfg(target_os = "linux")]
use rmac_calendar_store::expand;
#[cfg(any(target_os = "linux", test))]
use rmac_calendar_store::Calendar as IcalCalendar;
use rmac_calendar_store::{Event as IcalEvent, TimeValue, Zone};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_UID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub enum Mutation {
    Create {
        source: String,
        event: IcalEvent,
    },
    Modify {
        source: String,
        before: IcalEvent,
        after: IcalEvent,
        scope: &'static str,
    },
    Delete {
        source: String,
        event: IcalEvent,
        scope: &'static str,
    },
    MoveSource {
        from: String,
        to: String,
        before: IcalEvent,
        after: IcalEvent,
    },
}

impl Mutation {
    pub fn inverse(&self) -> Self {
        match self {
            Self::Create { source, event } => Self::Delete {
                source: source.clone(),
                event: event.clone(),
                scope: "all",
            },
            Self::Modify {
                source,
                before,
                after,
                scope,
            } => Self::Modify {
                source: source.clone(),
                before: after.clone(),
                after: before.clone(),
                scope,
            },
            Self::Delete { source, event, .. } => Self::Create {
                source: source.clone(),
                event: event.clone(),
            },
            Self::MoveSource {
                from,
                to,
                before,
                after,
            } => Self::MoveSource {
                from: to.clone(),
                to: from.clone(),
                before: after.clone(),
                after: before.clone(),
            },
        }
    }
}

pub fn new_event(start: DateTime<Utc>, end: DateTime<Utc>, all_day: bool) -> IcalEvent {
    let uid = format!(
        "lulo-{}-{}@local",
        Utc::now().timestamp_micros(),
        NEXT_UID.fetch_add(1, Ordering::Relaxed)
    );
    let zone = if all_day { Zone::Date } else { Zone::Utc };
    IcalEvent {
        uid,
        summary: "New Event".into(),
        start: TimeValue {
            local: start.naive_utc(),
            zone,
        },
        end: TimeValue {
            local: end.naive_utc(),
            zone,
        },
        rrules: Vec::new(),
        rdates: Vec::new(),
        exdates: Vec::new(),
        recurrence_id: None,
        cancelled: false,
        other_properties: Vec::new(),
    }
}

pub fn property(event: &IcalEvent, name: &str) -> String {
    event
        .other_properties
        .iter()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key == name).then(|| value.replace("\\n", "\n").replace("\\,", ","))
        })
        .unwrap_or_default()
}

pub fn set_property(event: &mut IcalEvent, name: &str, value: &str) {
    event
        .other_properties
        .retain(|line| !line.starts_with(&format!("{name}:")));
    if !value.trim().is_empty() {
        let escaped = value
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace(',', "\\,");
        event.other_properties.push(format!("{name}:{escaped}"));
    }
}

#[cfg(any(target_os = "linux", test))]
fn object(event: &IcalEvent) -> String {
    let wire = IcalCalendar {
        events: vec![event.clone()],
    }
    .to_ical();
    let begin = wire.find("BEGIN:VEVENT").expect("serialiser emits VEVENT");
    let end = wire.find("END:VEVENT").expect("serialiser closes VEVENT") + "END:VEVENT".len();
    format!("{}\r\n", &wire[begin..end])
}

#[cfg(target_os = "linux")]
pub fn apply(change: &Mutation) -> Result<(), String> {
    use rmac_calendar_eds::Eds;
    let source = match change {
        Mutation::Create { source, .. }
        | Mutation::Modify { source, .. }
        | Mutation::Delete { source, .. } => source,
        Mutation::MoveSource { from, .. } => from,
    };
    let eds = Eds::session().map_err(|_| "Calendar service unavailable".to_owned())?;
    let calendar = eds
        .open(source)
        .map_err(|_| "Couldn't open calendar".to_owned())?;
    if !calendar
        .writable()
        .map_err(|_| "Couldn't check calendar permissions".to_owned())?
    {
        return Err("This calendar is read-only".into());
    }
    match change {
        Mutation::Create { event, .. } => {
            calendar
                .create(&[object(event)])
                .map_err(|_| "Couldn't create event".to_owned())?;
        }
        Mutation::Modify { after, scope, .. } => {
            calendar
                .modify(&[object(after)], scope)
                .map_err(|_| "Couldn't change event".to_owned())?;
        }
        Mutation::Delete { event, scope, .. } => {
            let rid = event
                .recurrence_id
                .map(|date| match date.zone {
                    Zone::Date => date.local.format("%Y%m%d").to_string(),
                    _ => date.local.format("%Y%m%dT%H%M%SZ").to_string(),
                })
                .unwrap_or_default();
            calendar
                .remove(&[(event.uid.clone(), rid)], scope)
                .map_err(|_| "Couldn't delete event".to_owned())?;
        }
        Mutation::MoveSource {
            to, before, after, ..
        } => {
            let target = eds
                .open(to)
                .map_err(|_| "Couldn't open destination calendar".to_owned())?;
            if !target.writable().unwrap_or(false) {
                return Err("Destination calendar is read-only".into());
            }
            target
                .create(&[object(after)])
                .map_err(|_| "Couldn't move event".to_owned())?;
            if calendar
                .remove(&[(before.uid.clone(), String::new())], "all")
                .is_err()
            {
                let _ = target.remove(&[(after.uid.clone(), String::new())], "all");
                return Err("Couldn't move event".into());
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn apply(_change: &Mutation) -> Result<(), String> {
    Err("Calendar service unavailable".into())
}

#[cfg(target_os = "linux")]
pub fn load() -> Result<WeekSnapshot, String> {
    use rmac_calendar_eds::Eds;
    let eds = Eds::session().map_err(|_| "Calendar service unavailable".to_owned())?;
    eds.check_available()
        .map_err(|_| "Calendar service unavailable".to_owned())?;
    let mut snapshot = WeekSnapshot::empty();
    let today = Utc::now().date_naive();
    let from = today
        .checked_sub_signed(Duration::days(366 * 2))
        .ok_or("Invalid date")?;
    let to = today
        .checked_add_signed(Duration::days(366 * 3))
        .ok_or("Invalid date")?;
    for source in eds
        .sources()
        .map_err(|_| "Couldn't load calendars".to_owned())?
        .into_iter()
        .filter(|source| source.enabled)
    {
        let client = match eds.open(&source.uid) {
            Ok(client) => client,
            Err(_) => continue,
        };
        let writable = client.writable().unwrap_or(false);
        let calendar_index = snapshot.calendars.len();
        snapshot.calendars.push(Calendar {
            name: source.display_name.clone(),
            account: if source.backend == "local" {
                "On My Computer".into()
            } else {
                source.backend.clone()
            },
            color: match calendar_index % 6 {
                0 => CalendarColor::Blue,
                1 => CalendarColor::Teal,
                2 => CalendarColor::Orange,
                3 => CalendarColor::Green,
                4 => CalendarColor::Purple,
                _ => CalendarColor::Red,
            },
            visible: true,
            id: source.uid.clone(),
            source_uid: Some(source.uid),
            writable,
            removed: false,
            subscription_url: None,
        });
        let mut ical = IcalCalendar { events: Vec::new() };
        for raw in client
            .object_list("#t")
            .map_err(|_| "Couldn't load events".to_owned())?
        {
            let wrapped = if raw.contains("BEGIN:VCALENDAR") {
                raw
            } else {
                format!("BEGIN:VCALENDAR\nVERSION:2.0\n{raw}\nEND:VCALENDAR")
            };
            let parsed =
                IcalCalendar::parse(&wrapped).map_err(|_| "Couldn't read an event".to_owned())?;
            ical.events.extend(parsed.events);
        }
        let start = from.and_hms_opt(0, 0, 0).ok_or("Invalid date")?.and_utc();
        let end = to.and_hms_opt(0, 0, 0).ok_or("Invalid date")?.and_utc();
        for occurrence in expand(&ical, start, end, chrono_tz::UTC, 10_000)
            .map_err(|_| "Couldn't expand repeating events".to_owned())?
        {
            let Some(master) = ical
                .events
                .iter()
                .find(|event| event.uid == occurrence.uid && event.recurrence_id.is_none())
            else {
                continue;
            };
            let source_event = ical
                .events
                .iter()
                .find(|event| {
                    event.uid == occurrence.uid
                        && event
                            .recurrence_id
                            .and_then(|id| id.resolve(chrono_tz::UTC).ok())
                            == Some(occurrence.recurrence_id)
                })
                .unwrap_or(master);
            snapshot.events.push(Event {
                id: format!(
                    "{}#{}#{}",
                    calendar_index,
                    occurrence.uid,
                    occurrence.recurrence_id.timestamp()
                ),
                title: occurrence.summary,
                location: property(source_event, "LOCATION"),
                calendar: calendar_index,
                start: occurrence.start,
                end: occurrence.end,
                all_day: occurrence.all_day,
                ical: Some(source_event.clone()),
            });
        }
    }
    snapshot.slots =
        snapshot.slots_for(today - Duration::days(today.weekday().num_days_from_monday() as i64));
    Ok(snapshot)
}

#[cfg(not(target_os = "linux"))]
pub fn load() -> Result<WeekSnapshot, String> {
    Err("Calendar service unavailable".into())
}

pub fn at_day(date: NaiveDate, hour: u32) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let start = date.and_hms_opt(hour, 0, 0)?.and_utc();
    Some((start, start + Duration::hours(1)))
}

/// A fake calendar store standing in for the EDS adapter. `editing::apply` writes
/// to the real `rmac_calendar_eds::Eds` service on Linux; this fake lets the
/// undo/redo contract that `CalendarView::submit`/`undo`/`redo` rely on be
/// exercised in CI without a D-Bus session (CAL-5: "writes through the EDS
/// adapter, with fakes in tests").
#[cfg(test)]
struct FakeCalendar {
    events: Vec<IcalEvent>,
}

#[cfg(test)]
impl FakeCalendar {
    fn new() -> Self {
        Self { events: Vec::new() }
    }

    fn apply(&mut self, change: &Mutation) -> Result<(), String> {
        match change {
            Mutation::Create { event, .. } => {
                self.events.push(event.clone());
                Ok(())
            }
            Mutation::Modify { before, after, .. } => {
                let slot = self
                    .events
                    .iter_mut()
                    .find(|existing| existing.uid == before.uid)
                    .ok_or("event not found")?;
                *slot = after.clone();
                Ok(())
            }
            Mutation::Delete { event, .. } => {
                let before = self.events.len();
                self.events.retain(|existing| existing.uid != event.uid);
                if self.events.len() == before {
                    return Err("event not found".into());
                }
                Ok(())
            }
            Mutation::MoveSource { .. } => Err("move not exercised by this fake".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulates `CalendarView`'s own undo stack (push the inverse on every
    /// submit, pop-and-replay on undo, push back onto redo) against the fake
    /// store, proving create -> modify -> delete -> undo x3 returns to the
    /// starting state and redo x3 replays it forward again.
    #[test]
    fn undo_redo_stack_round_trips_through_a_fake_calendar() {
        // Mirrors `CalendarView::submit`'s three-way distinction: undoing moves a
        // change onto redo, redoing moves it back onto undo without disturbing the
        // rest of either stack, and only a genuinely new edit clears redo.
        #[derive(Clone, Copy)]
        enum Direction {
            Do,
            Undo,
            Redo,
        }
        fn submit(
            store: &mut FakeCalendar,
            undo: &mut Vec<Mutation>,
            redo: &mut Vec<Mutation>,
            change: Mutation,
            direction: Direction,
        ) {
            store.apply(&change).unwrap();
            match direction {
                Direction::Undo => redo.push(change.inverse()),
                Direction::Redo => undo.push(change.inverse()),
                Direction::Do => {
                    undo.push(change.inverse());
                    redo.clear();
                }
            }
        }

        let mut store = FakeCalendar::new();
        let mut undo: Vec<Mutation> = Vec::new();
        let mut redo: Vec<Mutation> = Vec::new();
        let source = "local".to_string();

        let (start, end) = at_day(NaiveDate::from_ymd_opt(2026, 10, 3).unwrap(), 10).unwrap();
        let created = new_event(start, end, false);

        submit(
            &mut store,
            &mut undo,
            &mut redo,
            Mutation::Create {
                source: source.clone(),
                event: created.clone(),
            },
            Direction::Do,
        );
        assert_eq!(store.events.len(), 1);

        let mut renamed = created.clone();
        renamed.summary = "Renamed".into();
        submit(
            &mut store,
            &mut undo,
            &mut redo,
            Mutation::Modify {
                source: source.clone(),
                before: created.clone(),
                after: renamed.clone(),
                scope: "all",
            },
            Direction::Do,
        );
        assert_eq!(store.events[0].summary, "Renamed");

        submit(
            &mut store,
            &mut undo,
            &mut redo,
            Mutation::Delete {
                source: source.clone(),
                event: renamed.clone(),
                scope: "all",
            },
            Direction::Do,
        );
        assert!(store.events.is_empty());
        assert_eq!(undo.len(), 3);

        // Undo the delete, the rename, then the create: back to empty.
        while let Some(change) = undo.pop() {
            submit(&mut store, &mut undo, &mut redo, change, Direction::Undo);
        }
        assert!(store.events.is_empty());
        assert_eq!(redo.len(), 3);

        // Redo everything: ends up renamed-and-deleted again.
        while let Some(change) = redo.pop() {
            submit(&mut store, &mut undo, &mut redo, change, Direction::Redo);
        }
        assert!(store.events.is_empty());
        assert_eq!(undo.len(), 3);
    }

    #[test]
    fn undo_reverses_create_and_modify() {
        let (start, end) = at_day(NaiveDate::from_ymd_opt(2026, 10, 3).unwrap(), 10).unwrap();
        let first = new_event(start, end, false);
        let source = "local".to_string();
        let create = Mutation::Create {
            source: source.clone(),
            event: first.clone(),
        };
        assert!(
            matches!(create.inverse(), Mutation::Delete { event, .. } if event.uid == first.uid)
        );
        let mut changed = first.clone();
        changed.summary = "Updated".into();
        let edit = Mutation::Modify {
            source,
            before: first.clone(),
            after: changed,
            scope: "all",
        };
        assert!(matches!(edit.inverse(), Mutation::Modify { after, .. } if after == first));
    }

    #[test]
    fn object_round_trip_preserves_location_and_alert() {
        let (start, end) = at_day(NaiveDate::from_ymd_opt(2026, 10, 3).unwrap(), 10).unwrap();
        let mut event = new_event(start, end, false);
        set_property(&mut event, "LOCATION", "Room 4");
        event.other_properties.extend([
            "BEGIN:VALARM".into(),
            "TRIGGER:-PT15M".into(),
            "ACTION:DISPLAY".into(),
            "END:VALARM".into(),
        ]);
        let wrapped = format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\n{}\nEND:VCALENDAR",
            object(&event)
        );
        let parsed = IcalCalendar::parse(&wrapped).unwrap();
        assert_eq!(parsed.events[0], event);
    }
}
