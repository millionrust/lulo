//! Per-occurrence alert state, `~/.local/state/lulo/calendar/alerts.json`
//! (ADR 0022 §6/§9). Shared between ticks so a re-scan never re-shows an
//! alarm the person already saw, snoozed or closed, and so a missed alert
//! after suspend is shown exactly once.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "lulo/calendar/alerts.json";
const MAX_STATE_BYTES: usize = 512 * 1024;
const MAX_ENTRIES: usize = 2048;
/// Entries whose trigger is this much older than "now" are pruned on every
/// save, so the file cannot grow without bound across years of use.
const PRUNE_AGE_DAYS: i64 = 30;
/// An alarm whose effective trigger is older than this is dropped rather
/// than shown: the agent (or the machine) was off for a long time, this is
/// not "just woke up from an afternoon nap".
pub const MAX_MISSED_AGE: chrono::Duration = chrono::Duration::hours(24);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum AlarmStatus {
    /// Shown once; never resurfaces for this exact trigger.
    Shown,
    /// Snoozed to `AlarmRecord::snoozed_until`, which supersedes the
    /// natural trigger until it fires or the alarm is closed.
    Snoozed,
    /// Dismissed, or given up on as too stale to show; never resurfaces.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AlarmRecord {
    /// The alarm's own natural trigger time, kept only to prune old
    /// entries and to tell one alarm's repeats apart from another's.
    pub trigger_at: DateTime<Utc>,
    pub status: AlarmStatus,
    #[serde(default)]
    pub snoozed_until: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AlertState {
    pub alarms: BTreeMap<String, AlarmRecord>,
}

/// A stable key for one alarm instance: the calendar, the event (its UID
/// alone for a non-recurring event; EDS reuses recurring events' UID across
/// every instance), the specific occurrence, and the alarm's own natural
/// trigger (so two alarms on the same occurrence -- "15 minutes before" and
/// "1 day before" -- never collide).
pub fn key(
    calendar_uid: &str,
    event_uid: &str,
    occurrence_start: DateTime<Utc>,
    trigger_at: DateTime<Utc>,
) -> String {
    format!(
        "{calendar_uid}\u{0}{event_uid}\u{0}{}\u{0}{}",
        occurrence_start.timestamp(),
        trigger_at.timestamp()
    )
}

impl AlertState {
    pub fn prune(&mut self, now: DateTime<Utc>) {
        let cutoff = now - chrono::Duration::days(PRUNE_AGE_DAYS);
        self.alarms.retain(|_, record| record.trigger_at > cutoff);
        while self.alarms.len() > MAX_ENTRIES {
            let Some(oldest) = self
                .alarms
                .iter()
                .min_by_key(|(_, record)| record.trigger_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.alarms.remove(&oldest);
        }
    }
}

pub fn state_path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|root| root.join(STATE_FILE))
}

pub fn load_from(path: &Path) -> io::Result<AlertState> {
    match rmac_storage::read_bounded_no_follow(path, MAX_STATE_BYTES) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AlertState::default()),
        Err(error) => Err(error),
    }
}

pub fn load() -> io::Result<AlertState> {
    load_from(&state_path().ok_or_else(no_home)?)
}

pub fn save_to(path: &Path, state: &AlertState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

pub fn save(state: &AlertState) -> io::Result<()> {
    save_to(&state_path().ok_or_else(no_home)?, state)
}

fn no_home() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no state directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rmac-calendar-agent-state-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory.join("alerts.json")
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    #[test]
    fn missing_file_loads_defaults_and_round_trips() {
        let path = scratch("roundtrip");
        assert_eq!(load_from(&path).unwrap(), AlertState::default());
        let mut state = AlertState::default();
        state.alarms.insert(
            key("local", "a@local", at(1_000), at(900)),
            AlarmRecord {
                trigger_at: at(900),
                status: AlarmStatus::Snoozed,
                snoozed_until: Some(at(2_000)),
            },
        );
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path).unwrap(), state);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn keys_disambiguate_multiple_alarms_on_the_same_occurrence() {
        let a = key("cal", "event", at(1_000), at(900));
        let b = key("cal", "event", at(1_000), at(100));
        assert_ne!(a, b);
    }

    #[test]
    fn prune_drops_stale_entries_and_caps_the_total() {
        let mut state = AlertState::default();
        let now = at(100 * 24 * 3600);
        state.alarms.insert(
            "stale".into(),
            AlarmRecord {
                trigger_at: at(0),
                status: AlarmStatus::Shown,
                snoozed_until: None,
            },
        );
        state.alarms.insert(
            "fresh".into(),
            AlarmRecord {
                trigger_at: now,
                status: AlarmStatus::Shown,
                snoozed_until: None,
            },
        );
        state.prune(now);
        assert_eq!(state.alarms.len(), 1);
        assert!(state.alarms.contains_key("fresh"));

        let mut state = AlertState::default();
        for index in 0..(MAX_ENTRIES + 10) {
            state.alarms.insert(
                format!("entry-{index}"),
                AlarmRecord {
                    trigger_at: now + chrono::Duration::seconds(index as i64),
                    status: AlarmStatus::Shown,
                    snoozed_until: None,
                },
            );
        }
        state.prune(now);
        assert_eq!(state.alarms.len(), MAX_ENTRIES);
    }

    #[test]
    fn corrupt_state_is_an_error_not_a_reset() {
        let path = scratch("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load_from(&path).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
