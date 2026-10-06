//! Calendar's own saved state (CAL-6): per-calendar visibility, colour,
//! display name and soft-delete prefs keyed by the calendar's stable id
//! (`Calendar::id`, the EDS source uid for live calendars), locally-created
//! calendars and ICS subscriptions, and General settings.
//! `~/.config/lulo/calendar.json`, the file ADR 0022 §9 names for Calendar's
//! own UI state. [`overlay`] applies it on top of the EDS (or fixture)
//! calendar list every time a snapshot loads.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::editing::AlertOffset;
use crate::{Calendar, CalendarColor};

const SETTINGS_FILE: &str = "lulo/calendar.json";
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
const MAX_SAVED_CALENDARS: usize = 64;
const MAX_CALENDAR_PREFS: usize = 256;
const MAX_ID_LEN: usize = 256;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    /// Locally-created calendars and ICS subscriptions, in creation order.
    /// They are appended after the EDS (or fixture) calendars, so
    /// `Event::calendar` indices into the snapshot stay valid.
    pub saved_calendars: Vec<SavedCalendar>,
    /// Per-calendar overrides keyed by `Calendar::id`.
    pub calendars: BTreeMap<String, CalendarPrefs>,
    pub general: GeneralSettings,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct CalendarPrefs {
    /// `None` keeps the source's own default (the fixture's Gym starts hidden).
    pub visible: Option<bool>,
    pub color: Option<CalendarColor>,
    /// A Lulo-side display name; renaming doesn't write back to EDS yet.
    pub name: Option<String>,
    /// Removed from Lulo's calendar list. The EDS source itself is kept.
    pub removed: bool,
}

impl CalendarPrefs {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

fn bounded_name(name: &str) -> String {
    name.chars().take(crate::MAX_CALENDAR_NAME_LEN).collect()
}

impl Settings {
    pub fn normalized(mut self) -> Self {
        self.saved_calendars.truncate(MAX_SAVED_CALENDARS);
        self.saved_calendars.retain(|calendar| {
            !calendar.name.trim().is_empty() && calendar.id.chars().count() <= MAX_ID_LEN
        });
        let mut seen = std::collections::BTreeSet::new();
        for calendar in &mut self.saved_calendars {
            calendar.name = bounded_name(&calendar.name);
            if calendar.id.trim().is_empty() || !seen.insert(calendar.id.clone()) {
                calendar.id = String::new();
            }
        }
        // Give old or duplicate entries a fresh, unique local id.
        for index in 0..self.saved_calendars.len() {
            if self.saved_calendars[index].id.is_empty() {
                let id = next_local_id(&self.saved_calendars);
                self.saved_calendars[index].id = id;
            }
        }
        self.calendars.retain(|id, prefs| {
            !id.trim().is_empty() && id.chars().count() <= MAX_ID_LEN && !prefs.is_default()
        });
        for prefs in self.calendars.values_mut() {
            prefs.name = prefs
                .name
                .take()
                .map(|name| bounded_name(name.trim()))
                .filter(|name| !name.is_empty());
        }
        while self.calendars.len() > MAX_CALENDAR_PREFS {
            let Some(last) = self.calendars.keys().next_back().cloned() else {
                break;
            };
            self.calendars.remove(&last);
        }
        self.general = self.general.normalized();
        self
    }

    pub fn prefs_mut(&mut self, id: &str) -> &mut CalendarPrefs {
        self.calendars.entry(id.to_owned()).or_default()
    }

    /// Adds a local calendar or subscription with a fresh id and returns it.
    pub fn add_calendar(
        &mut self,
        name: String,
        account: &str,
        color: CalendarColor,
        subscription_url: Option<String>,
    ) -> String {
        let id = next_local_id(&self.saved_calendars);
        self.saved_calendars.push(SavedCalendar {
            id: id.clone(),
            name,
            account: account.to_owned(),
            color,
            subscription_url,
        });
        id
    }

    /// Deletes a calendar from Lulo's list: a local calendar or subscription
    /// is dropped outright; an EDS or fixture calendar is soft-deleted.
    pub fn remove_calendar(&mut self, id: &str) {
        let before = self.saved_calendars.len();
        self.saved_calendars.retain(|calendar| calendar.id != id);
        if self.saved_calendars.len() != before {
            self.calendars.remove(id);
        } else {
            self.prefs_mut(id).removed = true;
        }
    }
}

fn next_local_id(saved: &[SavedCalendar]) -> String {
    let mut index = saved.len() + 1;
    loop {
        let candidate = format!("local-{index}");
        if !saved.iter().any(|calendar| calendar.id == candidate) {
            return candidate;
        }
        index += 1;
    }
}

/// The rendered calendar list: `base` (EDS or fixture calendars, which
/// `Event::calendar` indexes) with the saved prefs applied, followed by the
/// saved local calendars and subscriptions.
pub fn overlay(base: &[Calendar], settings: &Settings) -> Vec<Calendar> {
    let mut calendars: Vec<Calendar> = base.to_vec();
    calendars.extend(
        settings
            .saved_calendars
            .iter()
            .map(SavedCalendar::to_calendar),
    );
    for calendar in &mut calendars {
        let Some(prefs) = settings.calendars.get(&calendar.id) else {
            continue;
        };
        if let Some(name) = &prefs.name {
            calendar.name = name.clone();
        }
        if let Some(color) = prefs.color {
            calendar.color = color;
        }
        if let Some(visible) = prefs.visible {
            calendar.visible = visible;
        }
        if prefs.removed {
            calendar.removed = true;
            calendar.visible = false;
        }
    }
    calendars
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SavedCalendar {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub account: String,
    pub color: CalendarColor,
    pub subscription_url: Option<String>,
}

impl SavedCalendar {
    pub fn to_calendar(&self) -> Calendar {
        Calendar {
            name: self.name.clone(),
            account: self.account.clone(),
            color: self.color,
            visible: true,
            source_uid: None,
            writable: false,
            id: self.id.clone(),
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
    pub default_alerts: DefaultAlerts,
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
            default_alerts: DefaultAlerts::default(),
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
        self.default_calendar = bounded_name(&self.default_calendar);
        self
    }
}

/// Calendar ▸ Settings ▸ Alerts' Default Alerts (CAL-7, ADR 0022 §6): the
/// alert new events get when the person does not pick one explicitly, as
/// the Mac applies at event creation rather than at alert time. Birthdays
/// has no row yet -- Lulo has no Birthdays calendar (`docs/parity.md`),
/// so a default for it would do nothing.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct DefaultAlerts {
    pub events: AlertOffset,
    pub all_day_events: AlertOffset,
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

/// UIA-04: whether event and "now" times should read in 24-hour time, the
/// same source the menu bar clock and System Settings ▸ Date & Time read
/// (`rmac_shell_settings::ClockFormat`, with the system locale's own hour
/// cycle deciding the `Locale` default -- `rmac-top-bar`'s
/// `top_bar_hour_cycle`/`clock_label` do the same combination). Falls back
/// to 12-hour, matching `ClockFormat`'s own `Locale` default, if either
/// read fails (no shell-settings file yet, or the locale service is
/// unavailable).
pub fn twenty_four_hour_preference() -> bool {
    let format = rmac_shell_settings::ShellSettingsStore::from_environment()
        .and_then(|store| store.load())
        .map(|snapshot| snapshot.settings.clock.format)
        .unwrap_or_default();
    match format {
        rmac_shell_settings::ClockFormat::TwentyFourHour => true,
        rmac_shell_settings::ClockFormat::TwelveHour => false,
        rmac_shell_settings::ClockFormat::Locale => matches!(
            rmac_locale_linux::hour_cycle(),
            Ok(rmac_locale::HourCycle::TwentyFourHour)
        ),
    }
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
        settings.add_calendar(
            "Team Releases".into(),
            "Subscribed",
            CalendarColor::Teal,
            Some("https://example.com/team.ics".into()),
        );
        settings.prefs_mut("fixture:Gym").visible = Some(true);
        settings.prefs_mut("eds-source-1").color = Some(CalendarColor::Red);
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
    fn overlay_applies_prefs_by_id_and_appends_local_calendars() {
        let mut base = crate::seed_calendars();
        base[0].id = "eds-work".into();
        base[0].source_uid = Some("eds-work".into());
        let mut settings = Settings::default();
        {
            let prefs = settings.prefs_mut("eds-work");
            prefs.visible = Some(false);
            prefs.color = Some(CalendarColor::Purple);
            prefs.name = Some("Office".into());
        }
        settings.prefs_mut("fixture:Gym").visible = Some(true);
        settings.remove_calendar("fixture:Holidays");
        let local = settings.add_calendar("Trips".into(), "On My Mac", CalendarColor::Green, None);
        let calendars = overlay(&base, &settings);
        assert_eq!(calendars.len(), base.len() + 1);
        assert_eq!(calendars[0].name, "Office");
        assert_eq!(calendars[0].color, CalendarColor::Purple);
        assert!(!calendars[0].visible);
        assert_eq!(calendars[0].source_uid.as_deref(), Some("eds-work"));
        assert!(calendars[4].visible, "Gym was ticked on");
        assert!(calendars[5].removed && !calendars[5].visible);
        assert_eq!(calendars[6].id, local);
        assert_eq!(calendars[6].name, "Trips");
        // Deleting a local calendar drops it entirely.
        settings.remove_calendar(&local);
        assert_eq!(overlay(&base, &settings).len(), base.len());
    }

    #[test]
    fn normalization_assigns_unique_local_ids_and_drops_default_prefs() {
        let saved = |id: &str| SavedCalendar {
            id: id.into(),
            name: "Home".into(),
            account: "On My Mac".into(),
            color: CalendarColor::Green,
            subscription_url: None,
        };
        let mut settings = Settings {
            saved_calendars: vec![saved(""), saved("local-2"), saved("local-2")],
            ..Settings::default()
        };
        settings.prefs_mut("fixture:Work");
        let settings = settings.normalized();
        let ids: Vec<_> = settings
            .saved_calendars
            .iter()
            .map(|calendar| calendar.id.as_str())
            .collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.iter().all(|id| !id.is_empty()));
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 3);
        assert!(settings.calendars.is_empty());
    }

    #[test]
    fn saved_calendar_becomes_a_visible_local_calendar() {
        let saved = SavedCalendar {
            id: "local-1".into(),
            name: "Home".into(),
            account: "On My Mac".into(),
            color: CalendarColor::Green,
            subscription_url: None,
        };
        let calendar = saved.to_calendar();
        assert_eq!(calendar.name, "Home");
        assert_eq!(calendar.id, "local-1");
        assert!(calendar.visible);
        assert!(!calendar.removed);
        assert!(!calendar.writable && calendar.source_uid.is_none());
    }
}
