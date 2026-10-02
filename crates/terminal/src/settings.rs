//! Terminal ▸ Settings… preferences beyond the Profile/Font/Option-as-Meta
//! ones `profiles.rs` already persists (one file per setting): cursor
//! style/blink, the window size new windows open with, "when the shell
//! exits", and "new windows open with" (working directory). Kept in one
//! versioned JSON document (`settings.json`, `rmac-storage`-backed, atomic
//! and owner-only) rather than more flat files, since these six values are
//! always read and written together from the same Settings window pane.
//!
//! Every value here takes effect for the *next* window or tab without
//! restarting the app — the same "no restart needed" contract
//! `profiles.rs`'s font-size/option-as-meta already have, not a push into
//! windows already open (mirrors "Use Settings as Default").

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage;

const VERSION: u32 = 1;
const FILE_NAME: &str = "settings.json";
const MAX_FILE_BYTES: usize = 4 * 1024;

/// A new Terminal window on macOS 26.2 is 80 × 24 (measured, `controller.rs`).
pub(crate) const DEFAULT_COLUMNS: u16 = 80;
pub(crate) const DEFAULT_ROWS: u16 = 24;
pub(crate) const MIN_COLUMNS: u16 = 20;
pub(crate) const MAX_COLUMNS: u16 = 500;
pub(crate) const MIN_ROWS: u16 = 5;
pub(crate) const MAX_ROWS: u16 = 300;

/// Terminal ▸ Settings… ▸ Text ▸ Cursor style.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

impl CursorStyle {
    pub(crate) const ALL: [Self; 3] = [Self::Block, Self::Underline, Self::Bar];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Block => "▊ Block",
            Self::Underline => "▁ Underline",
            Self::Bar => "┃ Vertical Bar",
        }
    }
}

/// Terminal ▸ Settings… ▸ Shell ▸ "When the shell exits".
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ShellExitBehavior {
    #[default]
    DontClose,
    CloseIfCleanExit,
}

impl ShellExitBehavior {
    pub(crate) const ALL: [Self; 2] = [Self::DontClose, Self::CloseIfCleanExit];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::DontClose => "Don't close the window",
            Self::CloseIfCleanExit => "Close if the shell exited cleanly",
        }
    }
}

/// Terminal ▸ Settings… ▸ General ▸ "New windows open with".
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NewWindowWorkingDirectory {
    #[default]
    Home,
    SameWorkingDirectory,
}

impl NewWindowWorkingDirectory {
    pub(crate) const ALL: [Self; 2] = [Self::Home, Self::SameWorkingDirectory];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Home => "Home directory",
            Self::SameWorkingDirectory => "Same working directory",
        }
    }
}

/// Every value Terminal ▸ Settings… edits beyond Profile/Font/Option-as-Meta.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) cursor_style: CursorStyle,
    pub(crate) cursor_blink: bool,
    pub(crate) use_bold_fonts: bool,
    pub(crate) bright_bold_text: bool,
    pub(crate) display_ansi_colours: bool,
    pub(crate) columns: u16,
    pub(crate) rows: u16,
    pub(crate) when_shell_exits: ShellExitBehavior,
    pub(crate) new_window_directory: NewWindowWorkingDirectory,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cursor_style: CursorStyle::default(),
            cursor_blink: true,
            use_bold_fonts: true,
            bright_bold_text: true,
            display_ansi_colours: true,
            columns: DEFAULT_COLUMNS,
            rows: DEFAULT_ROWS,
            when_shell_exits: ShellExitBehavior::default(),
            new_window_directory: NewWindowWorkingDirectory::default(),
        }
    }
}

impl Settings {
    fn normalized(mut self) -> Self {
        self.columns = self.columns.clamp(MIN_COLUMNS, MAX_COLUMNS);
        self.rows = self.rows.clamp(MIN_ROWS, MAX_ROWS);
        self
    }
}

#[derive(Deserialize, Serialize)]
struct StoredSettings {
    version: u32,
    settings: Settings,
}

fn settings_path() -> Result<PathBuf, storage::Failure> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::ResolveConfigPath,
            Path::new(FILE_NAME),
            "HOME is not set",
        )
    })?;
    #[cfg(target_os = "macos")]
    let directory = home.join("Library/Application Support/rmac-terminal");
    #[cfg(not(target_os = "macos"))]
    let directory = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path.join("rmac-terminal"),
        _ => home.join(".config/rmac-terminal"),
    };
    Ok(directory.join(FILE_NAME))
}

/// Load the saved settings, or the Mac-matching defaults if none were ever
/// saved. A corrupt or future-versioned file is reported — mirroring
/// `rmac-window-state`'s contract — rather than silently reset to defaults,
/// so a damaged `settings.json` surfaces instead of quietly losing edits.
pub(crate) fn load() -> Result<Settings, storage::Failure> {
    load_from(&settings_path()?)
}

fn load_from(path: &Path) -> Result<Settings, storage::Failure> {
    let bytes = match rmac_storage::read_bounded_no_follow(path, MAX_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default())
        }
        Err(error) => {
            return Err(storage::Failure::from_io(
                storage::Operation::LoadSetting,
                path,
                error,
            ))
        }
    };
    let stored: StoredSettings = serde_json::from_slice(&bytes).map_err(|error| {
        storage::Failure::message(storage::Operation::LoadSetting, path, error.to_string())
    })?;
    if stored.version != VERSION {
        return Err(storage::Failure::message(
            storage::Operation::LoadSetting,
            path,
            format!("settings file version {} is not supported", stored.version),
        ));
    }
    Ok(stored.settings.normalized())
}

pub(crate) fn save(settings: &Settings) -> Result<(), storage::Failure> {
    let path = settings_path()?;
    let parent = path.parent().ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::ResolveConfigPath,
            &path,
            "preferences path has no parent directory",
        )
    })?;
    rmac_storage::create_dir_all_private(parent).map_err(|error| {
        storage::Failure::from_io(storage::Operation::CreateConfigDirectory, parent, error)
    })?;
    let document = StoredSettings {
        version: VERSION,
        settings: settings.clone().normalized(),
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|error| {
        storage::Failure::message(storage::Operation::SaveSetting, &path, error.to_string())
    })?;
    rmac_storage::atomic_write_private(&path, &bytes)
        .map_err(|error| storage::Failure::from_io(storage::Operation::SaveSetting, &path, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_match_todays_mac_defaults() {
        let settings = Settings::default();
        assert_eq!(settings.columns, 80);
        assert_eq!(settings.rows, 24);
        assert_eq!(settings.cursor_style, CursorStyle::Block);
        assert!(settings.cursor_blink);
        assert!(settings.use_bold_fonts);
        assert!(settings.bright_bold_text);
        assert!(settings.display_ansi_colours);
        assert_eq!(settings.when_shell_exits, ShellExitBehavior::DontClose);
        assert_eq!(
            settings.new_window_directory,
            NewWindowWorkingDirectory::Home
        );
    }

    #[test]
    fn round_trips_through_json() {
        let settings = Settings {
            cursor_style: CursorStyle::Bar,
            cursor_blink: false,
            use_bold_fonts: false,
            bright_bold_text: false,
            display_ansi_colours: false,
            columns: 120,
            rows: 40,
            when_shell_exits: ShellExitBehavior::CloseIfCleanExit,
            new_window_directory: NewWindowWorkingDirectory::SameWorkingDirectory,
        };
        let document = StoredSettings {
            version: VERSION,
            settings: settings.clone(),
        };
        let bytes = serde_json::to_vec(&document).unwrap();
        let decoded: StoredSettings = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.version, VERSION);
        assert_eq!(decoded.settings, settings);
    }

    #[test]
    fn out_of_range_values_are_clamped_on_load() {
        let hostile = Settings {
            columns: 0,
            rows: 0,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(hostile.columns, MIN_COLUMNS);
        assert_eq!(hostile.rows, MIN_ROWS);

        let hostile_high = Settings {
            columns: u16::MAX,
            rows: u16::MAX,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(hostile_high.columns, MAX_COLUMNS);
        assert_eq!(hostile_high.rows, MAX_ROWS);
    }

    #[test]
    fn missing_or_wrong_version_file_falls_back_to_defaults() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-terminal-settings-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join(FILE_NAME);
        assert_eq!(load_from(&path), Ok(Settings::default()));

        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, br#"{"version":999,"settings":{}}"#).unwrap();
        assert!(load_from(&path).is_err());

        let settings = Settings {
            columns: 132,
            ..Settings::default()
        };
        let document = StoredSettings {
            version: VERSION,
            settings: settings.clone(),
        };
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(load_from(&path), Ok(settings));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
