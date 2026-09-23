//! Durable settings storage, recovery, and watch authority.

use super::*;

pub struct ShellSettingsStore<B = FileSystem> {
    path: PathBuf,
    backend: B,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryState {
    Current,
    LastGoodAvailable,
    DefaultsOnly,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StoreEvent {
    Changed,
    WatchError(Error),
}

pub struct ShellSettingsWatcher {
    events: async_channel::Receiver<StoreEvent>,
    _watcher: notify::RecommendedWatcher,
}

impl ShellSettingsWatcher {
    pub async fn recv(&self) -> Result<StoreEvent, async_channel::RecvError> {
        self.events.recv().await
    }

    pub fn try_recv(&self) -> Result<StoreEvent, async_channel::TryRecvError> {
        self.events.try_recv()
    }
}

impl ShellSettingsStore<FileSystem> {
    pub fn from_environment() -> Result<Self, Error> {
        Ok(Self::new(shell_settings_path()?))
    }

    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            backend: FileSystem,
        }
    }

    pub fn watch(&self) -> Result<ShellSettingsWatcher, Error> {
        use notify::Watcher as _;

        let parent = parent_path(&self.path)?;
        std::fs::create_dir_all(parent)
            .map_err(|error| Failure::from_io(Operation::CreateDirectory, parent, error))?;
        let watched_path = self.path.clone();
        let callback_path = watched_path.clone();
        let (sender, receiver) = async_channel::bounded(1);
        let callback_sender = sender.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let event = match result {
                    Ok(event)
                        if !event.kind.is_access()
                            && event_targets_path(&event.paths, &callback_path) =>
                    {
                        Some(StoreEvent::Changed)
                    }
                    Ok(_) => None,
                    Err(error) => Some(StoreEvent::WatchError(Failure::message(
                        Operation::WatchSettings,
                        &callback_path,
                        error.to_string(),
                    ))),
                };
                if let Some(event) = event {
                    let _ = callback_sender.try_send(event);
                }
            })
            .map_err(|error| {
                Failure::message(Operation::WatchSettings, &watched_path, error.to_string())
            })?;
        watcher
            .watch(parent, notify::RecursiveMode::NonRecursive)
            .map_err(|error| {
                Failure::message(Operation::WatchSettings, &watched_path, error.to_string())
            })?;
        Ok(ShellSettingsWatcher {
            events: receiver,
            _watcher: watcher,
        })
    }
}

impl<B: Backend> ShellSettingsStore<B> {
    pub fn with_backend(path: PathBuf, backend: B) -> Self {
        Self { path, backend }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Snapshot, Error> {
        match self.read_settings(&self.path) {
            Ok(Some(loaded)) => self.finish_load(loaded, false, None),
            Ok(None) => match self.read_settings(&self.last_good_path()) {
                Ok(Some(loaded)) => self.finish_load(
                    loaded,
                    true,
                    Some("Recovered shell settings from the last-known-good copy.".into()),
                ),
                Ok(None) => self.snapshot(ShellSettings::default(), false, None, None),
                Err(error) => Err(error),
            },
            Err(primary_error) => match self.read_settings(&self.last_good_path()) {
                Ok(Some(loaded)) => self.finish_load(
                    loaded,
                    true,
                    Some(format!(
                        "Recovered shell settings after the primary file failed: {}",
                        primary_error.detail
                    )),
                ),
                Ok(None) => Err(primary_error),
                Err(backup_error) => Err(Failure::message(
                    Operation::ParseSettings,
                    &self.path,
                    format!(
                        "primary failed ({}); last-known-good copy failed ({})",
                        primary_error.detail, backup_error.detail
                    ),
                )),
            },
        }
    }

    pub fn save(&self, settings: &ShellSettings) -> Result<Snapshot, Error> {
        validate(settings, &self.path)?;
        self.persist(settings)?;
        self.snapshot(settings.clone(), false, None, None)
    }

    pub fn recovery_state(&self) -> RecoveryState {
        let primary = self.config_state(&self.path);
        if primary == ConfigState::Valid {
            return RecoveryState::Current;
        }
        let last_good = self.config_state(&self.last_good_path());
        if last_good == ConfigState::Valid {
            RecoveryState::LastGoodAvailable
        } else if primary == ConfigState::Missing && last_good == ConfigState::Missing {
            RecoveryState::DefaultsOnly
        } else {
            RecoveryState::Unavailable
        }
    }

    pub fn restore_last_good(&self) -> Result<Snapshot, Error> {
        if self.config_state(&self.path) == ConfigState::Valid {
            return Err(Failure::message_with_kind(
                Operation::RestoreLastGood,
                &self.path,
                io::ErrorKind::AlreadyExists,
                "the primary shell settings are already valid",
            ));
        }

        let backup_path = self.last_good_path();
        let loaded = self.read_settings(&backup_path)?.ok_or_else(|| {
            Failure::message_with_kind(
                Operation::RestoreLastGood,
                &backup_path,
                io::ErrorKind::NotFound,
                "no last-known-good shell settings are available",
            )
        })?;
        validate(&loaded.settings, &backup_path)?;
        let contents = serialize_settings(&loaded.settings, &self.path)?;
        let parent = parent_path(&self.path)?;
        self.backend
            .create_dir_all(parent)
            .map_err(|error| Failure::from_io(Operation::CreateDirectory, parent, error))?;

        match self
            .backend
            .read_bounded_no_follow(&self.path, MAX_RECOVERY_COPY_BYTES)
        {
            Ok(rejected) => {
                let rejected_path = self.rejected_path();
                self.backend
                    .write_atomic_private(&rejected_path, &rejected)
                    .map_err(|error| {
                        Failure::from_io(Operation::RestoreLastGood, &rejected_path, error)
                    })?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Failure::from_io(
                    Operation::RestoreLastGood,
                    &self.path,
                    error,
                ));
            }
        }

        self.backend
            .write_atomic(&self.path, &contents)
            .map_err(|error| Failure::from_io(Operation::RestoreLastGood, &self.path, error))?;
        self.snapshot(
            loaded.settings,
            false,
            loaded.migrated_from,
            Some("Restored shell settings from the last-known-good copy.".into()),
        )
    }

    fn finish_load(
        &self,
        loaded: Loaded,
        recovered: bool,
        detail: Option<String>,
    ) -> Result<Snapshot, Error> {
        validate(&loaded.settings, &self.path)?;
        if loaded.migrated_from.is_some() {
            self.persist(&loaded.settings).map_err(|error| {
                Failure::message_with_kind(
                    Operation::MigrateSettings,
                    &self.path,
                    error.error_kind,
                    error.detail,
                )
            })?;
        }
        self.snapshot(loaded.settings, recovered, loaded.migrated_from, detail)
    }

    fn snapshot(
        &self,
        settings: ShellSettings,
        recovered_from_last_good: bool,
        migrated_from: Option<u32>,
        detail: Option<String>,
    ) -> Result<Snapshot, Error> {
        validate(&settings, &self.path)?;
        Ok(Snapshot {
            settings,
            path: self.path.clone(),
            recovered_from_last_good,
            migrated_from,
            detail,
        })
    }

    fn read_settings(&self, path: &Path) -> Result<Option<Loaded>, Error> {
        let contents = match self.backend.read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Failure::from_io(Operation::ReadSettings, path, error)),
        };
        let envelope: serde_json::Value = serde_json::from_str(&contents)
            .map_err(|error| Failure::message(Operation::ParseSettings, path, error.to_string()))?;
        let version = envelope
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                Failure::message(Operation::ParseSettings, path, "missing numeric version")
            })?;
        match version {
            1 => {
                let legacy: LegacySettings = serde_json::from_value(envelope).map_err(|error| {
                    Failure::message(Operation::ParseSettings, path, error.to_string())
                })?;
                let _ = legacy.version;
                Ok(Some(Loaded {
                    settings: migrate_v1(legacy),
                    migrated_from: Some(1),
                }))
            }
            2 => {
                let stored: StoredSettings = serde_json::from_value(envelope).map_err(|error| {
                    Failure::message(Operation::ParseSettings, path, error.to_string())
                })?;
                let settings = migrate_application_ids(stored.settings);
                validate(&settings, path)?;
                Ok(Some(Loaded {
                    settings,
                    migrated_from: Some(2),
                }))
            }
            3 => {
                let stored: StoredSettings = serde_json::from_value(envelope).map_err(|error| {
                    Failure::message(Operation::ParseSettings, path, error.to_string())
                })?;
                let settings = migrate_application_ids(stored.settings);
                validate(&settings, path)?;
                Ok(Some(Loaded {
                    settings,
                    migrated_from: Some(3),
                }))
            }
            version if version == u64::from(CURRENT_VERSION) => {
                let stored: StoredSettings = serde_json::from_value(envelope).map_err(|error| {
                    Failure::message(Operation::ParseSettings, path, error.to_string())
                })?;
                debug_assert_eq!(stored.version, CURRENT_VERSION);
                validate(&stored.settings, path)?;
                Ok(Some(Loaded {
                    settings: stored.settings,
                    migrated_from: None,
                }))
            }
            other => Err(Failure::message(
                Operation::ParseSettings,
                path,
                format!("unsupported shell settings version {other}; expected 1 through 4"),
            )),
        }
    }

    fn persist(&self, settings: &ShellSettings) -> Result<(), Error> {
        let parent = parent_path(&self.path)?;
        self.backend
            .create_dir_all(parent)
            .map_err(|error| Failure::from_io(Operation::CreateDirectory, parent, error))?;
        let contents = serialize_settings(settings, &self.path)?;
        let backup_path = self.last_good_path();
        let previous_backup = match self.backend.read(&backup_path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(Failure::from_io(
                    Operation::ReadSettings,
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
                Operation::SaveSettings,
                &self.path,
                save_error.kind(),
                detail,
            ));
        }
        Ok(())
    }

    fn last_good_path(&self) -> PathBuf {
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shell.json");
        self.path.with_file_name(format!("{name}.last-good"))
    }

    fn rejected_path(&self) -> PathBuf {
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shell.json");
        self.path
            .with_file_name(format!("{name}.rejected-before-restore"))
    }

    fn config_state(&self, path: &Path) -> ConfigState {
        match self.read_settings(path) {
            Ok(Some(loaded)) if validate(&loaded.settings, path).is_ok() => ConfigState::Valid,
            Ok(None) => ConfigState::Missing,
            Ok(Some(_)) | Err(_) => ConfigState::Invalid,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ConfigState {
    Missing,
    Valid,
    Invalid,
}
