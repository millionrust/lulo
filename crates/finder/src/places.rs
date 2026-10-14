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
            ("Pictures", KnownFolder::Pictures, "icons/image.svg"),
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
        ("Photos", "Pictures", "icons/image.svg"),
        ("Movies", "Movies", "icons/movie.svg"),
    ]
    .into_iter()
    .filter_map(|(name, folder, icon)| {
        let path = home.join(folder);
        path.is_dir().then(|| place(name, path, icon, false))
    })
    .collect()
}

/// One of the Mac's own Movies, Music and Pictures favourites, at the folder
/// it really is here (`~/Videos` is the XDG name for `~/Movies`), only when
/// that folder exists.
pub fn media_favourite(home: &Path, media: MediaFolder) -> Option<PlaceSpec> {
    let (name, folders, icon): (&str, &[&str], &'static str) = match media {
        MediaFolder::Movies => ("Movies", &["Movies", "Videos"], "icons/movie.svg"),
        MediaFolder::Music => ("Music", &["Music"], "icons/music.svg"),
        MediaFolder::Pictures => ("Pictures", &["Pictures"], "icons/image.svg"),
    };
    folders
        .iter()
        .map(|folder| home.join(folder))
        .find(|path| path.is_dir())
        .map(|path| place(name, path, icon, false))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaFolder {
    Movies,
    Music,
    Pictures,
}

/// Which Finder ▸ Settings ▸ Sidebar checkbox a mounted volume belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountClass {
    HardDisk,
    ExternalDisk,
    /// "CDs, DVDs and iOS Devices": optical discs and phones or cameras.
    OpticalOrDevice,
    CloudStorage,
    ConnectedServer,
}

/// Sorts one mount into its sidebar checkbox. GVFS mounts carry their
/// scheme in the mount-point name (`smb-share:server=nas,share=docs`);
/// `optical` says the filesystem is ISO 9660 or UDF.
pub fn classify_mount(path: &Path, ejectable: bool, optical: bool) -> MountClass {
    if optical {
        return MountClass::OpticalOrDevice;
    }
    let gvfs = path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|parent| parent == "gvfs");
    if gvfs {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let scheme = name.split(':').next().unwrap_or_default();
        return match scheme {
            "google-drive" | "onedrive" => MountClass::CloudStorage,
            "mtp" | "gphoto2" | "afc" | "cdda" => MountClass::OpticalOrDevice,
            _ => MountClass::ConnectedServer,
        };
    }
    if ejectable {
        MountClass::ExternalDisk
    } else {
        MountClass::HardDisk
    }
}

/// The mount points of optical discs (ISO 9660 and UDF) right now, read
/// from the kernel's mount table each time the sidebar is rebuilt.
#[cfg(target_os = "linux")]
pub fn optical_mount_points() -> Vec<PathBuf> {
    let Ok(bytes) = FileSystem.read_bounded_no_follow(Path::new("/proc/self/mounts"), 4 << 20)
    else {
        return Vec::new();
    };
    optical_mount_points_in(&String::from_utf8_lossy(&bytes))
}

/// No mount table to read: discs show under External disks.
#[cfg(not(target_os = "linux"))]
pub fn optical_mount_points() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(any(target_os = "linux", test))]
fn optical_mount_points_in(table: &str) -> Vec<PathBuf> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _source = fields.next()?;
            let target = fields.next()?;
            let kind = fields.next()?;
            matches!(kind, "iso9660" | "udf")
                .then(|| PathBuf::from(target.replace("\\040", " ").replace("\\011", "\t")))
        })
        .collect()
}

/// This computer's name, for the Locations row the Mac names after the Mac
/// (the pretty name from `/etc/machine-info`, else the host name).
pub fn computer_name() -> String {
    #[cfg(target_os = "linux")]
    {
        let read = |path: &str| {
            FileSystem
                .read_bounded_no_follow(Path::new(path), 64 * 1024)
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        };
        let pretty = read("/etc/machine-info").and_then(|info| {
            info.lines().find_map(|line| {
                line.strip_prefix("PRETTY_HOSTNAME=")
                    .map(|value| value.trim().trim_matches('"').to_owned())
            })
        });
        if let Some(pretty) = pretty.filter(|name| !name.is_empty()) {
            return pretty;
        }
        if let Some(host) = read("/proc/sys/kernel/hostname") {
            let host = host.trim();
            if !host.is_empty() {
                return host.to_owned();
            }
        }
    }
    #[cfg(target_os = "windows")]
    if let Some(name) = std::env::var_os("COMPUTERNAME") {
        return name.to_string_lossy().into_owned();
    }
    "Computer".to_owned()
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

    /// Every sidebar icon this module names ships in `assets/` (Pictures on
    /// Windows and Photos on Lulo OS named a `photo.svg` that never existed,
    /// and drew no icon).
    #[test]
    fn every_place_icon_exists() {
        let source = include_str!("places.rs");
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let mut checked = 0;
        for piece in source.split('"').filter(|piece| {
            piece.starts_with("icons/") && piece.ends_with(".svg") && !piece.contains(' ')
        }) {
            assert!(assets.join(piece).is_file(), "{piece} is missing");
            checked += 1;
        }
        assert!(checked > 5);
    }

    #[test]
    fn favourites_and_locations_follow_files() {
        let home = Path::new("/nonexistent-rmac-home/jake");
        let names: Vec<_> = favourite_folders(home)
            .into_iter()
            .map(|place| place.name)
            .collect();
        assert_eq!(names[..3], ["Downloads", "Documents", "Desktop"]);
        // Windows adds the known folders that exist there; nothing else.
        assert!(cfg!(windows) || names.len() == 3, "{names:?}");
        for name in &names[3..] {
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
    fn mounts_sort_into_the_mac_sidebar_checkboxes() {
        let gvfs = |name: &str| PathBuf::from(format!("/run/user/1000/gvfs/{name}"));
        assert_eq!(
            classify_mount(&gvfs("smb-share:server=nas,share=docs"), true, false),
            MountClass::ConnectedServer
        );
        assert_eq!(
            classify_mount(&gvfs("sftp:host=pi"), true, false),
            MountClass::ConnectedServer
        );
        assert_eq!(
            classify_mount(&gvfs("google-drive:host=gmail.com,user=a"), true, false),
            MountClass::CloudStorage
        );
        assert_eq!(
            classify_mount(&gvfs("mtp:host=Pixel"), true, false),
            MountClass::OpticalOrDevice
        );
        assert_eq!(
            classify_mount(Path::new("/media/jake/DVD"), true, true),
            MountClass::OpticalOrDevice
        );
        assert_eq!(
            classify_mount(Path::new("/media/jake/USB"), true, false),
            MountClass::ExternalDisk
        );
        assert_eq!(
            classify_mount(Path::new("/mnt/data"), false, false),
            MountClass::HardDisk
        );
    }

    #[test]
    fn optical_mounts_are_read_from_the_mount_table() {
        let table = "/dev/sr0 /media/jake/My\\040Disc iso9660 ro 0 0\n\
                     /dev/sdb1 /media/jake/USB vfat rw 0 0\n\
                     /dev/sr1 /media/jake/BD udf ro 0 0\n";
        assert_eq!(
            optical_mount_points_in(table),
            [
                PathBuf::from("/media/jake/My Disc"),
                PathBuf::from("/media/jake/BD")
            ]
        );
    }

    #[test]
    fn movies_falls_back_to_the_xdg_videos_folder() {
        let home = std::env::temp_dir().join(format!(
            "rmac-places-videos-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(home.join("Videos")).unwrap();
        let movies = media_favourite(&home, MediaFolder::Movies).unwrap();
        assert_eq!(movies.name, "Movies");
        assert_eq!(movies.path, home.join("Videos"));
        assert!(media_favourite(&home, MediaFolder::Music).is_none());
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
