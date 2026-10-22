//! Sidebar section/place construction, shared by window startup
//! (`startup.rs`) and by [`FinderView::rebuild_sidebar_sections`], which
//! the Settings window's Sidebar and Tags tabs call into (via
//! `settings::update`'s broadcast) whenever their checkboxes change, so
//! every open window's sidebar reflects a change immediately — matching
//! the Mac's own "live" preferences contract for FILES-35.

use super::settings::FinderSettings;
use super::*;

/// What the sidebar reads from the machine besides the settings: the
/// computer's name and which mounts are optical discs. Gathered by
/// [`SidebarSystem::current`] each time the sidebar is rebuilt (a settings
/// change or a mount change), so [`build_sections`] itself stays pure.
#[derive(Clone, Debug, Default)]
pub(super) struct SidebarSystem {
    pub(super) computer_name: String,
    pub(super) optical_mounts: Vec<PathBuf>,
}

impl SidebarSystem {
    pub(super) fn current() -> Self {
        Self {
            computer_name: rmac_finder::places::computer_name(),
            optical_mounts: rmac_finder::places::optical_mount_points(),
        }
    }
}

/// Builds every sidebar section, filtered by the Settings window's Sidebar
/// and Tags tabs. Pure, so the initial build, every settings change and
/// every mount change share one code path.
pub(super) fn build_sections(
    home: &Path,
    mounts: &[rmac_mounts::Mount],
    favourite_extras: &[PathBuf],
    favourite_order: &[FavouriteKey],
    file_words: &rmac_locale::FileVocabulary,
    settings: &FinderSettings,
    system: &SidebarSystem,
) -> Vec<Section> {
    use rmac_finder::places::{MediaFolder, MountClass};

    let p = |name: &str, path: PathBuf, icon: &'static str, tint: Hsla, kind: PlaceKind| Place {
        name: name.to_string().into(),
        path,
        icon,
        tint,
        kind,
    };
    let sidebar = &settings.sidebar;

    // Folder places come from the shared Files model (also used by the
    // Open/Save panel), so both sidebars list the same real folders.
    let from_spec = |spec: rmac_finder::places::PlaceSpec| {
        let tint = if spec.secondary_tint {
            drive_gray()
        } else {
            accent()
        };
        p(&spec.name, spec.path, spec.icon, tint, PlaceKind::Item)
    };

    // Locations, in the Mac's Settings order: Cloud Storage (iCloud Drive
    // when it syncs here), the home folder, the computer, then each mount
    // under its own checkbox, then the Bin.
    let mut locations = Vec::new();
    for spec in rmac_finder::places::standard_locations(home) {
        let shown = if spec.path == home {
            sidebar.show_home
        } else {
            sidebar.show_cloud_storage
        };
        if shown {
            locations.push(from_spec(spec));
        }
    }
    if sidebar.show_computer {
        let name = if system.computer_name.is_empty() {
            rmac_finder::places::root_volume_name()
        } else {
            system.computer_name.as_str()
        };
        locations.push(p(
            name,
            PathBuf::from("/"),
            "icons/hard-drive.svg",
            drive_gray(),
            PlaceKind::Item,
        ));
    }
    for mount in mounts {
        let optical = system.optical_mounts.contains(&mount.path);
        let class = rmac_finder::places::classify_mount(&mount.path, mount.ejectable, optical);
        let (shown, icon) = match class {
            MountClass::HardDisk => (sidebar.show_hard_disks, "icons/hard-drive.svg"),
            MountClass::ExternalDisk => (sidebar.show_external_disks, "icons/hard-drive.svg"),
            MountClass::OpticalOrDevice => (
                sidebar.show_cds_dvds_and_ios_devices,
                if optical {
                    "icons/hard-drive.svg"
                } else {
                    "icons/smartphone.svg"
                },
            ),
            MountClass::CloudStorage => (sidebar.show_cloud_storage, "icons/cloud.svg"),
            MountClass::ConnectedServer => (sidebar.show_connected_servers, "icons/share-2.svg"),
        };
        if shown {
            locations.push(p(
                &mount.name,
                mount.path.clone(),
                icon,
                drive_gray(),
                if mount.ejectable {
                    PlaceKind::Volume
                } else {
                    PlaceKind::Item
                },
            ));
        }
    }
    // Network: connected servers and the computers GVFS finds (FILES-07).
    // Finder lists it whether or not Bonjour computers are shown too.
    if cfg!(target_os = "linux") {
        locations.push(p(
            "Network",
            PathBuf::new(),
            "icons/globe.svg",
            drive_gray(),
            PlaceKind::Network,
        ));
    }
    if cfg!(target_os = "linux") && sidebar.show_bin {
        locations.push(p(
            file_words.bin(),
            PathBuf::new(),
            "icons/trash-2.svg",
            accent(),
            PlaceKind::Trash,
        ));
    }

    let mut prominent = Vec::new();
    if sidebar.show_recents {
        prominent.push(p(
            "Recents",
            PathBuf::new(),
            "icons/clock.svg",
            accent(),
            PlaceKind::Recents,
        ));
    }
    if sidebar.show_shared {
        if let Some(shared) = rmac_finder::places::shared_folder(home) {
            prominent.push(from_spec(shared));
        }
    }

    let mut favorites = Vec::new();
    if sidebar.show_applications {
        favorites.push(p(
            "Applications",
            PathBuf::new(),
            "icons/layout-grid.svg",
            accent(),
            PlaceKind::Applications,
        ));
    }
    favorites.extend(
        rmac_finder::places::favourite_folders(home)
            .into_iter()
            .filter(|spec| match spec.name.as_str() {
                "Desktop" => sidebar.show_desktop,
                "Documents" => sidebar.show_documents,
                "Downloads" => sidebar.show_downloads,
                // Explorer's own known folders on Windows.
                "Videos" => sidebar.show_movies,
                "Music" => sidebar.show_music,
                "Pictures" => sidebar.show_pictures,
                "OneDrive" => sidebar.show_cloud_storage,
                _ => true,
            })
            .map(from_spec),
    );
    // Windows' `favourite_folders` already lists its Videos, Music and
    // Pictures; elsewhere the Mac's Movies, Music and Pictures come from
    // the home folder.
    if !cfg!(windows) {
        for (shown, media) in [
            (sidebar.show_movies, MediaFolder::Movies),
            (sidebar.show_music, MediaFolder::Music),
            (sidebar.show_pictures, MediaFolder::Pictures),
        ] {
            if let Some(spec) = shown
                .then(|| rmac_finder::places::media_favourite(home, media))
                .flatten()
            {
                favorites.push(from_spec(spec));
            }
        }
    }
    // Keep missing targets visible so a click can explain the problem.
    favorites.extend(
        favourite_extras
            .iter()
            .map(|path| sidebar_favourites::extra_favourite_place(path)),
    );
    favorites.sort_by_key(|place| {
        favourite_order
            .iter()
            .position(|key| match key {
                FavouriteKey::Applications => place.kind == PlaceKind::Applications,
                FavouriteKey::Path(path) => {
                    path == &place.path && place.kind != PlaceKind::Applications
                }
            })
            .unwrap_or(usize::MAX)
    });

    let mut sections = vec![
        Section {
            title: "".into(),
            places: prominent,
        },
        Section {
            title: file_words.favourites().into(),
            places: favorites,
        },
        Section {
            title: "Locations".into(),
            places: locations,
        },
    ];
    if sidebar.show_tags {
        let tag = |name: &str, color: u32| p(name, PathBuf::new(), "", hsl(color), PlaceKind::Tag);
        let places = settings
            .tags
            .iter()
            .filter(|tag| tag.show_in_sidebar)
            .map(|setting| tag(&setting.display_name(), setting.color))
            .collect::<Vec<_>>();
        if !places.is_empty() {
            sections.push(Section {
                title: "Tags".into(),
                places,
            });
        }
    }
    sections
}

impl FinderView {
    /// Rebuilds the sidebar from the current settings and mounts — called
    /// at startup, every time Finder ▸ Settings… ▸ Sidebar or ▸ Tags
    /// changes (`settings::update`'s broadcast to every open window), and
    /// whenever a volume is mounted or unmounted.
    pub(super) fn rebuild_sidebar_sections(&mut self, cx: &mut Context<Self>) {
        self.rebuild_sidebar_sections_now();
        cx.notify();
    }

    pub(super) fn rebuild_sidebar_sections_now(&mut self) {
        self.sections = build_sections(
            &self.home,
            &self.mounts,
            &self.favourite_extras,
            &self.favourite_order,
            &self.file_words,
            &super::settings::current(),
            &SidebarSystem::current(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(sections: &[Section]) -> Vec<String> {
        sections
            .iter()
            .flat_map(|section| &section.places)
            .map(|place| place.name.to_string())
            .collect()
    }

    #[test]
    fn sidebar_checkboxes_hide_the_matching_favourite_and_location_rows() {
        let home = Path::new("/nonexistent-rmac-sidebar-test/jake");
        let words = rmac_locale::FileVocabulary::for_locale("en_US.UTF-8");
        let mut settings = FinderSettings::default();
        settings.sidebar.show_recents = false;
        settings.sidebar.show_applications = false;
        settings.sidebar.show_desktop = false;
        settings.sidebar.show_documents = false;
        settings.sidebar.show_downloads = true;

        let shown = names(&build_sections(
            home,
            &[],
            &[],
            &[],
            &words,
            &settings,
            &SidebarSystem::default(),
        ));
        assert!(!shown.contains(&"Recents".to_owned()));
        assert!(!shown.contains(&"Applications".to_owned()));
        assert!(!shown.contains(&"Desktop".to_owned()));
        assert!(!shown.contains(&"Documents".to_owned()));
        assert!(shown.contains(&"Downloads".to_owned()));
    }

    #[test]
    fn tags_section_is_omitted_when_the_sidebar_setting_is_off() {
        let home = Path::new("/nonexistent-rmac-sidebar-test/jake");
        let words = rmac_locale::FileVocabulary::for_locale("en_US.UTF-8");
        let mut settings = FinderSettings::default();
        settings.sidebar.show_tags = false;

        let sections = build_sections(
            home,
            &[],
            &[],
            &[],
            &words,
            &settings,
            &SidebarSystem::default(),
        );
        assert!(!sections
            .iter()
            .any(|section| section.title.as_ref() == "Tags"));
    }

    #[test]
    fn a_tag_hidden_from_the_sidebar_is_not_in_its_section_but_others_still_are() {
        let home = Path::new("/nonexistent-rmac-sidebar-test/jake");
        let words = rmac_locale::FileVocabulary::for_locale("en_US.UTF-8");
        let mut settings = FinderSettings::default();
        settings.tags[0].show_in_sidebar = false; // "red"

        let shown = names(&build_sections(
            home,
            &[],
            &[],
            &[],
            &words,
            &settings,
            &SidebarSystem::default(),
        ));
        assert!(!shown.contains(&"Red".to_owned()));
        assert!(shown.contains(&"Blue".to_owned()));
    }

    #[test]
    fn mounts_are_split_between_hard_disks_and_external_disks() {
        let home = Path::new("/nonexistent-rmac-sidebar-test/jake");
        let words = rmac_locale::FileVocabulary::for_locale("en_US.UTF-8");
        let internal = rmac_mounts::Mount {
            identity: "internal".to_owned(),
            name: "Internal".into(),
            path: PathBuf::from("/mnt/internal"),
            ejectable: false,
        };
        let external = rmac_mounts::Mount {
            identity: "external".to_owned(),
            name: "External".into(),
            path: PathBuf::from("/mnt/external"),
            ejectable: true,
        };
        let mounts = [internal, external];

        let mut settings = FinderSettings::default();
        settings.sidebar.show_hard_disks = false;
        settings.sidebar.show_external_disks = true;
        let shown = names(&build_sections(
            home,
            &mounts,
            &[],
            &[],
            &words,
            &settings,
            &SidebarSystem::default(),
        ));
        assert!(!shown.contains(&"Internal".to_owned()));
        assert!(shown.contains(&"External".to_owned()));
    }

    #[test]
    #[cfg(unix)]
    fn saved_order_moves_custom_and_builtin_favourites_together() {
        let home = Path::new("/nonexistent-rmac-sidebar-test/jake");
        let words = rmac_locale::FileVocabulary::for_locale("en_US.UTF-8");
        let custom = PathBuf::from("/nonexistent-rmac-sidebar-test/project.txt");
        let order = [
            FavouriteKey::Path(custom.clone()),
            FavouriteKey::Path(home.join("Downloads")),
            FavouriteKey::Applications,
        ];
        let sections = build_sections(
            home,
            &[],
            &[custom],
            &order,
            &words,
            &FinderSettings::default(),
            &SidebarSystem::default(),
        );
        let names = sections
            .iter()
            .find(|section| section.title.as_ref() == words.favourites())
            .unwrap()
            .places
            .iter()
            .map(|place| place.name.as_ref())
            .collect::<Vec<&str>>();
        assert_eq!(names, ["project.txt", "Downloads", "Applications"]);
    }

    fn mount(name: &str, path: &str, ejectable: bool) -> rmac_mounts::Mount {
        rmac_mounts::Mount {
            identity: name.to_owned(),
            name: name.into(),
            path: PathBuf::from(path),
            ejectable,
        }
    }

    #[test]
    fn each_location_checkbox_hides_only_its_own_mounts() {
        let home = Path::new("/nonexistent-rmac-sidebar-test/jake");
        let words = rmac_locale::FileVocabulary::for_locale("en_US.UTF-8");
        let mounts = [
            mount(
                "Docs",
                "/run/user/1000/gvfs/smb-share:server=nas,share=docs",
                true,
            ),
            mount(
                "Drive",
                "/run/user/1000/gvfs/google-drive:host=gmail.com",
                true,
            ),
            mount("Pixel", "/run/user/1000/gvfs/mtp:host=Pixel", true),
            mount("Film", "/media/jake/Film", true),
            mount("USB", "/media/jake/USB", true),
        ];
        let system = SidebarSystem {
            computer_name: "Lulo Laptop".to_owned(),
            optical_mounts: vec![PathBuf::from("/media/jake/Film")],
        };
        let all = |settings: &FinderSettings| {
            names(&build_sections(
                home,
                &mounts,
                &[],
                &[],
                &words,
                settings,
                &system,
            ))
        };
        let defaults = FinderSettings::default();
        let shown = all(&defaults);
        for name in ["Docs", "Drive", "Pixel", "Film", "USB"] {
            assert!(shown.contains(&name.to_owned()), "{name} {shown:?}");
        }
        assert!(!shown.contains(&"Lulo Laptop".to_owned()));

        let mut settings = defaults.clone();
        settings.sidebar.show_connected_servers = false;
        assert!(!all(&settings).contains(&"Docs".to_owned()));
        let mut settings = defaults.clone();
        settings.sidebar.show_cloud_storage = false;
        assert!(!all(&settings).contains(&"Drive".to_owned()));
        let mut settings = defaults.clone();
        settings.sidebar.show_cds_dvds_and_ios_devices = false;
        let shown = all(&settings);
        assert!(!shown.contains(&"Pixel".to_owned()));
        assert!(!shown.contains(&"Film".to_owned()));
        assert!(shown.contains(&"USB".to_owned()));
        let mut settings = defaults.clone();
        settings.sidebar.show_external_disks = false;
        let shown = all(&settings);
        assert!(!shown.contains(&"USB".to_owned()));
        assert!(shown.contains(&"Docs".to_owned()));
        let mut settings = defaults.clone();
        settings.sidebar.show_computer = true;
        settings.sidebar.show_home = true;
        let shown = all(&settings);
        assert!(shown.contains(&"Lulo Laptop".to_owned()));
        assert!(shown.contains(&"jake".to_owned()));
    }

    #[test]
    #[cfg(not(windows))]
    fn shared_and_media_checkboxes_add_their_folders() {
        let home = std::env::temp_dir().join(format!(
            "rmac-sidebar-media-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        for folder in ["Public", "Videos", "Music", "Pictures"] {
            std::fs::create_dir_all(home.join(folder)).unwrap();
        }
        let words = rmac_locale::FileVocabulary::for_locale("en_US.UTF-8");
        let system = SidebarSystem::default();
        let mut settings = FinderSettings::default();
        let shown = names(&build_sections(
            &home,
            &[],
            &[],
            &[],
            &words,
            &settings,
            &system,
        ));
        assert!(shown.contains(&"Shared".to_owned()));
        for name in ["Movies", "Music", "Pictures"] {
            assert!(
                !shown.contains(&name.to_owned()),
                "{name} is off by default"
            );
        }
        settings.sidebar.show_shared = false;
        settings.sidebar.show_movies = true;
        settings.sidebar.show_music = true;
        settings.sidebar.show_pictures = true;
        let shown = names(&build_sections(
            &home,
            &[],
            &[],
            &[],
            &words,
            &settings,
            &system,
        ));
        assert!(!shown.contains(&"Shared".to_owned()));
        for name in ["Movies", "Music", "Pictures"] {
            assert!(shown.contains(&name.to_owned()), "{name} {shown:?}");
        }
        std::fs::remove_dir_all(&home).ok();
    }
}
