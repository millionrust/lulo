//! Clock's saved state (`~/.config/rmac/clock.json`), shared by the window
//! and the ring process. Every change is a locked read-modify-write so the
//! two never lose each other's updates.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::alarms::Alarm;
use crate::countdown::Countdown;
use crate::stopwatch::Stopwatch;

const STATE_FILE: &str = "rmac/clock.json";
const MAX_STATE_BYTES: usize = 256 * 1024;
pub const MAX_ALARMS: usize = 64;
pub const MAX_TIMERS: usize = 16;
pub const MAX_CITIES: usize = 48;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct State {
    /// World Clock cities by name; `None` until the user edits the list,
    /// which shows the system zone's city.
    pub cities: Option<Vec<String>>,
    pub alarms: Vec<Alarm>,
    pub timers: Vec<Countdown>,
    pub stopwatch: Stopwatch,
    pub next_id: u64,
}

impl State {
    pub fn allocate_id(&mut self) -> u64 {
        self.next_id = self.next_id.max(
            self.alarms
                .iter()
                .map(|alarm| alarm.id)
                .chain(self.timers.iter().map(|timer| timer.id))
                .max()
                .unwrap_or(0),
        ) + 1;
        self.next_id
    }

    /// Keep lists within bounds after loading an edited file.
    pub fn normalized(mut self) -> Self {
        self.alarms.truncate(MAX_ALARMS);
        self.timers.truncate(MAX_TIMERS);
        if let Some(cities) = self.cities.as_mut() {
            cities.truncate(MAX_CITIES);
        }
        for alarm in &mut self.alarms {
            alarm.hour = alarm.hour.min(23);
            alarm.minute = alarm.minute.min(59);
            truncate_chars(&mut alarm.label, 64);
        }
        for timer in &mut self.timers {
            truncate_chars(&mut timer.label, 64);
        }
        self
    }
}

/// Cut `text` to at most `bytes` bytes on a character boundary.
pub fn truncate_chars(text: &mut String, bytes: usize) {
    if text.len() > bytes {
        let mut end = bytes;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

pub fn state_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|root| root.join(STATE_FILE))
}

/// Read the state; a missing file is the default state and a corrupt one is
/// an error (so it is never silently overwritten).
pub fn load_from(path: &Path) -> io::Result<State> {
    let bytes = match rmac_storage::read_bounded_no_follow(path, MAX_STATE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        Err(error) => return Err(error),
    };
    serde_json::from_slice::<State>(&bytes)
        .map(State::normalized)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn load() -> io::Result<State> {
    load_from(&state_path().ok_or_else(no_home)?)
}

/// Lock, re-read, change and write the state; returns the saved state.
pub fn update<R>(change: impl FnOnce(&mut State) -> R) -> io::Result<(State, R)> {
    update_at(&state_path().ok_or_else(no_home)?, change)
}

pub fn update_at<R>(path: &Path, change: impl FnOnce(&mut State) -> R) -> io::Result<(State, R)> {
    let parent = path.parent().ok_or_else(no_home)?;
    rmac_storage::create_dir_all_private(parent)?;
    let _lock = Lock::acquire(&path.with_extension("lock"))?;
    let mut state = load_from(path)?;
    let result = change(&mut state);
    let state = state.normalized();
    let bytes = serde_json::to_vec_pretty(&state)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)?;
    Ok((state, result))
}

fn no_home() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no configuration directory")
}

struct Lock(File);

impl Lock {
    fn acquire(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        file.lock()?;
        Ok(Self(file))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alarms::Days;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("rmac-clock-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory.join("clock.json")
    }

    #[test]
    fn missing_file_is_default_and_updates_persist() {
        let path = scratch("persist");
        assert_eq!(load_from(&path).unwrap(), State::default());
        let (state, id) = update_at(&path, |state| {
            let id = state.allocate_id();
            state.alarms.push(Alarm {
                id,
                repeat: Days::WEEKDAYS,
                ..Alarm::default()
            });
            state.cities = Some(vec!["Tokyo".into()]);
            id
        })
        .unwrap();
        assert_eq!(id, 1);
        assert_eq!(load_from(&path).unwrap(), state);
        let (_, second) = update_at(&path, State::allocate_id).unwrap();
        assert_eq!(second, 2);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_state_is_an_error_not_a_reset() {
        let path = scratch("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load_from(&path).is_err());
        assert!(update_at(&path, |_| ()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn normalization_bounds_hostile_values() {
        let state = State {
            alarms: vec![
                Alarm {
                    hour: 99,
                    minute: 99,
                    label: "x".repeat(500),
                    ..Alarm::default()
                };
                100
            ],
            ..State::default()
        }
        .normalized();
        assert_eq!(state.alarms.len(), MAX_ALARMS);
        assert_eq!((state.alarms[0].hour, state.alarms[0].minute), (23, 59));
        assert_eq!(state.alarms[0].label.len(), 64);
        let mut text = "é".repeat(40);
        truncate_chars(&mut text, 63);
        assert_eq!(text.len(), 62);
    }
}
