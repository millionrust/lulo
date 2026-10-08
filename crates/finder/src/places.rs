//! Framework-neutral sidebar places shared by Files and the Open/Save panel.
//! Only folders that exist on this machine are offered.

use rmac_storage::{Backend as _, FileSystem};
use std::path::{Path, PathBuf};

/// The Files Settings ▸ Sidebar choices used by the separate chooser process.
/// It reads the same versioned document that Files writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidebarVisibility {
    pub recents: bool,
    pub applications: bool,
    pub desktop: bool,
    pub documents: bool,
    pub downloads: bool,
    pub home: bool,
    pub hard_disks: bool,
    pub external_disks: bool,
}

impl Default for SidebarVisibility {
    fn default() -> Self {
        Self {
            recents: true,
            applications: true,
            desktop: cfg!(windows),
            documents: cfg!(windows),
            downloads: true,
            home: false,
            hard_disks: true,
            external_disks: true,
        }
    }
}

pub fn sidebar_visibility() -> SidebarVisibility {
    let mut visibility = SidebarVisibility::default();
    let Some(state_home) = std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
    else {
        return visibility;
    };
    let path = state_home.join("rmac/files/settings.json");
    let Ok(bytes) = FileSystem.read_bounded_no_follow(&path, 64 * 1024) else {
        return visibility;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return visibility;
    };
    if value["version"].as_u64() != Some(1) {
        return visibility;
    }
    let sidebar = &value["settings"]["sidebar"];
    let enabled = |name: &str, default| sidebar[name].as_bool().unwrap_or(default);
    visibility.recents = enabled("show_recents", visibility.recents);
    visibility.applications = enabled("show_applications", visibility.applications);
    visibility.desktop = enabled("show_desktop", visibility.desktop);
    visibility.documents = enabled("show_documents", visibility.documents);
    visibility.downloads = enabled("show_downloads", visibility.downloads);
    visibility.home = enabled("show_home", visibility.home);
    visibility.hard_disks = enabled("show_hard_disks", visibility.hard_disks);
    visibility.external_disks = enabled("show_external_disks", visibility.external_disks);
    visibility
}

/// One real folder in the sidebar. `icon` is an asset path inside
/// `crates/finder/assets`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceSpec {
    pub name: String,
    pub path: PathBuf,
    pub icon: &'static str,
    /// Drawn in the secondary (drive-grey) tint rather than the accent.
    pub secondary_tint: bool,
}

fn place(name: &str, path: PathBuf, icon: &'static str, secondary_tint: bool) -> PlaceSpec {
    PlaceSpec {
        name: name.to_owned(),
        path,
        icon,
        secondary_tint,
    }
}

pub fn root_volume_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Macintosh HD"
    } else {
        "Computer"
    }
}

/// `~/Public`, shown as “Shared” when it exists.
pub fn shared_folder(home: &Path) -> Option<PlaceSpec> {
    let shared = home.join("Public");
    shared
        .is_dir()
        .then(|| place("Shared", shared, "icons/shared-folder.svg", false))
}

/// The folder favourites in the owner's Finder order. The Mac hides Movies
/// and the root volume from the sidebar by default, so rmac does too;
/// ⇧⌘C and the path bar still reach the root.
pub fn favourite_folders(home: &Path) -> Vec<PlaceSpec> {
    #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
    let mut favourites = vec![
        place(
            "Downloads",
            known_or_home_folder(KnownFolder::Downloads, home, "Downloads"),
            "icons/circle-arrow-down.svg",
            false,
        ),
        place(
            "Documents",
            known_or_home_folder(KnownFolder::Documents, home, "Documents"),
            "icons/file.svg",
            false,
        ),
        place(
            "Desktop",
            known_or_home_folder(KnownFolder::Desktop, home, "Desktop"),
            "icons/desktop.svg",
            false,
        ),
    ];
    // Explorer's own set on Windows: Pictures, Music and Videos (each where
    // the user's known folder really is), and OneDrive when it syncs here.
    #[cfg(target_os = "windows")]
    favourites.extend(
        [
            ("Pictures", KnownFolder::Pictures, "icons/photo.svg"),
            ("Music", KnownFolder::Music, "icons/music.svg"),
            ("Videos", KnownFolder::Videos, "icons/movie.svg"),
            ("OneDrive", KnownFolder::OneDrive, "icons/cloud.svg"),
        ]
        .into_iter()
        .filter_map(|(name, folder, icon)| {
            let path = known_or_home_folder(folder, home, name);
            path.is_dir().then(|| place(name, path, icon, false))
        }),
    );
    favourites
}

/// The known folders Favourites maps on Windows. Desktop, Documents and
/// Downloads can each be redirected (OneDrive's "Manage backup", a roaming
/// profile, a per-machine policy), so a plain `home.join(name)` would be
/// wrong exactly when it matters; `SHGetKnownFolderPath` is the one real
/// answer there (ADR 0023 phase 4). Linux and macOS have no such
/// redirection, so the enum only matters on Windows.
enum KnownFolder {
    Desktop,
    Documents,
    Downloads,
    #[cfg(target_os = "windows")]
    Pictures,
    #[cfg(target_os = "windows")]
    Music,
    #[cfg(target_os = "windows")]
    Videos,
    #[cfg(target_os = "windows")]
    OneDrive,
}

#[cfg(target_os = "windows")]
fn known_or_home_folder(folder: KnownFolder, home: &Path, fallback_name: &str) -> PathBuf {
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
        FOLDERID_Pictures, FOLDERID_SkyDrive, FOLDERID_Videos, SHGetKnownFolderPath,
        KF_FLAG_DEFAULT,
    };

    let id = match folder {
        KnownFolder::Desktop => &FOLDERID_Desktop,
        KnownFolder::Documents => &FOLDERID_Documents,
        KnownFolder::Downloads => &FOLDERID_Downloads,
        KnownFolder::Pictures => &FOLDERID_Pictures,
        KnownFolder::Music => &FOLDERID_Music,
        KnownFolder::Videos => &FOLDERID_Videos,
        KnownFolder::OneDrive => &FOLDERID_SkyDrive,
    };
    // SAFETY: `id` is one of the well-known folder GUIDs above; no token
    // (the current user) and the default flags ask for no extra behaviour.
    let resolved = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }
        .ok()
        .and_then(|wide| {
            let path = unsafe { wide.to_string() }.ok().map(PathBuf::from);
            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(wide.0 as *const _)) };
            path
        });
    resolved.unwrap_or_else(|| home.join(fallback_name))
}

/// Non-Windows: a plain `home.join(name)` is already right (Linux and
/// macOS have no per-folder redirection to resolve).
#[cfg(not(target_os = "windows"))]
fn known_or_home_folder(_folder: KnownFolder, home: &Path, fallback_name: &str) -> PathBuf {
    home.join(fallback_name)
}

/// The Mac's `/Applications`, offered whenever a like-named real folder
/// exists on this machine. Linux has no bundle-app folder for a file panel
/// to browse; `/usr/share/applications` is the closest real directory (the
/// desktop entries every installed app publishes), so it stands in rather
/// than leaving the section out entirely.
pub fn applications_folder() -> Option<PlaceSpec> {
    let path = PathBuf::from("/usr/share/applications");
    path.is_dir()
        .then(|| place("Applications", path, "icons/layout-grid.svg", false))
}

/// `~/Music`, `~/Pictures` and `~/Movies`, the Mac's Media section, shown
/// only for folders that exist (as `favourite_folders` already does).
pub fn media_folders(home: &Path) -> Vec<PlaceSpec> {
    [
        ("Music", "Music", "icons/music.svg"),
        ("Photos", "Pictures", "icons/photo.svg"),
        ("Movies", "Movies", "icons/movie.svg"),
    ]
    .into_iter()
    .filter_map(|(name, folder, icon)| {
        let path = home.join(folder);
        path.is_dir().then(|| place(name, path, icon, false))
    })
    .collect()
}

/// iCloud Drive (when synced here) and the home folder.
pub fn standard_locations(home: &Path) -> Vec<PlaceSpec> {
    let mut locations = Vec::new();
    let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    if icloud.is_dir() {
        locations.push(place("iCloud Drive", icloud, "icons/cloud.svg", false));
    }
    let host = home
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root_volume_name().to_owned());
    locations.push(place(&host, home.to_path_buf(), "icons/house.svg", true));
    locations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favourites_and_locations_follow_files() {
        let home = Path::new("/nonexistent-rmac-home/jake");
        let names: Vec<_> = favourite_folders(home)
            .into_iter()
            .map(|place| place.name)
            .collect();
        assert_eq!(names[..3], ["Downloads", "Documents", "Desktop"]);
        // Windows adds the known folders that exist there.
        for name in &names[3..] {
            assert!(cfg!(windows), "{name}");
            assert!(["Pictures", "Music", "Videos", "OneDrive"].contains(&name.as_str()));
        }
        let locations = standard_locations(home);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].name, "jake");
        assert!(shared_folder(home).is_none());
    }

    #[test]
    fn media_folders_are_offered_only_when_they_exist() {
        let home = Path::new("/nonexistent-rmac-home/jake");
        assert!(media_folders(home).is_empty());
    }

    #[test]
    fn media_folders_use_the_mac_names_in_the_mac_order() {
        let home = std::env::temp_dir().join(format!(
            "rmac-places-media-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        for folder in ["Music", "Pictures", "Movies"] {
            std::fs::create_dir_all(home.join(folder)).unwrap();
        }
        let names: Vec<_> = media_folders(&home)
            .into_iter()
            .map(|place| place.name)
            .collect();
        assert_eq!(names, ["Music", "Photos", "Movies"]);
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn applications_folder_is_offered_only_when_real() {
        // `/usr/share/applications` exists on every mainstream Linux desktop
        // (including CI containers); this only checks the function agrees
        // with the filesystem, not that the folder is present everywhere.
        assert_eq!(
            applications_folder().is_some(),
            Path::new("/usr/share/applications").is_dir()
        );
    }
}
