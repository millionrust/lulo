use std::io;
use std::path::{Path, PathBuf};

use rmac_appearance::{AccentColor, Snapshot as HostSnapshot};
use rmac_storage::{Backend, Failure, FileSystem};

use crate::model::{StoredPreferences, CURRENT_VERSION, DEFAULT_ACCENT};
use crate::{
    AccentPreference, Error, Operation, Preferences, Snapshot, StoreEvent, ThemeStore, ThemeWatcher,
};

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

pub(crate) fn validate_preferences(preferences: &Preferences, path: &Path) -> Result<(), Error> {
    if let AccentPreference::Custom([red, green, blue]) = preferences.accent_color {
        if AccentColor::new(red, green, blue).is_none() {
            return Err(invalid_accent(path));
        }
    }
    Ok(())
}

pub(crate) fn invalid_accent(path: &Path) -> Error {
    Failure::message(
        Operation::ValidatePreferences,
        path,
        "custom accent components must be finite numbers between 0 and 1",
    )
}

pub(crate) fn default_accent() -> AccentColor {
    AccentColor::new(DEFAULT_ACCENT.0, DEFAULT_ACCENT.1, DEFAULT_ACCENT.2)
        .expect("default accent is valid")
}

pub(crate) fn event_targets_path(paths: &[PathBuf], target: &Path) -> bool {
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
