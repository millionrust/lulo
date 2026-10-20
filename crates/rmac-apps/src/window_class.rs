//! Which desktop entry a window belongs to when its app id is not the
//! entry's desktop ID.
//!
//! A native Wayland window's app id is normally its desktop ID, but an
//! XWayland window reports its X11 `WM_CLASS` ("Navigator", "jetbrains-idea",
//! "code-url-handler"), which the entry names with `StartupWMClass=`, and some
//! apps set a class that differs from their file name. The Mac never shows a
//! running app without its own Dock tile, name and icon, so every entry's
//! `StartupWMClass` is remembered here as the catalog is read, and surfaces
//! that only know a window's app id (the Dock, the menu bar) resolve it with
//! [`desktop_id_for_window_class`].

use std::collections::HashMap;
use std::sync::RwLock;

static CLASSES: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

fn key(class: &str) -> String {
    class.trim().trim_end_matches(".desktop").to_lowercase()
}

/// Remember that windows of `class` belong to the entry `desktop_id`.
pub fn register_window_class(class: &str, desktop_id: &str) {
    let class = key(class);
    if class.is_empty() || desktop_id.is_empty() {
        return;
    }
    // An entry whose own ID is the class needs no alias.
    if key(desktop_id) == class {
        return;
    }
    if let Ok(mut classes) = CLASSES.write() {
        classes
            .get_or_insert_with(HashMap::new)
            .entry(class)
            .or_insert_with(|| desktop_id.to_owned());
    }
}

/// The desktop ID whose entry claims windows of `app_id` through
/// `StartupWMClass` (matched without case, as GNOME Shell does), if the
/// catalog read so far has one.
pub fn desktop_id_for_window_class(app_id: &str) -> Option<String> {
    CLASSES.read().ok()?.as_ref()?.get(&key(app_id)).cloned()
}

/// Whether the entry `values` (its `[Desktop Entry]` group) claims windows
/// of `app_id`.
pub(crate) fn entry_claims(values: &HashMap<String, String>, app_id: &str) -> bool {
    values
        .get("StartupWMClass")
        .is_some_and(|class| key(class) == key(app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_class_resolves_to_the_entry_that_claims_it() {
        register_window_class("Navigator-Test", "firefox-test.desktop");
        assert_eq!(
            desktop_id_for_window_class("navigator-test"),
            Some("firefox-test.desktop".to_owned())
        );
        // An entry named after its own class adds nothing.
        register_window_class("foot-test", "foot-test.desktop");
        assert_eq!(desktop_id_for_window_class("foot-test"), None);
        let values = HashMap::from([("StartupWMClass".to_owned(), "Code".to_owned())]);
        assert!(entry_claims(&values, "code"));
        assert!(!entry_claims(&values, "codium"));
    }
}
