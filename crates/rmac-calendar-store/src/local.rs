//! A calendar kept in plain files, for the platform that has no Evolution Data
//! Server (Windows, ADR 0023 phase 2f).
//!
//! `root/calendars.json` lists the calendars (a stable uid and a name each) and
//! `root/<uid>.ics` holds one calendar's events as iCalendar, so the data is
//! readable by any other calendar program. Every write replaces the file
//! atomically and privately. There is always at least one calendar: the first
//! call to [`LocalStore::sources`] makes "Calendar".
//!
//! Editing follows the same scopes Calendar already offers EDS (`this`,
//! `this-and-future`, `all`). A change to a detached occurrence (an event that
//! carries a RECURRENCE-ID) touches only that occurrence. A change to a
//! series' master applies to the whole series whatever the scope says, because
//! the editor does not tell the store which occurrence was picked.

use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Calendar, Event};

const INDEX: &str = "calendars.json";
const MAX_INDEX_BYTES: usize = 64 * 1024;
const MAX_CALENDAR_BYTES: usize = 8 * 1024 * 1024;
/// The calendar every store starts with.
pub const DEFAULT_UID: &str = "local-calendar";
pub const DEFAULT_NAME: &str = "Calendar";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Source {
    pub uid: String,
    pub name: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Index {
    calendars: Vec<Entry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Entry {
    uid: String,
    name: String,
}

pub struct LocalStore {
    root: PathBuf,
}

fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// A uid may name a file, so it is limited to what a file name can hold.
fn checked(uid: &str) -> io::Result<&str> {
    let ok = !uid.is_empty()
        && uid.len() <= 64
        && uid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if ok {
        Ok(uid)
    } else {
        Err(invalid("invalid calendar id"))
    }
}

impl LocalStore {
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `<data home>/lulo/calendar/local`, with the data home the XDG
    /// variable (which Lulo apps point at `%APPDATA%\Lulo\Data` on Windows).
    pub fn default_root() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|root| root.join("lulo/calendar/local"))
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(INDEX)
    }

    fn calendar_path(&self, uid: &str) -> io::Result<PathBuf> {
        Ok(self.root.join(format!("{}.ics", checked(uid)?)))
    }

    fn load_index(&self) -> io::Result<Index> {
        match rmac_storage::read_bounded_no_follow(&self.index_path(), MAX_INDEX_BYTES) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(invalid),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Index::default()),
            Err(error) => Err(error),
        }
    }

    fn save_index(&self, index: &Index) -> io::Result<()> {
        rmac_storage::create_dir_all_private(&self.root)?;
        let bytes = serde_json::to_vec_pretty(index).map_err(invalid)?;
        rmac_storage::atomic_write_private(&self.index_path(), &bytes)
    }

    /// The calendars, making the default one the first time.
    pub fn sources(&self) -> io::Result<Vec<Source>> {
        let mut index = self.load_index()?;
        if index.calendars.is_empty() {
            index.calendars.push(Entry {
                uid: DEFAULT_UID.into(),
                name: DEFAULT_NAME.into(),
            });
            self.save_index(&index)?;
        }
        Ok(index
            .calendars
            .into_iter()
            .filter(|entry| checked(&entry.uid).is_ok())
            .map(|entry| Source {
                uid: entry.uid,
                name: entry.name,
            })
            .collect())
    }

    /// One calendar's events; a calendar with no file yet is empty.
    pub fn read(&self, uid: &str) -> io::Result<Calendar> {
        match rmac_storage::read_bounded_no_follow(&self.calendar_path(uid)?, MAX_CALENDAR_BYTES) {
            Ok(bytes) => {
                let text = String::from_utf8(bytes).map_err(invalid)?;
                Calendar::parse(&text).map_err(invalid)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(Calendar { events: Vec::new() })
            }
            Err(error) => Err(error),
        }
    }

    fn write(&self, uid: &str, calendar: &Calendar) -> io::Result<()> {
        rmac_storage::create_dir_all_private(&self.root)?;
        rmac_storage::atomic_write_private(&self.calendar_path(uid)?, calendar.to_ical().as_bytes())
    }

    fn known(&self, uid: &str) -> io::Result<()> {
        if self.sources()?.iter().any(|source| source.uid == uid) {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "no such calendar"))
        }
    }

    pub fn create(&self, uid: &str, event: &Event) -> io::Result<()> {
        self.known(uid)?;
        let mut calendar = self.read(uid)?;
        if calendar
            .events
            .iter()
            .any(|existing| same_instance(existing, event))
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "that event already exists",
            ));
        }
        calendar.events.push(event.clone());
        self.write(uid, &calendar)
    }

    /// Replace the event `after` names. See the module note on scopes.
    pub fn modify(&self, uid: &str, after: &Event, _scope: &str) -> io::Result<()> {
        self.known(uid)?;
        let mut calendar = self.read(uid)?;
        let slot = calendar
            .events
            .iter_mut()
            .find(|existing| same_instance(existing, after))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such event"))?;
        *slot = after.clone();
        self.write(uid, &calendar)
    }

    /// Remove an event. A detached occurrence goes alone; a series' master
    /// goes with every occurrence detached from it.
    pub fn remove(&self, uid: &str, event: &Event, _scope: &str) -> io::Result<()> {
        self.known(uid)?;
        let mut calendar = self.read(uid)?;
        let before = calendar.events.len();
        if event.recurrence_id.is_some() {
            calendar
                .events
                .retain(|existing| !same_instance(existing, event));
        } else {
            calendar.events.retain(|existing| existing.uid != event.uid);
        }
        if calendar.events.len() == before {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such event"));
        }
        self.write(uid, &calendar)
    }
}

/// The same master, or the same detached occurrence of one.
fn same_instance(left: &Event, right: &Event) -> bool {
    left.uid == right.uid && left.recurrence_id == right.recurrence_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TimeValue, Zone};
    use chrono::NaiveDate;

    fn event(uid: &str, summary: &str, day: u32) -> Event {
        let at = |hour| TimeValue {
            local: NaiveDate::from_ymd_opt(2026, 10, day)
                .unwrap()
                .and_hms_opt(hour, 0, 0)
                .unwrap(),
            zone: Zone::Utc,
        };
        Event {
            uid: uid.into(),
            summary: summary.into(),
            start: at(9),
            end: at(10),
            rrules: Vec::new(),
            rdates: Vec::new(),
            exdates: Vec::new(),
            recurrence_id: None,
            cancelled: false,
            other_properties: vec![
                "LOCATION:Room 4".into(),
                "BEGIN:VALARM".into(),
                "TRIGGER:-PT15M".into(),
                "ACTION:DISPLAY".into(),
                "END:VALARM".into(),
            ],
        }
    }

    fn store(label: &str) -> (LocalStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("lulo-cal-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (LocalStore::open(&dir), dir)
    }

    #[test]
    fn a_new_store_has_one_empty_calendar() {
        let (store, dir) = store("fresh");
        let sources = store.sources().unwrap();
        assert_eq!(
            sources,
            vec![Source {
                uid: DEFAULT_UID.into(),
                name: DEFAULT_NAME.into()
            }]
        );
        assert!(store.read(DEFAULT_UID).unwrap().events.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn events_round_trip_through_the_file_with_their_alarms() {
        let (store, dir) = store("roundtrip");
        store.sources().unwrap();
        let lunch = event("a@local", "Lunch", 12);
        store.create(DEFAULT_UID, &lunch).unwrap();
        assert_eq!(store.read(DEFAULT_UID).unwrap().events, vec![lunch.clone()]);
        let mut moved = lunch.clone();
        moved.summary = "Late lunch".into();
        store.modify(DEFAULT_UID, &moved, "all").unwrap();
        assert_eq!(store.read(DEFAULT_UID).unwrap().events, vec![moved.clone()]);
        assert!(store.create(DEFAULT_UID, &moved).is_err());
        store.remove(DEFAULT_UID, &moved, "all").unwrap();
        assert!(store.read(DEFAULT_UID).unwrap().events.is_empty());
        assert!(store.remove(DEFAULT_UID, &moved, "all").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deleting_a_series_takes_its_detached_occurrences_and_deleting_one_does_not() {
        let (store, dir) = store("series");
        store.sources().unwrap();
        let mut master = event("s@local", "Standup", 5);
        master.rrules = vec!["FREQ=DAILY".into()];
        let mut detached = event("s@local", "Standup (moved)", 6);
        detached.recurrence_id = Some(master.start);
        store.create(DEFAULT_UID, &master).unwrap();
        store.create(DEFAULT_UID, &detached).unwrap();
        store.remove(DEFAULT_UID, &detached, "this").unwrap();
        assert_eq!(store.read(DEFAULT_UID).unwrap().events.len(), 1);
        store.create(DEFAULT_UID, &detached).unwrap();
        store.remove(DEFAULT_UID, &master, "all").unwrap();
        assert!(store.read(DEFAULT_UID).unwrap().events.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_calendars_and_unsafe_ids_are_refused() {
        let (store, dir) = store("refuse");
        store.sources().unwrap();
        let e = event("x@local", "X", 3);
        assert!(store.create("nope", &e).is_err());
        assert!(store.create("../evil", &e).is_err());
        assert!(store.read("a/b").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
