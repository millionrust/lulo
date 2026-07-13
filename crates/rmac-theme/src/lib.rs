//! Writable rmac theme preferences and effective appearance resolution.
//!
//! The XDG Settings portal is read-only. This store is the separate rmac-owned
//! authority for user choices, persisted as a versioned document with an
//! atomically replaced last-known-good sibling.

use rmac_appearance::{
    AccentColor, ColorScheme, Contrast, MotionPreference, ResolvedAppearance, ResolvedColorScheme,
    Snapshot as HostSnapshot, TextScale,
};
use rmac_storage::{Backend, Failure, FileSystem};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

const CURRENT_VERSION: u32 = 1;
const DEFAULT_ACCENT: (f64, f64, f64) = (0.0, 0.478_431_372_5, 1.0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchemePreference {
    #[default]
    Automatic,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "rgb", rename_all = "kebab-case")]
pub enum AccentPreference {
    #[default]
    Automatic,
    Custom([f64; 3]),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContrastPreference {
    #[default]
    Automatic,
    Normal,
    Higher,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MotionPreferenceSetting {
    #[default]
    Automatic,
    Full,
    Reduced,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextScalePreference {
    #[default]
    Standard,
    Large,
    ExtraLarge,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub color_scheme: SchemePreference,
    pub accent_color: AccentPreference,
    pub contrast: ContrastPreference,
    pub motion: MotionPreferenceSetting,
    pub text_scale: TextScalePreference,
}

impl Preferences {
    pub fn resolve(&self, host: &HostSnapshot) -> Result<ResolvedAppearance, Error> {
        validate_preferences(self, Path::new("theme.json"))?;
        let color_scheme = match self.color_scheme {
            SchemePreference::Automatic => match host.color_scheme {
                ColorScheme::PreferDark => ResolvedColorScheme::Dark,
                ColorScheme::NoPreference | ColorScheme::PreferLight => ResolvedColorScheme::Light,
            },
            SchemePreference::Light => ResolvedColorScheme::Light,
            SchemePreference::Dark => ResolvedColorScheme::Dark,
        };
        let accent_color = match self.accent_color {
            AccentPreference::Automatic => host.accent_color.unwrap_or_else(default_accent),
            AccentPreference::Custom([red, green, blue]) => AccentColor::new(red, green, blue)
                .ok_or_else(|| invalid_accent(Path::new("theme.json")))?,
        };
        let contrast = match self.contrast {
            ContrastPreference::Automatic => host.contrast,
            ContrastPreference::Normal => Contrast::Normal,
            ContrastPreference::Higher => Contrast::Higher,
        };
        let motion = match self.motion {
            MotionPreferenceSetting::Automatic => host.motion,
            MotionPreferenceSetting::Full => MotionPreference::Full,
            MotionPreferenceSetting::Reduced => MotionPreference::Reduced,
        };
        let text_scale = match self.text_scale {
            TextScalePreference::Standard => TextScale::Standard,
            TextScalePreference::Large => TextScale::Large,
            TextScalePreference::ExtraLarge => TextScale::ExtraLarge,
        };
        Ok(ResolvedAppearance {
            color_scheme,
            accent_color,
            contrast,
            motion,
            text_scale,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub preferences: Preferences,
    pub effective: ResolvedAppearance,
    pub path: PathBuf,
    pub recovered_from_last_good: bool,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    ResolvePath,
    CreateDirectory,
    ReadPreferences,
    WatchPreferences,
    ParsePreferences,
    ValidatePreferences,
    SerializePreferences,
    SaveLastGood,
    SavePreferences,
    RestoreLastGood,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResolvePath => "resolve the theme preference path",
            Self::CreateDirectory => "create the theme preference directory",
            Self::ReadPreferences => "read theme preferences",
            Self::WatchPreferences => "watch theme preferences",
            Self::ParsePreferences => "parse theme preferences",
            Self::ValidatePreferences => "validate theme preferences",
            Self::SerializePreferences => "serialize theme preferences",
            Self::SaveLastGood => "save the last-known-good theme preferences",
            Self::SavePreferences => "save theme preferences",
            Self::RestoreLastGood => "restore the previous last-known-good theme preferences",
        })
    }
}

pub type Error = Failure<Operation>;

#[derive(Debug, Serialize, Deserialize)]
struct StoredPreferences {
    version: u32,
    #[serde(default)]
    preferences: Preferences,
}

pub struct ThemeStore<B = FileSystem> {
    path: PathBuf,
    backend: B,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StoreEvent {
    Changed,
    WatchError(Error),
}

/// Keeps the platform watcher alive and exposes coalescible store events.
pub struct ThemeWatcher {
    events: async_channel::Receiver<StoreEvent>,
    _watcher: notify::RecommendedWatcher,
}

impl ThemeWatcher {
    pub async fn recv(&self) -> Result<StoreEvent, async_channel::RecvError> {
        self.events.recv().await
    }

    pub fn try_recv(&self) -> Result<StoreEvent, async_channel::TryRecvError> {
        self.events.try_recv()
    }
}

impl ThemeStore<FileSystem> {
    pub fn from_environment() -> Result<Self, Error> {
        Ok(Self::new(theme_path()?))
    }

    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            backend: FileSystem,
        }
    }

    /// Watch the preference file without polling. Multiple low-level events
    /// coalesce into one pending notification until the consumer refreshes.
    pub fn watch(&self) -> Result<ThemeWatcher, Error> {
        use notify::Watcher as _;

        let parent = self.path.parent().ok_or_else(|| {
            Failure::message(
                Operation::ResolvePath,
                &self.path,
                "preference path has no parent directory",
            )
        })?;
        std::fs::create_dir_all(parent)
            .map_err(|error| Failure::from_io(Operation::CreateDirectory, parent, error))?;
        let watched_path = self.path.clone();
        let (events_tx, events_rx) = async_channel::bounded(1);
        let callback_path = watched_path.clone();
        let callback_tx = events_tx.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let event = match result {
                    Ok(event) if event_targets_path(&event.paths, &callback_path) => {
                        Some(StoreEvent::Changed)
                    }
                    Ok(_) => None,
                    Err(error) => Some(StoreEvent::WatchError(Failure::message(
                        Operation::WatchPreferences,
                        &callback_path,
                        error.to_string(),
                    ))),
                };
                if let Some(event) = event {
                    let _ = callback_tx.try_send(event);
                }
            })
            .map_err(|error| {
                Failure::message(
                    Operation::WatchPreferences,
                    &watched_path,
                    error.to_string(),
                )
            })?;
        watcher
            .watch(parent, notify::RecursiveMode::NonRecursive)
            .map_err(|error| {
                Failure::message(
                    Operation::WatchPreferences,
                    &watched_path,
                    error.to_string(),
                )
            })?;
        Ok(ThemeWatcher {
            events: events_rx,
            _watcher: watcher,
        })
    }
}

impl<B: Backend> ThemeStore<B> {
    pub fn with_backend(path: PathBuf, backend: B) -> Self {
        Self { path, backend }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self, host: &HostSnapshot) -> Result<Snapshot, Error> {
        match self.read_preferences(&self.path) {
            Ok(Some(preferences)) => self.snapshot(preferences, host, false, None),
            Ok(None) => match self.read_preferences(&self.last_good_path()) {
                Ok(Some(preferences)) => self.snapshot(
                    preferences,
                    host,
                    true,
                    Some("Recovered theme preferences from the last-known-good copy.".into()),
                ),
                Ok(None) => self.snapshot(Preferences::default(), host, false, None),
                Err(error) => Err(error),
            },
            Err(primary_error) => match self.read_preferences(&self.last_good_path()) {
                Ok(Some(preferences)) => self.snapshot(
                    preferences,
                    host,
                    true,
                    Some(format!(
                        "Recovered theme preferences after the primary file failed: {}",
                        primary_error.detail
                    )),
                ),
                Ok(None) => Err(primary_error),
                Err(backup_error) => Err(Failure::message(
                    Operation::ParsePreferences,
                    &self.path,
                    format!(
                        "primary failed ({}); last-known-good copy failed ({})",
                        primary_error.detail, backup_error.detail
                    ),
                )),
            },
        }
    }

    pub fn save(&self, preferences: &Preferences, host: &HostSnapshot) -> Result<Snapshot, Error> {
        validate_preferences(preferences, &self.path)?;
        let parent = self.path.parent().ok_or_else(|| {
            Failure::message(
                Operation::ResolvePath,
                &self.path,
                "preference path has no parent directory",
            )
        })?;
        self.backend
            .create_dir_all(parent)
            .map_err(|error| Failure::from_io(Operation::CreateDirectory, parent, error))?;
        let stored = StoredPreferences {
            version: CURRENT_VERSION,
            preferences: preferences.clone(),
        };
        let contents = serde_json::to_vec_pretty(&stored).map_err(|error| {
            Failure::message(
                Operation::SerializePreferences,
                &self.path,
                error.to_string(),
            )
        })?;
        let backup_path = self.last_good_path();
        let previous_backup = match self.backend.read(&backup_path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(Failure::from_io(
                    Operation::ReadPreferences,
                    &backup_path,
                    error,
                ));
            }
        };
        self.backend
            .write_atomic(&backup_path, &contents)
            .map_err(|error| Failure::from_io(Operation::SaveLastGood, &backup_path, error))?;
        if let Err(save_error) = self.backend.write_atomic(&self.path, &contents) {
            let rollback = match previous_backup {
                Some(previous) => self.backend.write_atomic(&backup_path, &previous),
                None => match self.backend.remove_file(&backup_path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error),
                },
            };
            let detail = match rollback {
                Ok(()) => save_error.to_string(),
                Err(rollback_error) => format!(
                    "{}; restoring the previous last-known-good copy also failed: {}",
                    save_error, rollback_error
                ),
            };
            return Err(Failure::message_with_kind(
                Operation::SavePreferences,
                &self.path,
                save_error.kind(),
                detail,
            ));
        }
        self.snapshot(preferences.clone(), host, false, None)
    }

    fn snapshot(
        &self,
        preferences: Preferences,
        host: &HostSnapshot,
        recovered_from_last_good: bool,
        detail: Option<String>,
    ) -> Result<Snapshot, Error> {
        let effective = preferences.resolve(host).map_err(|error| {
            Failure::message(Operation::ValidatePreferences, &self.path, error.detail)
        })?;
        Ok(Snapshot {
            preferences,
            effective,
            path: self.path.clone(),
            recovered_from_last_good,
            detail,
        })
    }

    fn read_preferences(&self, path: &Path) -> Result<Option<Preferences>, Error> {
        let contents = match self.backend.read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(Failure::from_io(Operation::ReadPreferences, path, error));
            }
        };
        let stored: StoredPreferences = serde_json::from_str(&contents).map_err(|error| {
            Failure::message(Operation::ParsePreferences, path, error.to_string())
        })?;
        if stored.version != CURRENT_VERSION {
            return Err(Failure::message(
                Operation::ParsePreferences,
                path,
                format!(
                    "unsupported theme preference version {}; expected {}",
                    stored.version, CURRENT_VERSION
                ),
            ));
        }
        validate_preferences(&stored.preferences, path)?;
        Ok(Some(stored.preferences))
    }

    fn last_good_path(&self) -> PathBuf {
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("theme.json");
        self.path.with_file_name(format!("{name}.last-good"))
    }
}

fn validate_preferences(preferences: &Preferences, path: &Path) -> Result<(), Error> {
    if let AccentPreference::Custom([red, green, blue]) = preferences.accent_color {
        if AccentColor::new(red, green, blue).is_none() {
            return Err(invalid_accent(path));
        }
    }
    Ok(())
}

fn invalid_accent(path: &Path) -> Error {
    Failure::message(
        Operation::ValidatePreferences,
        path,
        "custom accent components must be finite numbers between 0 and 1",
    )
}

fn default_accent() -> AccentColor {
    AccentColor::new(DEFAULT_ACCENT.0, DEFAULT_ACCENT.1, DEFAULT_ACCENT.2)
        .expect("default accent is valid")
}

fn event_targets_path(paths: &[PathBuf], target: &Path) -> bool {
    paths.iter().any(|path| path == target)
}

fn theme_path() -> Result<PathBuf, Error> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        Failure::message(
            Operation::ResolvePath,
            Path::new("theme.json"),
            "HOME is not set",
        )
    })?;
    #[cfg(target_os = "macos")]
    let path = home.join("Library/Application Support/rmac/theme.json");
    #[cfg(not(target_os = "macos"))]
    let path = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(xdg) if xdg.is_absolute() => xdg.join("rmac/theme.json"),
        _ => home.join(".config/rmac/theme.json"),
    };
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_store(name: &str) -> (PathBuf, ThemeStore) {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rmac-theme-{name}-{}-{sequence}",
            std::process::id()
        ));
        (root.clone(), ThemeStore::new(root.join("theme.json")))
    }

    fn host() -> HostSnapshot {
        HostSnapshot {
            available: true,
            color_scheme: ColorScheme::PreferDark,
            accent_color: AccentColor::new(0.8, 0.2, 0.4),
            contrast: Contrast::Higher,
            motion: MotionPreference::Reduced,
            ..HostSnapshot::default()
        }
    }

    #[test]
    fn automatic_preferences_follow_host_values() {
        let resolved = Preferences::default().resolve(&host()).unwrap();
        assert_eq!(resolved.color_scheme, ResolvedColorScheme::Dark);
        assert_eq!(resolved.accent_color.components(), (0.8, 0.2, 0.4));
        assert_eq!(resolved.contrast, Contrast::Higher);
        assert_eq!(resolved.motion, MotionPreference::Reduced);
        assert_eq!(resolved.text_scale, TextScale::Standard);
    }

    #[test]
    fn explicit_preferences_override_host_values() {
        let preferences = Preferences {
            color_scheme: SchemePreference::Light,
            accent_color: AccentPreference::Custom([0.1, 0.2, 0.3]),
            contrast: ContrastPreference::Normal,
            motion: MotionPreferenceSetting::Full,
            text_scale: TextScalePreference::ExtraLarge,
        };
        let resolved = preferences.resolve(&host()).unwrap();
        assert_eq!(resolved.color_scheme, ResolvedColorScheme::Light);
        assert_eq!(resolved.accent_color.components(), (0.1, 0.2, 0.3));
        assert_eq!(resolved.contrast, Contrast::Normal);
        assert_eq!(resolved.motion, MotionPreference::Full);
        assert_eq!(resolved.text_scale, TextScale::ExtraLarge);
    }

    #[test]
    fn save_round_trips_versioned_preferences() {
        let (root, store) = test_store("round-trip");
        let preferences = Preferences {
            color_scheme: SchemePreference::Dark,
            accent_color: AccentPreference::Custom([0.2, 0.4, 0.6]),
            contrast: ContrastPreference::Higher,
            motion: MotionPreferenceSetting::Reduced,
            text_scale: TextScalePreference::Large,
        };
        store.save(&preferences, &host()).unwrap();
        let loaded = store.load(&host()).unwrap();
        assert_eq!(loaded.preferences, preferences);
        assert!(!loaded.recovered_from_last_good);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn version_one_documents_without_text_scale_keep_standard_size() {
        let stored: StoredPreferences =
            serde_json::from_str(r#"{"version":1,"preferences":{"color_scheme":"dark"}}"#).unwrap();
        assert_eq!(stored.preferences.text_scale, TextScalePreference::Standard);
    }

    #[test]
    fn corrupt_primary_recovers_last_known_good_preferences() {
        let (root, store) = test_store("recovery");
        let preferences = Preferences {
            color_scheme: SchemePreference::Dark,
            ..Preferences::default()
        };
        store.save(&preferences, &host()).unwrap();
        std::fs::write(store.path(), b"not json").unwrap();
        let loaded = store.load(&host()).unwrap();
        assert_eq!(loaded.preferences, preferences);
        assert!(loaded.recovered_from_last_good);
        assert!(loaded.detail.unwrap().contains("primary file failed"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_accent_without_touching_the_store() {
        let (root, store) = test_store("invalid");
        let preferences = Preferences {
            accent_color: AccentPreference::Custom([0.0, f64::NAN, 1.0]),
            ..Preferences::default()
        };
        let error = store.save(&preferences, &host()).unwrap_err();
        assert_eq!(error.operation, Operation::ValidatePreferences);
        assert!(!store.path().exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_future_fields_are_tolerated_for_forward_compatibility() {
        let (root, store) = test_store("unknown-field");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            store.path(),
            r#"{"version":1,"preferences":{"color_scheme":"dark","future":true}}"#,
        )
        .unwrap();
        let loaded = store.load(&host()).unwrap();
        assert_eq!(loaded.preferences.color_scheme, SchemePreference::Dark);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn watcher_filter_ignores_sibling_files_and_accepts_atomic_rename_targets() {
        let target = PathBuf::from("/tmp/rmac/theme.json");
        assert!(!event_targets_path(
            &[PathBuf::from("/tmp/rmac/theme.json.last-good")],
            &target
        ));
        assert!(event_targets_path(
            &[PathBuf::from("/tmp/rmac/.theme.json.tmp"), target.clone()],
            &target
        ));
    }
}
