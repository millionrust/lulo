//! Stable first-party desktop identities shared by UI and system services.

pub const FILES: &str = "org.rmac.Files";
pub const TERMINAL: &str = "org.rmac.Terminal";
pub const NOTES: &str = "org.rmac.Notes";
pub const TEXT_EDITOR: &str = "org.rmac.TextEditor";
pub const SYSTEM_MONITOR: &str = "org.rmac.SystemMonitor";
pub const APP_DRAWER: &str = "org.rmac.AppDrawer";
pub const SYSTEM_SETTINGS: &str = "org.rmac.SystemSettings";
pub const CALCULATOR: &str = "org.rmac.Calculator";
pub const PREVIEW: &str = "org.rmac.Preview";
pub const CLOCK: &str = "org.rmac.Clock";
pub const WEATHER: &str = "org.rmac.Weather";
pub const PLAYER: &str = "org.rmac.Player";
/// The Open panel (`crates/rmac-file-chooser`, `org.freedesktop.impl.portal.FileChooser`'s
/// `OpenFile`). Not a launchable app — never added to `ALL` — but its window
/// still needs a stable identity for [`is_transient_panel`].
pub const FILE_CHOOSER: &str = "org.rmac.FileChooser";
/// The Save panel; a separate app id so niri can clip it at the sheet radius
/// (docs/decisions/0012-file-chooser-portal.md).
pub const FILE_CHOOSER_SAVE: &str = "org.rmac.FileChooser.Save";

pub const ALL: [&str; 12] = [
    FILES,
    TERMINAL,
    NOTES,
    TEXT_EDITOR,
    SYSTEM_MONITOR,
    APP_DRAWER,
    SYSTEM_SETTINGS,
    CALCULATOR,
    PREVIEW,
    CLOCK,
    WEATHER,
    PLAYER,
];

/// Stable user-facing native window title for a first-party desktop identity.
///
/// Custom in-window chrome may show a document or location title, but every
/// toplevel still publishes this non-empty base title to the compositor.
pub fn window_title(app_id: &str) -> Option<&'static str> {
    match app_id {
        FILES => Some("Files"),
        TERMINAL => Some("Terminal"),
        NOTES => Some("Notes"),
        TEXT_EDITOR => Some("Text Editor"),
        SYSTEM_MONITOR => Some("System Monitor"),
        APP_DRAWER => Some("Apps"),
        SYSTEM_SETTINGS => Some("Settings"),
        CALCULATOR => Some("Calculator"),
        PREVIEW => Some("Preview"),
        CLOCK => Some("Clock"),
        WEATHER => Some("Weather"),
        PLAYER => Some("Media Player"),
        _ => None,
    }
}

/// Only applications that own local document journeys may ask the rmac
/// notification service to open one reviewed document target.
pub fn is_document_application(app_id: &str) -> bool {
    matches!(app_id, FILES | NOTES | TEXT_EDITOR)
}

/// A window that borrows its owner's spot in the menu bar rather than being
/// switched to as an application in its own right. On the Mac the Open/Save
/// panel is a sheet or a panel of the app that asked for it — never a
/// separate frontmost application — so its own window taking niri's focus
/// must not make the menu bar switch away from (and clear) the app that
/// opened it (OTHER-04, docs/parity.md).
pub fn is_transient_panel(app_id: &str) -> bool {
    matches!(app_id, FILE_CHOOSER | FILE_CHOOSER_SAVE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn identities_are_unique_reverse_domain_desktop_ids() {
        assert_eq!(
            ALL.iter().copied().collect::<BTreeSet<_>>().len(),
            ALL.len()
        );
        assert!(ALL.iter().all(|identity| {
            identity.starts_with("org.rmac.")
                && identity
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        }));
    }

    #[test]
    fn document_authority_is_an_exact_allow_list() {
        assert!(is_document_application(FILES));
        assert!(is_document_application(NOTES));
        assert!(is_document_application(TEXT_EDITOR));
        assert!(!is_document_application(TERMINAL));
        assert!(!is_document_application("org.rmac.TextEditor.Forged"));
        assert!(!is_document_application("org.example.TextEditor"));
    }

    #[test]
    fn every_first_party_identity_has_a_non_empty_native_window_title() {
        for app_id in ALL {
            assert!(window_title(app_id).is_some_and(|title| !title.trim().is_empty()));
        }
        assert_eq!(window_title("org.example.Unknown"), None);
    }

    /// The Open/Save panel is a real window with its own compositor
    /// identity (`crates/rmac-file-chooser/src/main.rs`), but it is not a
    /// launchable application and must never join `ALL`'s switcher/dock
    /// enumeration.
    #[test]
    fn the_file_chooser_panel_is_transient_but_not_a_launchable_app() {
        assert!(is_transient_panel(FILE_CHOOSER));
        assert!(is_transient_panel(FILE_CHOOSER_SAVE));
        assert!(!is_transient_panel(FILES));
        assert!(!ALL.contains(&FILE_CHOOSER));
        assert!(!ALL.contains(&FILE_CHOOSER_SAVE));
    }
}
