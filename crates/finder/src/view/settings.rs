//! Finder ▸ Settings… (⌘,): General, Tags, Sidebar and Advanced — the Mac's
//! own Finder ▸ Settings… window (macOS 26). One versioned JSON document
//! under `XDG_STATE_HOME/rmac/files/settings.json` (the same base tree as
//! `presentation_persistence.rs` and `sidebar_favourites.rs`), shared by
//! every Files window.
//!
//! The settings themselves live behind a process-wide static cache
//! (`current`, `update`) rather than a per-window field, so code with no
//! window at hand — a background-executor closure sorting a directory
//! listing, a rename check, a brand-new window's startup — always reads
//! the same value a just-closed Settings window last wrote, with no stale
//! per-window copy to go out of sync. [`update`] also walks every open
//! Files window and asks it to rebuild the parts of its UI that cache
//! settings-derived state (today, only the sidebar's places), so a change
//! in Settings reaches every open window immediately — the Mac's own
//! contract for these preferences.
//!
//! `open_folders_in_tabs` and the General tab's desktop-item checkboxes are
//! persisted here but not wired to a live effect yet: Files never opens a
//! folder in a new window today (so there is nothing for "instead of new
//! windows" to change), and the desktop icons themselves are drawn by
//! `rmac-desktop`, a separate process. See `docs/parity.md` FILES-35.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use gpui::{AnyWindowHandle, App, AppContext, WeakEntity};
use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

use super::FinderView;

const VERSION: u32 = 1;
const FILE_NAME: &str = "settings.json";
const MAX_FILE_BYTES: usize = 64 * 1024;

/// Finder ▸ Settings… ▸ General ▸ "New Finder windows show:". Mac also
/// offers Recents and Computer; those are special views rather than real
/// folders Files can hand to a new window process today (see `resolve`),
/// so this starts with the folder-backed choices only.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum NewWindowTarget {
    #[default]
    Home,
    Desktop,
    Documents,
    Downloads,
    Computer,
}

impl NewWindowTarget {
    pub(super) const ALL: [Self; 5] = [
        Self::Home,
        Self::Desktop,
        Self::Documents,
        Self::Downloads,
        Self::Computer,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Desktop => "Desktop",
            Self::Documents => "Documents",
            Self::Downloads => "Downloads",
            Self::Computer => "Computer",
        }
    }

    pub(super) fn resolve(self, home: &Path) -> PathBuf {
        match self {
            Self::Home => home.to_path_buf(),
            Self::Desktop => home.join("Desktop"),
            Self::Documents => home.join("Documents"),
            Self::Downloads => home.join("Downloads"),
            Self::Computer => PathBuf::from("/"),
        }
    }
}

/// Finder ▸ Settings… ▸ Advanced ▸ "When performing a search:".
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SearchScope {
    #[default]
    ThisMac,
    CurrentFolder,
    PreviousScope,
}

impl SearchScope {
    pub(super) const ALL: [Self; 3] = [Self::ThisMac, Self::CurrentFolder, Self::PreviousScope];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ThisMac => "Search This Mac",
            Self::CurrentFolder => "Search the Current Folder",
            Self::PreviousScope => "Use the Previous Search Scope",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(super) struct GeneralSettings {
    pub(super) show_hard_disks_on_desktop: bool,
    pub(super) show_external_disks_on_desktop: bool,
    pub(super) show_cds_dvds_on_desktop: bool,
    pub(super) show_connected_servers_on_desktop: bool,
    pub(super) new_window_target: NewWindowTarget,
    pub(super) open_folders_in_tabs: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            show_hard_disks_on_desktop: false,
            show_external_disks_on_desktop: true,
            show_cds_dvds_on_desktop: true,
            show_connected_servers_on_desktop: true,
            new_window_target: NewWindowTarget::default(),
            open_folders_in_tabs: false,
        }
    }
}

/// One row of the Tags tab: a Finder colour tag and whether it has its own
/// row in the sidebar's Tags section. `name` is the lowercase value Files
/// writes into the `user.rmac.tag` / `com.rmac.tag` extended attribute
/// (`view/item_operations.rs`); the sidebar and this tab both title-case it
/// for display.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct TagSetting {
    pub(super) name: String,
    pub(super) color: u32,
    pub(super) show_in_sidebar: bool,
}

impl TagSetting {
    pub(super) fn display_name(&self) -> String {
        let mut chars = self.name.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }
}

/// Finder's seven built-in colour tags, in the Mac's own order.
const DEFAULT_TAGS: [(&str, u32); 7] = [
    ("red", 0xff3b30),
    ("orange", 0xff9500),
    ("yellow", 0xffcc00),
    ("green", 0x34c759),
    ("blue", 0x1372f9),
    ("purple", 0xaf52de),
    ("gray", 0x8e8e93),
];

fn default_tags() -> Vec<TagSetting> {
    DEFAULT_TAGS
        .iter()
        .map(|&(name, color)| TagSetting {
            name: name.to_owned(),
            color,
            show_in_sidebar: true,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(super) struct SidebarSettings {
    pub(super) show_recents: bool,
    pub(super) show_applications: bool,
    pub(super) show_desktop: bool,
    pub(super) show_documents: bool,
    pub(super) show_downloads: bool,
    pub(super) show_home: bool,
    pub(super) show_bin: bool,
    pub(super) show_hard_disks: bool,
    pub(super) show_external_disks: bool,
    pub(super) show_tags: bool,
}

impl Default for SidebarSettings {
    fn default() -> Self {
        Self {
            show_recents: true,
            show_applications: true,
            show_desktop: false,
            show_documents: false,
            show_downloads: true,
            show_home: false,
            show_bin: true,
            show_hard_disks: true,
            show_external_disks: true,
            show_tags: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(super) struct AdvancedSettings {
    pub(super) show_all_filename_extensions: bool,
    pub(super) warn_before_changing_extension: bool,
    pub(super) warn_before_emptying_bin: bool,
    pub(super) remove_items_from_bin_after_30_days: bool,
    pub(super) keep_folders_on_top_in_windows: bool,
    pub(super) keep_folders_on_top_on_desktop: bool,
    pub(super) when_performing_search: SearchScope,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self {
            // The Mac's own default hides a known extension unless a file's
            // own "Hide extension" checkbox (not modelled here) overrides
            // it. Files has never hidden any extension, and the behaviour
            // suite's captures rely on names showing their extension in
            // full, so this starts `true` — "on" here reproduces exactly
            // what Files already did before this setting existed, and only
            // turning it off changes anything.
            show_all_filename_extensions: true,
            warn_before_changing_extension: true,
            warn_before_emptying_bin: true,
            remove_items_from_bin_after_30_days: false,
            keep_folders_on_top_in_windows: false,
            keep_folders_on_top_on_desktop: false,
            when_performing_search: SearchScope::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(super) struct FinderSettings {
    pub(super) general: GeneralSettings,
    pub(super) tags: Vec<TagSetting>,
    pub(super) sidebar: SidebarSettings,
    pub(super) advanced: AdvancedSettings,
}

impl Default for FinderSettings {
    fn default() -> Self {
        Self {
            general: GeneralSettings::default(),
            tags: default_tags(),
            sidebar: SidebarSettings::default(),
            advanced: AdvancedSettings::default(),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct StoredSettings {
    version: u32,
    settings: FinderSettings,
}

#[derive(Debug)]
pub(super) struct SettingsFailure(String);

impl std::fmt::Display for SettingsFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn settings_path() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".local/state"))
        })?;
    Some(state_home.join("rmac/files").join(FILE_NAME))
}

fn load() -> FinderSettings {
    match settings_path() {
        Some(path) => load_from(&path),
        None => FinderSettings::default(),
    }
}

fn load_from(path: &Path) -> FinderSettings {
    let bytes = match FileSystem.read_bounded_no_follow(path, MAX_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(_) => return FinderSettings::default(),
    };
    let Ok(stored) = serde_json::from_slice::<StoredSettings>(&bytes) else {
        return FinderSettings::default();
    };
    if stored.version != VERSION {
        return FinderSettings::default();
    }
    stored.settings
}

fn save(settings: &FinderSettings) -> Result<(), SettingsFailure> {
    let path = settings_path().ok_or_else(|| SettingsFailure("HOME is not set".to_owned()))?;
    save_to(&path, settings)
}

fn save_to(path: &Path, settings: &FinderSettings) -> Result<(), SettingsFailure> {
    let parent = path
        .parent()
        .ok_or_else(|| SettingsFailure("settings path has no parent directory".to_owned()))?;
    rmac_storage::create_dir_all_private(parent)
        .map_err(|error| SettingsFailure(error.to_string()))?;
    let document = StoredSettings {
        version: VERSION,
        settings: settings.clone(),
    };
    let bytes =
        serde_json::to_vec_pretty(&document).map_err(|error| SettingsFailure(error.to_string()))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(SettingsFailure("settings document is too large".to_owned()));
    }
    rmac_storage::atomic_write_private(path, &bytes)
        .map_err(|error| SettingsFailure(error.to_string()))
}

fn cache() -> &'static Mutex<FinderSettings> {
    static CACHE: OnceLock<Mutex<FinderSettings>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(load()))
}

/// The settings every open Files window and every background task reads.
/// Cheap: an in-memory clone, never a disk read, after the first call.
pub(super) fn current() -> FinderSettings {
    cache()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

/// Applies `edit` to the shared settings, saves the result, and asks every
/// open Files window to rebuild the parts of its UI that cache
/// settings-derived state. Returns the save error, if any, so the Settings
/// window can show it — the in-memory change and the broadcast to other
/// windows still happen even if the save itself fails, matching
/// `terminal`'s "edits always apply this session" contract.
pub(super) fn update(
    edit: impl FnOnce(&mut FinderSettings),
    cx: &mut App,
) -> Result<(), SettingsFailure> {
    let settings = {
        let mut guard = cache().lock().unwrap_or_else(|poison| poison.into_inner());
        edit(&mut guard);
        guard.clone()
    };
    broadcast(cx);
    save(&settings)
}

#[derive(Default)]
struct OpenFinderWindows(Vec<(WeakEntity<FinderView>, AnyWindowHandle)>);

impl gpui::Global for OpenFinderWindows {}

/// Registers one Files window to receive a sidebar rebuild whenever
/// settings change. Dead entries (a closed window) are pruned the next
/// time this runs, so the registry never grows across a long session.
pub(super) fn register_window(weak: WeakEntity<FinderView>, handle: AnyWindowHandle, cx: &mut App) {
    if !cx.has_global::<OpenFinderWindows>() {
        cx.set_global(OpenFinderWindows::default());
    }
    let windows = &mut cx.global_mut::<OpenFinderWindows>().0;
    windows.retain(|(existing, _)| existing.entity_id() != weak.entity_id());
    windows.push((weak, handle));
}

/// Every registered Files window; a closed one no longer upgrades.
pub(super) fn finder_windows(cx: &App) -> Vec<WeakEntity<FinderView>> {
    cx.try_global::<OpenFinderWindows>()
        .map(|windows| windows.0.iter().map(|(weak, _)| weak.clone()).collect())
        .unwrap_or_default()
}

/// Close each Finder window through its normal persistence path. The action
/// is deferred until after the active window's event finishes dispatching.
pub(super) fn close_all_windows(cx: &mut App) {
    let Some(windows) = cx.try_global::<OpenFinderWindows>() else {
        return;
    };
    let mut saves = Vec::new();
    for (weak, handle) in windows.0.clone() {
        let _ = cx.update_window(handle, |_, window, cx| {
            if let Ok(save) =
                weak.update(cx, |view, cx| view.close_finder_window_for_all(window, cx))
            {
                saves.push(save);
            }
        });
    }
    cx.background_executor()
        .spawn(async move {
            blocking::unblock(move || {
                for (persistence, state) in saves {
                    persistence.close(state);
                }
            })
            .await;
        })
        .detach();
}

pub(super) fn broadcast(cx: &mut App) {
    let Some(windows) = cx.try_global::<OpenFinderWindows>() else {
        return;
    };
    let live: Vec<_> = windows.0.clone();
    for (weak, _) in live {
        let _ = weak.update(cx, |view, cx| view.rebuild_sidebar_sections(cx));
    }
}

pub(super) fn broadcast_favourites(
    favourites: rmac_finder::sidebar_favourites::Favourites,
    cx: &mut App,
) {
    let Some(windows) = cx.try_global::<OpenFinderWindows>() else {
        return;
    };
    let live = windows.0.clone();
    for (weak, _) in live {
        let _ = weak.update(cx, |view, cx| {
            view.favourite_extras = favourites.paths.clone();
            view.favourite_order = favourites.order.clone();
            view.rebuild_sidebar_sections(cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_match_the_mac_defaults_this_was_measured_against() {
        let settings = FinderSettings::default();
        assert_eq!(settings.general.new_window_target, NewWindowTarget::Home);
        assert!(!settings.general.open_folders_in_tabs);
        assert_eq!(settings.tags.len(), 7);
        assert_eq!(settings.tags[0].name, "red");
        assert!(settings.tags.iter().all(|tag| tag.show_in_sidebar));
        assert!(settings.sidebar.show_recents);
        assert!(!settings.sidebar.show_desktop);
        assert!(settings.advanced.show_all_filename_extensions);
        assert!(settings.advanced.warn_before_emptying_bin);
        assert!(!settings.advanced.keep_folders_on_top_in_windows);
    }

    #[test]
    fn round_trips_through_json() {
        let mut settings = FinderSettings::default();
        settings.general.open_folders_in_tabs = true;
        settings.tags[0].show_in_sidebar = false;
        settings.sidebar.show_bin = false;
        settings.advanced.keep_folders_on_top_in_windows = true;
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
    fn missing_or_wrong_version_file_falls_back_to_defaults() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-files-settings-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join(FILE_NAME);
        assert_eq!(load_from(&path), FinderSettings::default());

        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, br#"{"version":999,"settings":{}}"#).unwrap();
        assert_eq!(load_from(&path), FinderSettings::default());

        let mut settings = FinderSettings::default();
        settings.sidebar.show_bin = false;
        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path), settings);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn tag_display_name_is_title_cased() {
        let tag = TagSetting {
            name: "blue".to_owned(),
            color: 0,
            show_in_sidebar: true,
        };
        assert_eq!(tag.display_name(), "Blue");
    }
}
