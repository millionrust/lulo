//! Renderer-neutral Dock content, semantics, and original embedded assets.

use std::fmt;
use std::path::PathBuf;

use crate::{Item, Model, SpecialItem, SpecialItemKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinIcon {
    Application,
    Files,
    Downloads,
    TrashEmpty,
    TrashFull,
}

impl BuiltinIcon {
    /// A self-contained original vector asset. Keeping these embedded avoids
    /// install-path races while a Dock surface is starting or being restored.
    pub fn svg(self) -> &'static str {
        match self {
            Self::Application => include_str!("../assets/icons/application.svg"),
            Self::Files => include_str!("../assets/icons/files.svg"),
            Self::Downloads => include_str!("../assets/icons/downloads.svg"),
            Self::TrashEmpty => include_str!("../assets/icons/trash-empty.svg"),
            Self::TrashFull => include_str!("../assets/icons/trash-full.svg"),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum Icon {
    File(PathBuf),
    Builtin(BuiltinIcon),
}

impl fmt::Debug for Icon {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(_) => formatter.write_str("File(<private>)"),
            Self::Builtin(icon) => formatter.debug_tuple("Builtin").field(icon).finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryId {
    Application(String),
    Special(SpecialItemKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityIndicator {
    None,
    Running,
    Active,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub id: EntryId,
    pub label: String,
    /// Complete, path-free state for the renderer's accessible name.
    pub accessible_label: String,
    pub icon: Icon,
    /// False means the item remains visible but cannot be activated.
    pub enabled: bool,
    pub activity: ActivityIndicator,
    pub urgent: bool,
    /// Exact authoritative count. Rendering may visually abbreviate it, but
    /// must preserve the exact count in the accessible label.
    pub badge: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShelfContent {
    pub applications: Vec<Entry>,
    pub places: Vec<Entry>,
}

impl ShelfContent {
    pub fn project(model: &Model) -> Self {
        Self {
            applications: model.items.iter().map(application_entry).collect(),
            places: model.special_items.iter().map(special_entry).collect(),
        }
    }

    pub fn has_separator(&self) -> bool {
        !self.applications.is_empty() && !self.places.is_empty()
    }
}

fn application_entry(item: &Item) -> Entry {
    let activity = if item.active {
        ActivityIndicator::Active
    } else if item.running {
        ActivityIndicator::Running
    } else {
        ActivityIndicator::None
    };
    let enabled = item.running || item.launchable;
    Entry {
        id: EntryId::Application(item.id.clone()),
        label: item.name.clone(),
        accessible_label: application_accessible_label(item, enabled),
        icon: item
            .icon
            .clone()
            .map(Icon::File)
            .unwrap_or(Icon::Builtin(BuiltinIcon::Application)),
        enabled,
        activity,
        urgent: item.urgent,
        badge: None,
    }
}

fn application_accessible_label(item: &Item, enabled: bool) -> String {
    let mut parts = vec![item.name.clone()];
    if item.active {
        parts.push("active".into());
    } else if item.running {
        parts.push("running".into());
    }
    if item.urgent {
        parts.push("needs attention".into());
    }
    if !item.windows.is_empty() {
        parts.push(count_label(item.windows.len(), "window"));
    }
    if !enabled {
        parts.push("unavailable".into());
    }
    parts.join(", ")
}

fn special_entry(item: &SpecialItem) -> Entry {
    let (icon, badge, state) = match item.kind {
        SpecialItemKind::Files => (BuiltinIcon::Files, None, None),
        SpecialItemKind::Downloads => (BuiltinIcon::Downloads, None, None),
        SpecialItemKind::Trash => match item.item_count {
            Some(0) => (BuiltinIcon::TrashEmpty, None, Some("empty".into())),
            Some(count) => (
                BuiltinIcon::TrashFull,
                Some(count),
                Some(count_label(count, "item")),
            ),
            None => (BuiltinIcon::TrashEmpty, None, None),
        },
    };
    let mut accessible = vec![item.name.to_owned()];
    if let Some(state) = state {
        accessible.push(state);
    }
    if !item.available {
        accessible.push("unavailable".into());
    }
    Entry {
        id: EntryId::Special(item.kind),
        label: item.name.to_owned(),
        accessible_label: accessible.join(", "),
        icon: Icon::Builtin(icon),
        enabled: item.available,
        activity: ActivityIndicator::None,
        urgent: false,
        badge,
    }
}

fn count_label(count: usize, singular: &str) -> String {
    let suffix = if count == 1 { "" } else { "s" };
    format!("{count} {singular}{suffix}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{SpecialActivation, WindowItem};

    fn item(name: &str) -> Item {
        Item {
            id: format!("{}.desktop", name.to_lowercase()),
            name: name.into(),
            icon: None,
            pinned: true,
            running: false,
            active: false,
            urgent: false,
            launchable: true,
            windows: Vec::new(),
            launch: None,
        }
    }

    fn special(
        kind: SpecialItemKind,
        name: &'static str,
        available: bool,
        item_count: Option<usize>,
    ) -> SpecialItem {
        SpecialItem {
            kind,
            name,
            available,
            item_count,
            activation: SpecialActivation::Unavailable {
                kind,
                detail: "test".into(),
            },
        }
    }

    #[test]
    fn applications_have_exact_visual_and_accessible_state() {
        let mut terminal = item("Terminal");
        terminal.icon = Some(Path::new("/home/alex/.icons/private.svg").into());
        terminal.running = true;
        terminal.active = true;
        terminal.urgent = true;
        terminal.windows = vec![
            WindowItem {
                id: rmac_compositor::WindowId(1),
                title: None,
                focused: true,
                urgent: false,
                focus_timestamp: None,
            },
            WindowItem {
                id: rmac_compositor::WindowId(2),
                title: None,
                focused: false,
                urgent: true,
                focus_timestamp: None,
            },
        ];
        let mut unavailable = item("Missing");
        unavailable.launchable = false;
        let content = ShelfContent::project(&Model {
            items: vec![terminal, unavailable],
            ..Default::default()
        });

        assert_eq!(content.applications[0].activity, ActivityIndicator::Active);
        assert!(content.applications[0].enabled);
        assert!(content.applications[0].urgent);
        assert_eq!(
            content.applications[0].accessible_label,
            "Terminal, active, needs attention, 2 windows"
        );
        assert!(!format!("{:?}", content.applications[0].icon).contains("alex"));
        assert_eq!(
            content.applications[1].icon,
            Icon::Builtin(BuiltinIcon::Application)
        );
        assert!(!content.applications[1].enabled);
        assert_eq!(
            content.applications[1].accessible_label,
            "Missing, unavailable"
        );
    }

    #[test]
    fn places_stay_separate_and_trash_uses_authoritative_count() {
        let content = ShelfContent::project(&Model {
            items: vec![item("Finder")],
            special_items: vec![
                special(SpecialItemKind::Files, "Files", true, None),
                special(SpecialItemKind::Downloads, "Downloads", false, None),
                special(SpecialItemKind::Trash, "Trash", true, Some(7)),
            ],
            ..Default::default()
        });

        assert!(content.has_separator());
        assert_eq!(content.places[0].icon, Icon::Builtin(BuiltinIcon::Files));
        assert!(!content.places[1].enabled);
        assert_eq!(content.places[1].accessible_label, "Downloads, unavailable");
        assert_eq!(content.places[2].badge, Some(7));
        assert_eq!(
            content.places[2].icon,
            Icon::Builtin(BuiltinIcon::TrashFull)
        );
        assert_eq!(content.places[2].accessible_label, "Trash, 7 items");

        let empty = special(SpecialItemKind::Trash, "Trash", true, Some(0));
        let empty = special_entry(&empty);
        assert_eq!(empty.badge, None);
        assert_eq!(empty.icon, Icon::Builtin(BuiltinIcon::TrashEmpty));
        assert_eq!(empty.accessible_label, "Trash, empty");
    }

    #[test]
    fn embedded_icons_are_bounded_self_contained_vectors() {
        let icons = [
            BuiltinIcon::Application,
            BuiltinIcon::Files,
            BuiltinIcon::Downloads,
            BuiltinIcon::TrashEmpty,
            BuiltinIcon::TrashFull,
        ];
        for icon in icons {
            let svg = icon.svg();
            assert!(svg.starts_with("<svg"));
            assert!(svg.contains("viewBox=\"0 0 64 64\""));
            assert!(svg.len() < 16 * 1024);
            assert!(!svg.contains("<script"));
            assert!(!svg.contains("<image"));
            assert!(!svg.contains("href="));
            assert!(!svg.contains("<text"));
            assert!(!svg.contains("Gradient"));
            assert!(!svg.contains("<filter"));
        }
    }
}
