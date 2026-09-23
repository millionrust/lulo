//! The Dock's "suggested and recent apps" section.
//!
//! Observed on macOS 26.2 with the default setting on
//! (docs/dock-behaviour-2026-09-23.md): apps that are not kept in the Dock
//! appear after a separator while they run and stay there, without a dot,
//! after they quit. The section holds three apps; launching another adds it
//! at the right end and the oldest app that is not running shrinks away.
//! Running apps are never evicted, so more than three may show while that
//! many are open. Dragging an app out of the section removes it.

use std::collections::BTreeSet;

use crate::{canonical_app_id, Item, Model};

/// Apps the section keeps when none of them is running.
pub const MAX_RECENT_APPLICATIONS: usize = 3;

/// Advance the persisted section order.
///
/// `previous` is the stored order, `pinned` the kept apps and `running` the
/// running apps that are not kept, in any order. Newly running apps join at
/// the end; kept apps leave; the oldest apps that are not running are evicted
/// until at most [`MAX_RECENT_APPLICATIONS`] remain (running apps stay).
pub fn advance(previous: &[String], pinned: &[String], running: &[String]) -> Vec<String> {
    let pinned: BTreeSet<String> = pinned.iter().map(|id| canonical_app_id(id)).collect();
    let running_set: BTreeSet<String> = running.iter().map(|id| canonical_app_id(id)).collect();
    let mut seen = BTreeSet::new();
    let mut order: Vec<String> = Vec::new();
    for id in previous.iter().chain(running.iter()) {
        let canonical = canonical_app_id(id);
        if canonical.is_empty() || pinned.contains(&canonical) || !seen.insert(canonical) {
            continue;
        }
        order.push(id.clone());
    }
    while order.len() > MAX_RECENT_APPLICATIONS {
        let Some(oldest) = order
            .iter()
            .position(|id| !running_set.contains(&canonical_app_id(id)))
        else {
            break;
        };
        order.remove(oldest);
    }
    order
}

/// Remove one app from the section (dragged out of the Dock).
pub fn remove(previous: &[String], app_id: &str) -> Vec<String> {
    let canonical = canonical_app_id(app_id);
    previous
        .iter()
        .filter(|id| canonical_app_id(id) != canonical)
        .cloned()
        .collect()
}

/// Serialise the section for its state file: one application ID per line.
pub fn encode(order: &[String]) -> String {
    order
        .iter()
        .filter(|id| !id.trim().is_empty() && !id.contains('\n'))
        .map(|id| format!("{id}\n"))
        .collect()
}

/// Read the state file back, tolerating blank lines and bounding its size.
pub fn decode(contents: &str) -> Vec<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= 512)
        .take(64)
        .map(str::to_owned)
        .collect()
}

impl Model {
    /// The app IDs of running apps that are not kept in the Dock.
    pub fn running_unpinned(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|item| !item.pinned && item.running)
            .map(|item| item.id.clone())
            .collect()
    }

    /// Order the not-kept section by `recent` and add the recent apps that
    /// have quit, without a running dot. Only installed apps are added; a
    /// running app keeps its place even if it is missing from `recent`.
    pub fn with_recent_applications(
        mut self,
        recent: &[String],
        catalog: &[rmac_apps::Application],
    ) -> Self {
        let applications = crate::catalog_index(catalog);
        let pinned_len = self.items.iter().take_while(|item| item.pinned).count();
        let mut section: Vec<Item> = self.items.split_off(pinned_len);
        let represented: BTreeSet<String> = self
            .items
            .iter()
            .chain(section.iter())
            .map(|item| canonical_app_id(&item.id))
            .collect();
        for id in recent {
            let canonical = canonical_app_id(id);
            if represented.contains(&canonical) {
                continue;
            }
            if let Some(application) = applications.get(&canonical) {
                section.push(crate::build_item(
                    &application.id,
                    Some(*application),
                    false,
                    Vec::new(),
                ));
            }
        }
        let position = |item: &Item| -> usize {
            let canonical = canonical_app_id(&item.id);
            recent
                .iter()
                .position(|id| canonical_app_id(id) == canonical)
                .unwrap_or(usize::MAX)
        };
        // Stable: running apps not yet recorded keep their name order.
        section.sort_by_key(position);
        self.items.extend(section);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn a_new_app_joins_at_the_end_and_evicts_the_oldest_closed_one() {
        // Measured: Chrome (running), Chrome, Chrome; Calculator launched.
        let previous = ids(&["a.desktop", "b.desktop", "c.desktop"]);
        let next = advance(&previous, &[], &ids(&["a.desktop", "calculator.desktop"]));
        assert_eq!(next, ids(&["a.desktop", "c.desktop", "calculator.desktop"]));
    }

    #[test]
    fn quit_apps_stay_and_running_apps_are_never_evicted() {
        let previous = ids(&["a", "b"]);
        // b quit: it stays.
        assert_eq!(advance(&previous, &[], &ids(&["a"])), ids(&["a", "b"]));
        // Five running apps all show.
        let running = ids(&["a", "b", "c", "d", "e"]);
        assert_eq!(advance(&previous, &[], &running), running);
    }

    #[test]
    fn kept_apps_leave_the_section_and_duplicates_collapse() {
        let previous = ids(&["files.desktop", "Files", "notes"]);
        let next = advance(&previous, &ids(&["notes.desktop"]), &[]);
        assert_eq!(next, ids(&["files.desktop"]));
    }

    fn application(id: &str, name: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            keywords: Vec::new(),
            source: std::path::PathBuf::from(format!("/apps/{id}")),
            icon: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: id.trim_end_matches(".desktop").into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    #[test]
    fn quit_recent_apps_show_without_a_dot_in_section_order() {
        let catalog = [
            application("files.desktop", "Files"),
            application("clock.desktop", "Clock"),
            application("weather.desktop", "Weather"),
        ];
        let compositor = rmac_compositor::Snapshot {
            windows: vec![rmac_compositor::Window {
                id: rmac_compositor::WindowId(1),
                title: None,
                app_id: Some("weather".into()),
                pid: None,
                workspace: None,
                focused: false,
                floating: true,
                urgent: false,
                focus_timestamp: None,
                layout: Default::default(),
            }],
            ..Default::default()
        };
        let model = Model::build(
            &[rmac_shell_settings::AppId("files.desktop".into())],
            &Default::default(),
            &catalog,
            &compositor,
        )
        .with_recent_applications(
            &ids(&[
                "clock.desktop",
                "weather.desktop",
                "gone.desktop",
                "files.desktop",
            ]),
            &catalog,
        );
        let names: Vec<_> = model.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["Files", "Clock", "Weather"]);
        assert!(!model.items[1].running && !model.items[1].pinned);
        assert!(model.items[2].running);
        assert!(matches!(
            model.activate("clock.desktop"),
            crate::Activation::Launch { .. }
        ));
    }

    #[test]
    fn removing_and_state_file_round_trip() {
        let order = ids(&["a.desktop", "b.desktop"]);
        assert_eq!(remove(&order, "A"), ids(&["b.desktop"]));
        assert_eq!(decode(&encode(&order)), order);
        assert_eq!(decode("\n  x \n\n"), ids(&["x"]));
        assert_eq!(encode(&ids(&["bad\nid", " "])), "");
    }
}
