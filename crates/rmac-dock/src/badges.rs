//! Dock tile badges, progress bars and attention requests from applications.
//!
//! macOS apps set a Dock badge (Mail's unread count), a progress bar
//! (downloads) or request attention through `NSDockTile`/`NSApplication`.
//! Linux apps publish the same through the `com.canonical.Unity.LauncherEntry`
//! D-Bus signal `Update(app_uri, properties)`, which Thunderbird, Telegram,
//! Firefox downloads and others emit. This module keeps the protocol state;
//! the Dock only draws what a running app has published.

use std::collections::BTreeMap;

use crate::canonical_app_id;

/// One property update from a LauncherEntry signal. Absent fields keep their
/// previous value, as the protocol specifies.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LauncherUpdate {
    pub count: Option<i64>,
    pub count_visible: Option<bool>,
    pub progress: Option<f64>,
    pub progress_visible: Option<bool>,
    pub urgent: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LauncherEntry {
    pub count: i64,
    pub count_visible: bool,
    pub progress: f64,
    pub progress_visible: bool,
    pub urgent: bool,
}

impl LauncherEntry {
    /// The badge text, only for a visible positive count.
    pub fn badge(&self) -> Option<String> {
        (self.count_visible && self.count > 0).then(|| self.count.to_string())
    }

    /// Progress in 0..=1 while the app shows a progress bar.
    pub fn progress(&self) -> Option<f32> {
        (self.progress_visible && self.progress.is_finite())
            .then(|| self.progress.clamp(0.0, 1.0) as f32)
    }
}

/// Canonical Dock app ID for a LauncherEntry `app_uri`
/// (`application://org.mozilla.Thunderbird.desktop`).
pub fn app_id_from_uri(app_uri: &str) -> Option<String> {
    let id = app_uri.strip_prefix("application://")?;
    let id = canonical_app_id(id);
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// Every app's published state, keyed by canonical app ID.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LauncherEntries {
    entries: BTreeMap<String, LauncherEntry>,
}

/// At most this many apps are tracked, so a misbehaving sender cannot grow
/// the Dock's memory without bound.
pub const MAX_LAUNCHER_ENTRIES: usize = 256;

impl LauncherEntries {
    /// Apply one signal; returns true when anything the Dock draws changed.
    pub fn apply(&mut self, app_uri: &str, update: LauncherUpdate) -> bool {
        let Some(app_id) = app_id_from_uri(app_uri) else {
            return false;
        };
        if !self.entries.contains_key(&app_id) && self.entries.len() >= MAX_LAUNCHER_ENTRIES {
            return false;
        }
        let entry = self.entries.entry(app_id).or_default();
        let before = *entry;
        if let Some(count) = update.count {
            entry.count = count;
        }
        if let Some(visible) = update.count_visible {
            entry.count_visible = visible;
        }
        if let Some(progress) = update.progress.filter(|progress| progress.is_finite()) {
            entry.progress = progress;
        }
        if let Some(visible) = update.progress_visible {
            entry.progress_visible = visible;
        }
        if let Some(urgent) = update.urgent {
            entry.urgent = urgent;
        }
        before != *entry
    }

    pub fn get(&self, app_id: &str) -> Option<&LauncherEntry> {
        self.entries.get(&canonical_app_id(app_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_maps_to_the_canonical_dock_identity() {
        assert_eq!(
            app_id_from_uri("application://org.mozilla.Thunderbird.desktop").as_deref(),
            Some("org.mozilla.thunderbird")
        );
        assert_eq!(app_id_from_uri("file:///tmp/x.desktop"), None);
        assert_eq!(app_id_from_uri("application://"), None);
    }

    #[test]
    fn partial_updates_keep_earlier_properties() {
        let mut entries = LauncherEntries::default();
        assert!(entries.apply(
            "application://mail.desktop",
            LauncherUpdate {
                count: Some(3),
                count_visible: Some(true),
                ..Default::default()
            },
        ));
        assert_eq!(entries.get("mail").unwrap().badge().as_deref(), Some("3"));
        assert!(entries.apply(
            "application://mail.desktop",
            LauncherUpdate {
                progress: Some(0.25),
                progress_visible: Some(true),
                ..Default::default()
            },
        ));
        let mail = entries.get("mail.desktop").unwrap();
        assert_eq!(mail.badge().as_deref(), Some("3"));
        assert_eq!(mail.progress(), Some(0.25));
        // Repeating the same state changes nothing on screen.
        assert!(!entries.apply(
            "application://mail.desktop",
            LauncherUpdate {
                count: Some(3),
                ..Default::default()
            },
        ));
    }

    #[test]
    fn hidden_zero_and_invalid_values_draw_nothing() {
        let mut entries = LauncherEntries::default();
        entries.apply(
            "application://a.desktop",
            LauncherUpdate {
                count: Some(0),
                count_visible: Some(true),
                progress: Some(f64::NAN),
                progress_visible: Some(true),
                ..Default::default()
            },
        );
        let a = entries.get("a").unwrap();
        assert_eq!(a.badge(), None);
        assert_eq!(a.progress(), Some(0.0));
        entries.apply(
            "application://a.desktop",
            LauncherUpdate {
                count: Some(9),
                count_visible: Some(false),
                progress: Some(4.0),
                ..Default::default()
            },
        );
        let a = entries.get("a").unwrap();
        assert_eq!(a.badge(), None);
        assert_eq!(a.progress(), Some(1.0));
    }

    #[test]
    fn tracking_is_bounded() {
        let mut entries = LauncherEntries::default();
        for index in 0..MAX_LAUNCHER_ENTRIES + 10 {
            entries.apply(
                &format!("application://app{index}.desktop"),
                LauncherUpdate {
                    urgent: Some(true),
                    ..Default::default()
                },
            );
        }
        assert!(entries.get("app0").is_some());
        assert!(entries
            .get(&format!("app{}", MAX_LAUNCHER_ENTRIES + 5))
            .is_none());
    }
}
