//! Sidebar section/place construction, shared by window startup
//! (`startup.rs`) and by [`FinderView::rebuild_sidebar_sections`], which
//! the Settings window's Sidebar and Tags tabs call into (via
//! `settings::update`'s broadcast) whenever their checkboxes change, so
//! every open window's sidebar reflects a change immediately — matching
//! the Mac's own "live" preferences contract for FILES-35.

use super::settings::FinderSettings;
use super::*;

/// Builds every sidebar section exactly as `startup.rs` once did inline,
/// now filtered by the Settings window's Sidebar and Tags tabs. Pure, so
/// both the initial build and every later rebuild share one code path.
pub(super) fn build_sections(
    home: &Path,
    mounts: &[rmac_mounts::Mount],
    favourite_extras: &[PathBuf],
    favourite_order: &[FavouriteKey],
    file_words: &rmac_locale::FileVocabulary,
    settings: &FinderSettings,
) -> Vec<Section> {
    let p = |name: &str, path: PathBuf, icon: &'static str, tint: Hsla, kind: PlaceKind| Place {
        name: name.to_string().into(),
        path,
        icon,
        tint,
        kind,
    };

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

    // Real mounted volumes: ejectable ones are Settings' "External disks",
    // the rest are its "Hard disks" (the home volume's own entry below is
    // never filtered this way — it is the Sidebar tab's separate "Home").
    let mut locations: Vec<Place> = rmac_finder::places::standard_locations(home)
        .into_iter()
        .map(from_spec)
        .filter(|place| settings.sidebar.show_home || place.path != home)
        .collect();
    let mounts_filtered = mounts.iter().filter(|mount| {
        if mount.ejectable {
            settings.sidebar.show_external_disks
        } else {
            settings.sidebar.show_hard_disks
        }
    });
    locations.extend(mounts_filtered.cloned().map(|mount| {
        p(
            &mount.name,
            mount.path,
            "icons/hard-drive.svg",
            drive_gray(),
            if mount.ejectable {
                PlaceKind::Volume
            } else {
                PlaceKind::Item
            },
        )
    }));

    let mut prominent = Vec::new();
    if settings.sidebar.show_recents {
        prominent.push(p(
            "Recents",
            PathBuf::new(),
            "icons/clock.svg",
            accent(),
            PlaceKind::Recents,
        ));
    }
    if let Some(shared) = rmac_finder::places::shared_folder(home) {
        prominent.push(from_spec(shared));
    }

    let mut favorites = Vec::new();
    if settings.sidebar.show_applications {
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
                "Desktop" => settings.sidebar.show_desktop,
                "Documents" => settings.sidebar.show_documents,
                "Downloads" => settings.sidebar.show_downloads,
                _ => true,
            })
            .map(from_spec),
    );
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
    if cfg!(target_os = "linux") && settings.sidebar.show_bin {
        locations.push(p(
            file_words.bin(),
            PathBuf::new(),
            "icons/trash-2.svg",
            accent(),
            PlaceKind::Trash,
        ));
    }

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
    if settings.sidebar.show_tags {
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
    /// Rebuilds the sidebar from the current settings — called once at
    /// startup and again every time Finder ▸ Settings… ▸ Sidebar or ▸ Tags
    /// changes (`settings::update`'s broadcast to every open window).
    pub(super) fn rebuild_sidebar_sections(&mut self, cx: &mut Context<Self>) {
        self.sections = build_sections(
            &self.home,
            &self.mounts,
            &self.favourite_extras,
            &self.favourite_order,
            &self.file_words,
            &super::settings::current(),
        );
        cx.notify();
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

        let shown = names(&build_sections(home, &[], &[], &[], &words, &settings));
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

        let sections = build_sections(home, &[], &[], &[], &words, &settings);
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

        let shown = names(&build_sections(home, &[], &[], &[], &words, &settings));
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
        let shown = names(&build_sections(home, &mounts, &[], &[], &words, &settings));
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
}
