//! Saved places (`~/.config/rmac/weather.json`) and the forecast cache
//! (`~/.cache/rmac/weather/<place>.json`) that keeps the last forecast
//! readable offline.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::geocode::Place;

const SETTINGS_FILE: &str = "rmac/weather.json";
const CACHE_DIRECTORY: &str = "rmac/weather";
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
pub const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PLACES: usize = 20;
/// A forecast younger than this is shown without refetching.
pub const FRESH_SECONDS: i64 = 15 * 60;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    pub places: Vec<Place>,
    pub selected: usize,
    /// `None` follows the locale.
    pub fahrenheit: Option<bool>,
}

impl Settings {
    pub fn normalized(mut self) -> Self {
        self.places.retain(Place::is_valid);
        self.places.truncate(MAX_PLACES);
        for place in &mut self.places {
            place.name = place.name.chars().take(80).collect();
            place.region = place.region.chars().take(120).collect();
        }
        if self.selected >= self.places.len() {
            self.selected = 0;
        }
        self
    }

    /// Add (or re-select) a place and make it current.
    pub fn add(&mut self, place: Place) {
        if let Some(index) = self
            .places
            .iter()
            .position(|saved| saved.key() == place.key())
        {
            self.selected = index;
            return;
        }
        if self.places.len() >= MAX_PLACES {
            self.places.remove(0);
        }
        self.places.push(place);
        self.selected = self.places.len() - 1;
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.places.len() {
            self.places.remove(index);
            if self.selected > index || self.selected >= self.places.len() {
                self.selected = self.selected.saturating_sub(1);
            }
        }
    }

    pub fn current(&self) -> Option<&Place> {
        self.places.get(self.selected)
    }
}

fn config_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

fn cache_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
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

/// A cached response and when it was fetched (Unix seconds).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Cached {
    pub fetched_at: i64,
    pub body: String,
}

pub fn cache_path(place: &Place) -> Option<PathBuf> {
    cache_root().map(|root| {
        root.join(CACHE_DIRECTORY)
            .join(format!("{}.json", place.key()))
    })
}

pub fn read_cache_from(path: &Path) -> Option<Cached> {
    let bytes = rmac_storage::read_bounded_no_follow(path, MAX_CACHE_BYTES).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn write_cache_to(path: &Path, cached: &Cached) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let bytes = serde_json::to_vec(cached)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

fn no_home() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no home directory")
}

/// "Updated just now", "Updated 5 min ago", "Updated 3 hr ago",
/// "Updated 2 days ago".
pub fn age_text(fetched_at: i64, now: i64) -> String {
    let age = (now - fetched_at).max(0);
    match age {
        0..=59 => "Updated just now".to_owned(),
        60..=3_599 => format!("Updated {} min ago", age / 60),
        3_600..=86_399 => format!("Updated {} hr ago", age / 3600),
        _ => {
            let days = age / 86_400;
            format!("Updated {days} day{} ago", if days == 1 { "" } else { "s" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str, latitude: f64) -> Place {
        Place {
            name: name.into(),
            region: String::new(),
            latitude,
            longitude: 10.0,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("rmac-weather-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn adding_selects_and_deduplicates() {
        let mut settings = Settings::default();
        settings.add(place("A", 1.0));
        settings.add(place("B", 2.0));
        assert_eq!(settings.selected, 1);
        settings.add(place("A again", 1.001));
        assert_eq!(settings.places.len(), 2);
        assert_eq!(settings.selected, 0);
        settings.remove(0);
        assert_eq!(settings.current().unwrap().name, "B");
        settings.remove(0);
        assert!(settings.current().is_none());
        assert_eq!(settings.selected, 0);
    }

    #[test]
    fn normalization_drops_bad_places() {
        let settings = Settings {
            places: vec![place("", 1.0), place("Ok", 1.0), place("Bad", 95.0)],
            selected: 9,
            fahrenheit: None,
        }
        .normalized();
        assert_eq!(settings.places.len(), 1);
        assert_eq!(settings.selected, 0);
    }

    #[test]
    fn settings_and_cache_round_trip() {
        let directory = scratch("roundtrip");
        let path = directory.join("weather.json");
        assert_eq!(load_settings_from(&path).unwrap(), Settings::default());
        let mut settings = Settings::default();
        settings.add(place("Oslo", 59.9));
        settings.fahrenheit = Some(false);
        save_settings_to(&path, &settings).unwrap();
        assert_eq!(load_settings_from(&path).unwrap(), settings);
        let cache = directory.join("cache/x.json");
        assert!(read_cache_from(&cache).is_none());
        let cached = Cached {
            fetched_at: 5,
            body: "{}".into(),
        };
        write_cache_to(&cache, &cached).unwrap();
        assert_eq!(read_cache_from(&cache), Some(cached));
        std::fs::write(&path, b"garbage").unwrap();
        assert!(load_settings_from(&path).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn ages() {
        assert_eq!(age_text(100, 130), "Updated just now");
        assert_eq!(age_text(0, 300), "Updated 5 min ago");
        assert_eq!(age_text(0, 3 * 3600 + 5), "Updated 3 hr ago");
        assert_eq!(age_text(0, 86_400), "Updated 1 day ago");
        assert_eq!(age_text(0, 3 * 86_400), "Updated 3 days ago");
        assert_eq!(age_text(500, 100), "Updated just now");
    }
}
