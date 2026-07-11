//! Durable, compositor-independent settings authority for rmac shell processes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_storage::{Backend, Failure, FileSystem};
use serde::{Deserialize, Serialize};

const CURRENT_VERSION: u32 = 3;
const MAX_PINNED_APPS: usize = 128;
const MAX_SPOTLIGHT_EXCLUSIONS: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AppId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProviderId(pub String);

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DockPlacement {
    Left,
    #[default]
    Bottom,
    Right,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "scope", content = "output", rename_all = "kebab-case")]
pub enum OutputScope {
    #[default]
    All,
    Primary,
    Named(String),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepeatedClickBehavior {
    #[default]
    CycleWindows,
    HideApplication,
    DoNothing,
}

fn default_magnification_scale() -> f32 {
    1.5
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct DockSettings {
    pub placement: DockPlacement,
    pub outputs: OutputScope,
    pub autohide: bool,
    pub magnification: bool,
    pub magnification_scale: f32,
    pub reserve_space: bool,
    pub repeated_click: RepeatedClickBehavior,
}

impl Default for DockSettings {
    fn default() -> Self {
        Self {
            placement: DockPlacement::Bottom,
            outputs: OutputScope::All,
            autohide: false,
            magnification: true,
            magnification_scale: default_magnification_scale(),
            reserve_space: true,
            repeated_click: RepeatedClickBehavior::CycleWindows,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockFormat {
    #[default]
    Locale,
    TwelveHour,
    TwentyFourHour,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ClockSettings {
    pub format: ClockFormat,
    pub show_date: bool,
    pub show_seconds: bool,
    pub show_workspace: bool,
}

impl Default for ClockSettings {
    fn default() -> Self {
        Self {
            format: ClockFormat::Locale,
            show_date: true,
            show_seconds: false,
            show_workspace: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct IndicatorSettings {
    pub network: bool,
    pub vpn: bool,
    pub bluetooth: bool,
    pub sound: bool,
    pub power: bool,
    pub battery_percentage: bool,
    pub notifications: bool,
    pub focus: bool,
}

impl Default for IndicatorSettings {
    fn default() -> Self {
        Self {
            network: true,
            vpn: true,
            bluetooth: true,
            sound: true,
            power: true,
            battery_percentage: false,
            notifications: true,
            focus: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WallpaperFit {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WallpaperSelection {
    pub source: Option<String>,
    pub fit: WallpaperFit,
}

impl Default for WallpaperSelection {
    fn default() -> Self {
        Self {
            source: None,
            fit: WallpaperFit::Fill,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WallpaperSettings {
    pub default: WallpaperSelection,
    pub per_output: BTreeMap<String, WallpaperSelection>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct FocusSettings {
    pub enabled: bool,
    pub selected_mode: Option<String>,
    pub ends_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ProviderPolicy {
    pub enabled: bool,
    pub allow_private_content: bool,
    pub allow_network: bool,
}

impl Default for ProviderPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_private_content: false,
            allow_network: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct SpotlightSettings {
    /// Absolute directory roots omitted from filename and recent-document
    /// results. Shell text and URI forms are deliberately unsupported.
    pub excluded_paths: Vec<String>,
    /// Search across filesystem device boundaries. Off by default so mounted
    /// removable media is never traversed merely by opening the launcher.
    pub include_removable_mounts: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ShellSettings {
    pub pinned_apps: Vec<AppId>,
    pub dock: DockSettings,
    pub clock: ClockSettings,
    pub indicators: IndicatorSettings,
    pub wallpaper: WallpaperSettings,
    pub focus: FocusSettings,
    pub providers: BTreeMap<ProviderId, ProviderPolicy>,
    pub spotlight: SpotlightSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub settings: ShellSettings,
    pub path: PathBuf,
    pub recovered_from_last_good: bool,
    pub migrated_from: Option<u32>,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolvePath,
    CreateDirectory,
    ReadSettings,
    WatchSettings,
    ParseSettings,
    MigrateSettings,
    ValidateSettings,
    SerializeSettings,
    SaveLastGood,
    SaveSettings,
    RestoreLastGood,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResolvePath => "resolve the shell settings path",
            Self::CreateDirectory => "create the shell settings directory",
            Self::ReadSettings => "read shell settings",
            Self::WatchSettings => "watch shell settings",
            Self::ParseSettings => "parse shell settings",
            Self::MigrateSettings => "migrate shell settings",
            Self::ValidateSettings => "validate shell settings",
            Self::SerializeSettings => "serialize shell settings",
            Self::SaveLastGood => "save last-known-good shell settings",
            Self::SaveSettings => "save shell settings",
            Self::RestoreLastGood => "restore last-known-good shell settings",
        })
    }
}

pub type Error = Failure<Operation>;

#[derive(Debug, Deserialize, Serialize)]
struct StoredSettings {
    version: u32,
    #[serde(default)]
    settings: ShellSettings,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct LegacyDockSettings {
    placement: DockPlacement,
    autohide: bool,
    magnification: bool,
}

impl Default for LegacyDockSettings {
    fn default() -> Self {
        Self {
            placement: DockPlacement::Bottom,
            autohide: false,
            magnification: true,
        }
    }
}

#[derive(Debug, Deserialize)]
struct LegacySettings {
    version: u32,
    #[serde(default)]
    pinned_apps: Vec<AppId>,
    #[serde(default)]
    dock: LegacyDockSettings,
    #[serde(default)]
    wallpaper: Option<String>,
    #[serde(default)]
    focus_mode: Option<String>,
}

struct Loaded {
    settings: ShellSettings,
    migrated_from: Option<u32>,
}

pub struct ShellSettingsStore<B = FileSystem> {
    path: PathBuf,
    backend: B,
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
                    Ok(event) if event_targets_path(&event.paths, &callback_path) => {
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
                validate(&stored.settings, path)?;
                Ok(Some(Loaded {
                    settings: stored.settings,
                    migrated_from: Some(2),
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
                format!("unsupported shell settings version {other}; expected 1, 2, or 3"),
            )),
        }
    }

    fn persist(&self, settings: &ShellSettings) -> Result<(), Error> {
        let parent = parent_path(&self.path)?;
        self.backend
            .create_dir_all(parent)
            .map_err(|error| Failure::from_io(Operation::CreateDirectory, parent, error))?;
        let contents = serde_json::to_vec_pretty(&StoredSettings {
            version: CURRENT_VERSION,
            settings: settings.clone(),
        })
        .map_err(|error| {
            Failure::message(Operation::SerializeSettings, &self.path, error.to_string())
        })?;
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
}

fn migrate_v1(legacy: LegacySettings) -> ShellSettings {
    ShellSettings {
        pinned_apps: legacy.pinned_apps,
        dock: DockSettings {
            placement: legacy.dock.placement,
            autohide: legacy.dock.autohide,
            magnification: legacy.dock.magnification,
            ..DockSettings::default()
        },
        wallpaper: WallpaperSettings {
            default: WallpaperSelection {
                source: legacy.wallpaper,
                ..WallpaperSelection::default()
            },
            ..WallpaperSettings::default()
        },
        focus: FocusSettings {
            selected_mode: legacy.focus_mode,
            ..FocusSettings::default()
        },
        ..ShellSettings::default()
    }
}

fn validate(settings: &ShellSettings, path: &Path) -> Result<(), Error> {
    if settings.pinned_apps.len() > MAX_PINNED_APPS {
        return Err(invalid(
            path,
            "pinned apps exceed the 128-item safety limit",
        ));
    }
    let mut pinned = BTreeSet::new();
    for app in &settings.pinned_apps {
        validate_identifier(&app.0, "pinned application", path)?;
        if !pinned.insert(&app.0) {
            return Err(invalid(path, "pinned applications must be unique"));
        }
    }
    if !(1.0..=2.5).contains(&settings.dock.magnification_scale)
        || !settings.dock.magnification_scale.is_finite()
    {
        return Err(invalid(
            path,
            "Dock magnification scale must be finite and between 1.0 and 2.5",
        ));
    }
    if let OutputScope::Named(output) = &settings.dock.outputs {
        validate_identifier(output, "Dock output", path)?;
    }
    validate_wallpaper(&settings.wallpaper.default, path)?;
    for (output, wallpaper) in &settings.wallpaper.per_output {
        validate_identifier(output, "wallpaper output", path)?;
        validate_wallpaper(wallpaper, path)?;
    }
    if let Some(mode) = &settings.focus.selected_mode {
        validate_identifier(mode, "Focus mode", path)?;
    }
    if settings.focus.enabled && settings.focus.selected_mode.is_none() {
        return Err(invalid(path, "enabled Focus requires a selected mode"));
    }
    if settings.focus.ends_at_unix_ms.is_some() && !settings.focus.enabled {
        return Err(invalid(
            path,
            "a disabled Focus mode cannot have an end time",
        ));
    }
    for provider in settings.providers.keys() {
        validate_identifier(&provider.0, "provider", path)?;
    }
    if settings.spotlight.excluded_paths.len() > MAX_SPOTLIGHT_EXCLUSIONS {
        return Err(invalid(
            path,
            "Spotlight exclusions exceed the 128-item safety limit",
        ));
    }
    let mut exclusions = BTreeSet::new();
    for exclusion in &settings.spotlight.excluded_paths {
        validate_identifier(exclusion, "Spotlight exclusion", path)?;
        let exclusion_path = Path::new(exclusion);
        if !exclusion_path.is_absolute()
            || exclusion_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(invalid(
                path,
                "Spotlight exclusions must be normalized absolute paths",
            ));
        }
        if !exclusions.insert(exclusion) {
            return Err(invalid(path, "Spotlight exclusions must be unique"));
        }
    }
    Ok(())
}

fn validate_wallpaper(selection: &WallpaperSelection, path: &Path) -> Result<(), Error> {
    if let Some(source) = &selection.source {
        validate_identifier(source, "wallpaper source", path)?;
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str, path: &Path) -> Result<(), Error> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(invalid(
            path,
            format!("{label} must be non-empty, bounded, and contain no control characters"),
        ));
    }
    Ok(())
}

fn invalid(path: &Path, detail: impl Into<String>) -> Error {
    Failure::message(Operation::ValidateSettings, path, detail)
}

fn parent_path(path: &Path) -> Result<&Path, Error> {
    path.parent().ok_or_else(|| {
        Failure::message(
            Operation::ResolvePath,
            path,
            "shell settings path has no parent directory",
        )
    })
}

fn event_targets_path(paths: &[PathBuf], target: &Path) -> bool {
    paths.iter().any(|path| path == target)
}

fn shell_settings_path() -> Result<PathBuf, Error> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        Failure::message(
            Operation::ResolvePath,
            Path::new("shell.json"),
            "HOME is not set",
        )
    })?;
    #[cfg(target_os = "macos")]
    let path = home.join("Library/Application Support/rmac/shell.json");
    #[cfg(not(target_os = "macos"))]
    let path = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(xdg) if xdg.is_absolute() => xdg.join("rmac/shell.json"),
        _ => home.join(".config/rmac/shell.json"),
    };
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_store(label: &str) -> (PathBuf, ShellSettingsStore) {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rmac-shell-settings-{label}-{}-{sequence}",
            std::process::id()
        ));
        (
            root.clone(),
            ShellSettingsStore::new(root.join("shell.json")),
        )
    }

    fn settings() -> ShellSettings {
        ShellSettings {
            pinned_apps: vec![
                AppId("org.rmac.Finder".into()),
                AppId("org.rmac.Terminal".into()),
            ],
            dock: DockSettings {
                placement: DockPlacement::Left,
                outputs: OutputScope::Named("DP-1".into()),
                autohide: true,
                magnification: true,
                magnification_scale: 1.8,
                reserve_space: false,
                repeated_click: RepeatedClickBehavior::HideApplication,
            },
            clock: ClockSettings {
                format: ClockFormat::TwentyFourHour,
                show_seconds: true,
                ..ClockSettings::default()
            },
            wallpaper: WallpaperSettings {
                default: WallpaperSelection {
                    source: Some("file:///home/test/Pictures/wallpaper.jpg".into()),
                    fit: WallpaperFit::Fill,
                },
                ..WallpaperSettings::default()
            },
            focus: FocusSettings {
                enabled: true,
                selected_mode: Some("work".into()),
                ends_at_unix_ms: Some(4_000_000_000_000),
            },
            providers: BTreeMap::from([(
                ProviderId("files".into()),
                ProviderPolicy {
                    enabled: true,
                    allow_private_content: true,
                    allow_network: false,
                },
            )]),
            spotlight: SpotlightSettings {
                excluded_paths: vec!["/home/test/Private".into()],
                include_removable_mounts: true,
            },
            ..ShellSettings::default()
        }
    }

    #[test]
    fn settings_round_trip_through_versioned_primary_and_last_good_files() {
        let (root, store) = test_store("round-trip");
        let expected = settings();
        store.save(&expected).unwrap();

        assert_eq!(store.load().unwrap().settings, expected);
        let stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
        assert_eq!(stored["version"], CURRENT_VERSION);
        assert!(root.join("shell.json.last-good").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v1_is_migrated_and_rewritten_without_losing_user_choices() {
        let (root, store) = test_store("migration");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            store.path(),
            r#"{
                "version": 1,
                "pinned_apps": ["org.rmac.Finder"],
                "dock": {"placement":"right","autohide":true,"magnification":false},
                "wallpaper": "file:///home/test/old.jpg",
                "focus_mode": "quiet"
            }"#,
        )
        .unwrap();

        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.migrated_from, Some(1));
        assert_eq!(snapshot.settings.dock.placement, DockPlacement::Right);
        assert!(snapshot.settings.dock.autohide);
        assert!(!snapshot.settings.dock.magnification);
        assert_eq!(
            snapshot.settings.focus.selected_mode.as_deref(),
            Some("quiet")
        );
        let rewritten: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
        assert_eq!(rewritten["version"], CURRENT_VERSION);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_primary_recovers_last_known_good_settings() {
        let (root, store) = test_store("recovery");
        let expected = settings();
        store.save(&expected).unwrap();
        std::fs::write(store.path(), b"not json").unwrap();

        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.settings, expected);
        assert!(snapshot.recovered_from_last_good);
        assert!(snapshot.detail.unwrap().contains("primary file failed"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_settings_are_rejected_before_any_write() {
        let (root, store) = test_store("invalid");
        let mut invalid = settings();
        invalid.dock.magnification_scale = f32::NAN;
        let error = store.save(&invalid).unwrap_err();
        assert_eq!(error.operation, Operation::ValidateSettings);
        assert!(!store.path().exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_fields_are_tolerated_but_unknown_versions_are_not() {
        let (root, store) = test_store("future");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            store.path(),
            r#"{"version":2,"settings":{"future":true,"dock":{"future":42}}}"#,
        )
        .unwrap();
        let migrated = store.load().unwrap();
        assert_eq!(migrated.settings, ShellSettings::default());
        assert_eq!(migrated.migrated_from, Some(2));

        std::fs::remove_file(root.join("shell.json.last-good")).unwrap();
        std::fs::write(store.path(), r#"{"version":99,"settings":{}}"#).unwrap();
        let error = store.load().unwrap_err();
        assert_eq!(error.operation, Operation::ParseSettings);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validation_rejects_duplicates_and_incoherent_focus_expiry() {
        let path = Path::new("shell.json");
        let mut duplicate = ShellSettings {
            pinned_apps: vec![AppId("same".into()), AppId("same".into())],
            ..ShellSettings::default()
        };
        assert!(validate(&duplicate, path).is_err());
        duplicate.pinned_apps.clear();
        duplicate.focus.ends_at_unix_ms = Some(1);
        assert!(validate(&duplicate, path).is_err());
        duplicate.focus.enabled = true;
        duplicate.focus.ends_at_unix_ms = None;
        assert!(validate(&duplicate, path).is_err());

        let mut invalid_exclusion = ShellSettings::default();
        invalid_exclusion.spotlight.excluded_paths = vec!["relative/private".into()];
        assert!(validate(&invalid_exclusion, path).is_err());
        invalid_exclusion.spotlight.excluded_paths =
            vec!["/home/test/Private".into(), "/home/test/Private".into()];
        assert!(validate(&invalid_exclusion, path).is_err());
    }

    #[test]
    fn watcher_filter_ignores_last_good_and_accepts_primary_replacement() {
        let target = PathBuf::from("/tmp/rmac/shell.json");
        assert!(!event_targets_path(
            &[PathBuf::from("/tmp/rmac/shell.json.last-good")],
            &target
        ));
        assert!(event_targets_path(
            &[PathBuf::from("/tmp/rmac/.shell.tmp"), target.clone()],
            &target
        ));
    }

    struct FailPrimaryWrite {
        primary: PathBuf,
    }

    impl Backend for FailPrimaryWrite {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            std::fs::read(path)
        }

        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            std::fs::read_to_string(path)
        }

        fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
            if path == self.primary {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected primary write failure",
                ))
            } else {
                rmac_storage::atomic_write(path, contents)
            }
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            std::fs::create_dir_all(path)
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            std::fs::remove_file(path)
        }
    }

    #[test]
    fn failed_primary_write_restores_the_previous_last_good_copy() {
        let (root, store) = test_store("rollback");
        let original = settings();
        store.save(&original).unwrap();
        let path = store.path().to_path_buf();
        let failing = ShellSettingsStore::with_backend(
            path.clone(),
            FailPrimaryWrite {
                primary: path.clone(),
            },
        );
        let mut changed = original.clone();
        changed.dock.placement = DockPlacement::Right;

        let error = failing.save(&changed).unwrap_err();
        assert_eq!(error.operation, Operation::SaveSettings);
        assert_eq!(error.error_kind, io::ErrorKind::PermissionDenied);
        let backup = path.with_file_name("shell.json.last-good");
        let stored: StoredSettings =
            serde_json::from_slice(&std::fs::read(backup).unwrap()).unwrap();
        assert_eq!(stored.settings, original);
        assert_eq!(store.load().unwrap().settings, original);
        std::fs::remove_dir_all(root).unwrap();
    }
}
