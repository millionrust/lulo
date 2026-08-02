//! Versioned settings serialization and migrations.

use super::*;

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct StoredSettings {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) settings: ShellSettings,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(super) struct LegacyDockSettings {
    pub(super) placement: DockPlacement,
    pub(super) autohide: bool,
    pub(super) magnification: bool,
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
pub(super) struct LegacySettings {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) pinned_apps: Vec<AppId>,
    #[serde(default)]
    pub(super) dock: LegacyDockSettings,
    #[serde(default)]
    pub(super) wallpaper: Option<String>,
    #[serde(default)]
    pub(super) focus_mode: Option<String>,
}

pub(super) struct Loaded {
    pub(super) settings: ShellSettings,
    pub(super) migrated_from: Option<u32>,
}

pub(super) fn serialize_settings(settings: &ShellSettings, path: &Path) -> Result<Vec<u8>, Error> {
    serde_json::to_vec_pretty(&StoredSettings {
        version: CURRENT_VERSION,
        settings: settings.clone(),
    })
    .map_err(|error| Failure::message(Operation::SerializeSettings, path, error.to_string()))
}

pub(super) fn migrate_v1(legacy: LegacySettings) -> ShellSettings {
    migrate_application_ids(ShellSettings {
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
    })
}

pub(super) fn migrate_application_ids(mut settings: ShellSettings) -> ShellSettings {
    let mut files_origin: Option<bool> = None;
    settings.pinned_apps.retain_mut(|app| {
        let legacy = app.0 == LEGACY_FILES_APP_ID;
        if legacy {
            app.0 = FILES_APP_ID.into();
        }
        if app.0 != FILES_APP_ID {
            return true;
        }
        match files_origin {
            Some(first_was_legacy) if first_was_legacy != legacy => false,
            Some(_) => true,
            None => {
                files_origin = Some(legacy);
                true
            }
        }
    });
    settings
}
