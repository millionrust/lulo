//! Calendar's own saved state (CAL-6): which calendars are hidden or
//! soft-deleted, locally-created calendars and ICS subscriptions (until
//! CAL-2 wires a real EDS adapter calendars come from nowhere else), and
//! General settings. `~/.config/lulo/calendar.json`, the file ADR 0022 §9
//! names for Calendar's own UI state.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Calendar, CalendarColor};

const SETTINGS_FILE: &str = "lulo/calendar.json";
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
const MAX_SAVED_CALENDARS: usize = 64;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    /// Locally-created calendars and ICS subscriptions, in creation order.
    /// The fixture's seed calendars (Work, Team, Family, Home, Gym,
    /// Holidays) are not saved here; CAL-2 replaces them with EDS sources.
    pub saved_calendars: Vec<SavedCalendar>,
    /// Names of calendars (seed or saved) the user unticked.
    pub hidden: Vec<String>,
    /// Names of calendars (seed or saved) the user deleted.
    pub removed: Vec<String>,
    pub general: GeneralSettings,
}

impl Settings {
    pub fn normalized(mut self) -> Self {
        self.saved_calendars.truncate(MAX_SAVED_CALENDARS);
        self.saved_calendars
            .retain(|calendar| !calendar.name.trim().is_empty());
        for calendar in &mut self.saved_calendars {
            calendar.name = calendar
                .name
                .chars()
                .take(crate::MAX_CALENDAR_NAME_LEN)
                .collect();
        }
        self.hidden.retain(|name| !name.trim().is_empty());
        self.hidden.sort();
        self.hidden.dedup();
        self.removed.retain(|name| !name.trim().is_empty());
        self.removed.sort();
        self.removed.dedup();
        self.general = self.general.normalized();
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SavedCalendar {
    pub name: String,
    pub account: String,
    pub color: CalendarColor,
    pub subscription_url: Option<String>,
}

impl SavedCalendar {
    pub fn from_calendar(calendar: &Calendar) -> Self {
        Self {
            name: calendar.name.clone(),
            account: calendar.account.clone(),
            color: calendar.color,
            subscription_url: calendar.subscription_url.clone(),
        }
    }

    pub fn to_calendar(&self, visible: bool) -> Calendar {
        Calendar {
            name: self.name.clone(),
            account: self.account.clone(),
            color: self.color,
            visible,
            removed: false,
            subscription_url: self.subscription_url.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct GeneralSettings {
    /// A calendar name; empty (or no longer present) falls back to the
    /// first calendar.
    pub default_calendar: String,
    pub start_of_week_sunday: bool,
    pub day_starts_hour: u32,
    pub day_ends_hour: u32,
    pub time_zone_support: bool,
    /// An IANA zone name (`chrono_tz::Tz`); empty means "follow the system".
    pub time_zone: String,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            default_calendar: String::new(),
            start_of_week_sunday: false,
            day_starts_hour: 8,
            day_ends_hour: 20,
            time_zone_support: false,
            time_zone: String::new(),
        }
    }
}

impl GeneralSettings {
    pub fn normalized(mut self) -> Self {
        self.day_starts_hour = self.day_starts_hour.min(23);
        if self.day_ends_hour <= self.day_starts_hour || self.day_ends_hour > 24 {
            self.day_ends_hour = (self.day_starts_hour + 1).min(24);
        }
        if self.time_zone_support && self.time_zone.parse::<chrono_tz::Tz>().is_err() {
            self.time_zone.clear();
            self.time_zone_support = false;
        }
        self.default_calendar = self
            .default_calendar
            .chars()
            .take(crate::MAX_CALENDAR_NAME_LEN)
            .collect();
        self
    }
}

fn config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

pub fn settings_path() -> Option<PathBuf> {
    config_root().map(|root| root.join(SETTINGS_FILE))
}

pub fn load_settings_from(path: &Path) -> io::Result<Settings> {
    match rmac_storage::read_bounded_no_follow(path, MAX_SETTINGS_BYTES) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes)
            .map(Settings::normalized)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(error),
    }
}

pub fn save_settings_to(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&settings.clone().normalized())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

pub fn load_settings() -> io::Result<Settings> {
    load_settings_from(&settings_path().ok_or_else(no_home)?)
}

pub fn save_settings(settings: &Settings) -> io::Result<()> {
    save_settings_to(&settings_path().ok_or_else(no_home)?, settings)
}

fn no_home() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no home directory")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("rmac-calendar-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn missing_file_loads_defaults() {
        let path = scratch("missing").join("calendar.json");
        assert_eq!(load_settings_from(&path).unwrap(), Settings::default());
    }

    #[test]
    fn settings_round_trip() {
        let directory = scratch("roundtrip");
        let path = directory.join("calendar.json");
        let mut settings = Settings::default();
        settings.saved_calendars.push(SavedCalendar {
            name: "Team Releases".into(),
            account: "Subscribed".into(),
            color: CalendarColor::Teal,
            subscription_url: Some("https://example.com/team.ics".into()),
        });
        settings.hidden.push("Gym".into());
        settings.general.start_of_week_sunday = true;
        save_settings_to(&path, &settings).unwrap();
        assert_eq!(load_settings_from(&path).unwrap(), settings.normalized());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn normalization_fixes_invalid_hours_and_unknown_time_zone() {
        let settings = Settings {
            general: GeneralSettings {
                day_starts_hour: 30,
                day_ends_hour: 5,
                time_zone_support: true,
                time_zone: "Not/AZone".into(),
                ..GeneralSettings::default()
            },
            ..Settings::default()
        }
        .normalized();
        assert_eq!(settings.general.day_starts_hour, 23);
        assert_eq!(settings.general.day_ends_hour, 24);
        assert!(!settings.general.time_zone_support);
        assert!(settings.general.time_zone.is_empty());
    }

    #[test]
    fn normalization_accepts_a_real_time_zone() {
        let settings = Settings {
            general: GeneralSettings {
                time_zone_support: true,
                time_zone: "Europe/London".into(),
                ..GeneralSettings::default()
            },
            ..Settings::default()
        }
        .normalized();
        assert!(settings.general.time_zone_support);
        assert_eq!(settings.general.time_zone, "Europe/London");
    }

    #[test]
    fn saved_calendar_round_trips_through_a_calendar() {
        let saved = SavedCalendar {
            name: "Home".into(),
            account: "On My Mac".into(),
            color: CalendarColor::Green,
            subscription_url: None,
        };
        let calendar = saved.to_calendar(true);
        assert_eq!(calendar.name, "Home");
        assert!(calendar.visible);
        assert!(!calendar.removed);
        assert_eq!(SavedCalendar::from_calendar(&calendar), saved);
    }
}
