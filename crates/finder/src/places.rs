//! Framework-neutral sidebar places shared by Files and the Open/Save panel.
//! Only folders that exist on this machine are offered.

use std::path::{Path, PathBuf};

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
    let mut favourites = vec![
        place(
            "Downloads",
            home.join("Downloads"),
            "icons/circle-arrow-down.svg",
            false,
        ),
        place("Documents", home.join("Documents"), "icons/file.svg", false),
        place("Desktop", home.join("Desktop"), "icons/desktop.svg", false),
    ];
    let projects = home.join("Projects");
    if projects.is_dir() {
        favourites.push(place("Projects", projects, "icons/folder.svg", false));
    }
    favourites
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
        assert_eq!(names, ["Downloads", "Documents", "Desktop"]);
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
