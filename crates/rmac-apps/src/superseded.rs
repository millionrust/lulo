//! Apps superseded, in the Lulo OS session, by a first-party equivalent.
//!
//! Ubuntu's Files (`org.gnome.Nautilus`), and a handful of other GNOME
//! utilities that do exactly the job a shipped rmac app already does, stay
//! installed and untouched -- a plain Ubuntu/GNOME session never reads this
//! list, and nothing here uninstalls or disables the package. Inside Lulo OS,
//! the App Drawer (Launchpad), Spotlight, and the Dock's quit-app suggestion
//! strip hide the superseded desktop entry from browsing so it does not sit
//! next to its rmac replacement and confuse the picture -- the way macOS
//! keeps extra vendor utilities out of Launchpad's default view. "Open With"
//! and any command that names the app directly (including `nautilus` itself)
//! still find it: only `discover_for_browsing` and the Dock's recent-apps
//! section consult this list; `discover()` and file-association lookups stay
//! exact and complete.

use super::*;

/// Installed path for the packaged list
/// (`packaging/rmac-apps/superseded-apps.list`, staged to this path by
/// `rmac-apps`). A fixed system path is unusual for a library that otherwise
/// follows XDG search order, but this list is packaging state curated by
/// rmac, not third-party application data, so it ships next to the package
/// manifest instead.
const SUPERSEDED_LIST_PATH: &str = "/usr/share/rmac/superseded-apps.list";

/// Compiled-in fallback mirroring the packaged list, used only when that file
/// is absent -- a development checkout, an incomplete install, or the macOS
/// build -- so hiding still behaves predictably before packaging runs. Keep
/// this in sync with `packaging/rmac-apps/superseded-apps.list`; a packaging
/// test checks the two agree.
pub(super) const DEFAULT_SUPERSEDED: &[(&str, &str)] = &[
    ("org.gnome.Nautilus.desktop", identity::FILES),
    ("org.gnome.TextEditor.desktop", identity::TEXT_EDITOR),
    ("gnome-system-monitor.desktop", identity::SYSTEM_MONITOR),
    ("org.gnome.SystemMonitor.desktop", identity::SYSTEM_MONITOR),
    ("gnome-calculator.desktop", identity::CALCULATOR),
    ("org.gnome.Calculator.desktop", identity::CALCULATOR),
    ("org.gnome.Loupe.desktop", identity::PREVIEW),
    ("org.gnome.Evince.desktop", identity::PREVIEW),
    ("org.gnome.Papers.desktop", identity::PREVIEW),
    ("org.gnome.Terminal.desktop", identity::TERMINAL),
    ("org.gnome.Ptyxis.desktop", identity::TERMINAL),
    ("foot.desktop", identity::TERMINAL),
    ("footclient.desktop", identity::TERMINAL),
    ("foot-server.desktop", identity::TERMINAL),
    ("org.gnome.clocks.desktop", identity::CLOCK),
    ("org.gnome.Weather.desktop", identity::WEATHER),
];

/// Desktop-entry IDs (e.g. `org.gnome.Nautilus.desktop`) hidden from
/// browsing/search surfaces because a first-party Lulo OS app already does
/// the same job, mapped to that app's identity. Reads the packaged list
/// first; falls back to the compiled default when that file is missing or
/// unreadable.
pub fn superseded_desktop_ids() -> HashMap<String, String> {
    read_superseded_list(Path::new(SUPERSEDED_LIST_PATH)).unwrap_or_else(default_superseded_map)
}

pub(super) fn default_superseded_map() -> HashMap<String, String> {
    DEFAULT_SUPERSEDED
        .iter()
        .map(|(id, equivalent)| (id.to_string(), equivalent.to_string()))
        .collect()
}

pub(super) fn read_superseded_list(path: &Path) -> Option<HashMap<String, String>> {
    let contents = std::fs::read_to_string(path).ok()?;
    Some(parse_superseded_list(&contents))
}

/// Parses `desktop-id=rmac-app-id` lines, ignoring blank lines and `#`
/// comments. A line missing the separator, or naming an empty desktop ID, is
/// skipped rather than failing the whole list -- one malformed line must not
/// stop every other entry from hiding.
pub(super) fn parse_superseded_list(contents: &str) -> HashMap<String, String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (id, equivalent) = line.split_once('=')?;
            let id = id.trim();
            let equivalent = equivalent.trim();
            if id.is_empty() || !id.ends_with(".desktop") || equivalent.is_empty() {
                return None;
            }
            Some((id.to_string(), equivalent.to_string()))
        })
        .collect()
}

/// Removes catalog entries superseded by a first-party app. Meant for
/// browsing and search surfaces only: callers that resolve "Open With"
/// handlers, running windows, or an explicit launch by ID must keep using the
/// full, unfiltered catalog from [`discover`].
pub fn hide_superseded(
    catalog: Vec<Application>,
    superseded: &HashMap<String, String>,
) -> Vec<Application> {
    catalog
        .into_iter()
        .filter(|application| !superseded.contains_key(&application.id))
        .collect()
}

/// [`discover`] filtered by the current superseded-app list, for the App
/// Drawer, Spotlight, and any other pure browsing/search surface.
pub fn discover_for_browsing() -> io::Result<Vec<Application>> {
    let catalog = discover()?;
    Ok(hide_superseded(catalog, &superseded_desktop_ids()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn application(id: &str) -> Application {
        Application {
            id: id.into(),
            name: id.into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from(format!("/usr/share/applications/{id}")),
            icon: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            launch: LaunchSpec::Command {
                program: "/usr/bin/true".into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    #[test]
    fn parses_ids_and_skips_comments_blanks_and_malformed_lines() {
        let parsed = parse_superseded_list(
            "# comment\n\n\
             org.gnome.Nautilus.desktop=org.rmac.Files\n  \
             org.gnome.Ptyxis.desktop = org.rmac.Terminal \n\
             missing-equals-sign\n\
             =org.rmac.Files\n\
             org.gnome.NoTarget.desktop=\n\
             not-a-desktop-id=org.rmac.Files\n",
        );
        assert_eq!(
            parsed,
            HashMap::from([
                (
                    "org.gnome.Nautilus.desktop".to_string(),
                    "org.rmac.Files".to_string()
                ),
                (
                    "org.gnome.Ptyxis.desktop".to_string(),
                    "org.rmac.Terminal".to_string()
                ),
            ])
        );
    }

    #[test]
    fn missing_packaged_list_falls_back_to_compiled_default() {
        assert!(read_superseded_list(Path::new("/nonexistent/superseded-apps.list")).is_none());
        assert_eq!(superseded_desktop_ids(), default_superseded_map());
    }

    #[test]
    fn hide_superseded_removes_only_listed_ids() {
        let catalog = vec![
            application("org.gnome.Nautilus.desktop"),
            application("org.rmac.Files.desktop"),
            application("org.example.Other.desktop"),
        ];
        let superseded = HashMap::from([(
            "org.gnome.Nautilus.desktop".to_string(),
            "org.rmac.Files".to_string(),
        )]);
        let visible = hide_superseded(catalog, &superseded);
        assert_eq!(
            visible
                .iter()
                .map(|app| app.id.as_str())
                .collect::<Vec<_>>(),
            ["org.rmac.Files.desktop", "org.example.Other.desktop"],
        );
    }

    #[test]
    fn default_list_only_names_apps_lulo_os_actually_ships() {
        for (_, equivalent) in DEFAULT_SUPERSEDED {
            assert!(
                identity::ALL.contains(equivalent),
                "{equivalent} is not a shipped rmac app identity"
            );
        }
    }

    /// Foot (and its client/server split) is a duplicate terminal emulator,
    /// not a system utility with no rmac equivalent -- see the reference
    /// install incident where all three showed up next to Terminal in the
    /// App Drawer. Pinned here so DEFAULT_SUPERSEDED cannot silently drift
    /// from `packaging/rmac-apps/superseded-apps.list` (kept in sync by a
    /// packaging test in scripts/test_application_package.py).
    #[test]
    fn foot_terminal_variants_are_superseded_by_terminal() {
        let map = default_superseded_map();
        for foot_id in ["foot.desktop", "footclient.desktop", "foot-server.desktop"] {
            assert_eq!(
                map.get(foot_id).map(String::as_str),
                Some(identity::TERMINAL)
            );
        }
    }
}
