//! Dock item projection and interaction authority.

use super::*;

impl Model {
    pub fn build(
        pinned: &[rmac_shell_settings::AppId],
        settings: &rmac_shell_settings::DockSettings,
        catalog: &[rmac_apps::Application],
        compositor: &rmac_compositor::Snapshot,
    ) -> Self {
        Self::build_inner(pinned, settings, catalog, compositor, None)
    }

    pub fn build_with_places(
        pinned: &[rmac_shell_settings::AppId],
        settings: &rmac_shell_settings::DockSettings,
        catalog: &[rmac_apps::Application],
        compositor: &rmac_compositor::Snapshot,
        places: &rmac_places::Snapshot,
    ) -> Self {
        Self::build_inner(pinned, settings, catalog, compositor, Some(places))
    }

    fn build_inner(
        pinned: &[rmac_shell_settings::AppId],
        settings: &rmac_shell_settings::DockSettings,
        catalog: &[rmac_apps::Application],
        compositor: &rmac_compositor::Snapshot,
        places: Option<&rmac_places::Snapshot>,
    ) -> Self {
        let applications = catalog_index(catalog);
        let mut windows = window_groups(compositor);
        let mut items = Vec::new();
        let mut represented = BTreeSet::new();

        for pinned_id in pinned {
            let canonical = canonical_app_id(&pinned_id.0);
            if canonical.is_empty() || !represented.insert(canonical.clone()) {
                continue;
            }
            let application = applications.get(&canonical).copied();
            let grouped = windows.remove(&canonical).unwrap_or_default();
            items.push(build_item(&pinned_id.0, application, true, grouped));
        }

        let mut running: Vec<_> = windows
            .into_iter()
            .filter(|(canonical, grouped)| {
                !canonical.is_empty()
                    && !grouped.is_empty()
                    && represented.insert(canonical.clone())
            })
            .map(|(canonical, grouped)| {
                let application = applications.get(&canonical).copied();
                let source_id = application
                    .map(|application| application.id.clone())
                    .or_else(|| grouped.first().map(|window| window.app_id.clone()))
                    .unwrap_or(canonical);
                build_item(&source_id, application, false, grouped)
            })
            .collect();
        // Focus changes must never move a button out from under the pointer.
        // A renderer/runtime may later retain launch order, but the stateless
        // model uses this deterministic identity order rather than recency.
        running.sort_by_key(|item| (item.name.to_lowercase(), item.id.clone()));
        items.extend(running);

        Self {
            items,
            special_items: places.map(project_special_items).unwrap_or_default(),
            repeated_click: settings.repeated_click,
        }
    }

    pub fn activate_special(&self, kind: SpecialItemKind) -> SpecialActivation {
        self.special_items
            .iter()
            .find(|item| item.kind == kind)
            .map(|item| item.activation.clone())
            .unwrap_or_else(|| SpecialActivation::Unavailable {
                kind,
                detail: "the place is not present in the Dock".into(),
            })
    }

    pub fn special_context_menu(&self, kind: SpecialItemKind) -> Option<SpecialContextMenu> {
        let item = self.special_items.iter().find(|item| item.kind == kind)?;
        Some(SpecialContextMenu {
            kind,
            open: item.activation.clone(),
            empty_trash: match (kind, item.available, item.item_count) {
                (SpecialItemKind::Trash, true, Some(item_count)) if item_count > 0 => {
                    Some(SpecialContextAction::EmptyTrash {
                        expected_item_count: item_count,
                    })
                }
                _ => None,
            },
        })
    }

    pub fn activate(&self, app_id: &str) -> Activation {
        let canonical = canonical_app_id(app_id);
        let Some(item) = self
            .items
            .iter()
            .find(|item| canonical_app_id(&item.id) == canonical)
        else {
            return Activation::Unavailable {
                app_id: app_id.to_owned(),
                detail: "application is not present in the Dock".into(),
            };
        };
        if item.windows.is_empty() {
            return match &item.launch {
                Some(spec) => Activation::Launch {
                    app_id: item.id.clone(),
                    spec: spec.clone(),
                },
                None => Activation::Unavailable {
                    app_id: item.id.clone(),
                    detail: "application is not installed".into(),
                },
            };
        }

        let focused = item.windows.iter().position(|window| window.focused);
        let target = match focused {
            None => item.windows.first().map(|window| window.id),
            Some(index) => match self.repeated_click {
                rmac_shell_settings::RepeatedClickBehavior::CycleWindows
                    if item.windows.len() > 1 =>
                {
                    Some(item.windows[(index + 1) % item.windows.len()].id)
                }
                rmac_shell_settings::RepeatedClickBehavior::CycleWindows
                | rmac_shell_settings::RepeatedClickBehavior::DoNothing => None,
                rmac_shell_settings::RepeatedClickBehavior::HideApplication => {
                    return Activation::Unavailable {
                        app_id: item.id.clone(),
                        detail: "niri does not expose application hiding".into(),
                    };
                }
            },
        };
        target
            .map(Activation::FocusWindow)
            .unwrap_or(Activation::NoAction)
    }

    pub fn context_menu(&self, app_id: &str) -> Option<ContextMenu> {
        let canonical = canonical_app_id(app_id);
        let index = self
            .items
            .iter()
            .position(|item| canonical_app_id(&item.id) == canonical)?;
        let item = &self.items[index];
        let windows = item
            .windows
            .iter()
            .map(|window| {
                let title = window
                    .title
                    .clone()
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| item.name.clone());
                WindowMenu {
                    id: window.id,
                    title,
                    focused: window.focused,
                    urgent: window.urgent,
                    focus: ContextAction::FocusWindow {
                        app_id: item.id.clone(),
                        window: window.id,
                    },
                    close: ContextAction::CloseWindow {
                        app_id: item.id.clone(),
                        window: window.id,
                    },
                }
            })
            .collect();
        let pinned_index = self.items[..index]
            .iter()
            .filter(|item| item.pinned)
            .count();
        let pinned_count = self.items.iter().filter(|item| item.pinned).count();
        Some(ContextMenu {
            app_id: item.id.clone(),
            application_name: item.name.clone(),
            launch_new: item.launch.clone().map(|spec| ContextAction::LaunchNew {
                app_id: item.id.clone(),
                spec,
            }),
            windows,
            pin: if item.pinned {
                PinCommand::Unpin {
                    app_id: item.id.clone(),
                }
            } else {
                PinCommand::Pin {
                    app_id: item.id.clone(),
                }
            },
            move_left: (item.pinned && pinned_index > 0).then(|| PinCommand::Move {
                app_id: item.id.clone(),
                direction: MoveDirection::Left,
            }),
            move_right: (item.pinned && pinned_index + 1 < pinned_count).then(|| {
                PinCommand::Move {
                    app_id: item.id.clone(),
                    direction: MoveDirection::Right,
                }
            }),
        })
    }
}

pub(super) fn project_special_items(places: &rmac_places::Snapshot) -> Vec<SpecialItem> {
    let files = SpecialItem {
        kind: SpecialItemKind::Files,
        name: "Files",
        available: places.home.exists,
        item_count: None,
        activation: if places.home.exists {
            SpecialActivation::OpenDirectory {
                kind: SpecialItemKind::Files,
                path: places.home.path.clone(),
            }
        } else {
            SpecialActivation::Unavailable {
                kind: SpecialItemKind::Files,
                detail: "the home directory is unavailable".into(),
            }
        },
    };
    let downloads_available = places.downloads.exists && places.downloads.path != places.home.path;
    let downloads = SpecialItem {
        kind: SpecialItemKind::Downloads,
        name: "Downloads",
        available: downloads_available,
        item_count: None,
        activation: if downloads_available {
            SpecialActivation::OpenDirectory {
                kind: SpecialItemKind::Downloads,
                path: places.downloads.path.clone(),
            }
        } else {
            SpecialActivation::Unavailable {
                kind: SpecialItemKind::Downloads,
                detail: if places.downloads.path == places.home.path {
                    "the Downloads user directory is disabled"
                } else {
                    "the Downloads directory is unavailable"
                }
                .into(),
            }
        },
    };
    let trash = SpecialItem {
        kind: SpecialItemKind::Trash,
        name: "Trash",
        available: places.trash.available,
        item_count: places.trash.available.then_some(places.trash.item_count),
        activation: if places.trash.available {
            SpecialActivation::OpenTrash
        } else {
            SpecialActivation::Unavailable {
                kind: SpecialItemKind::Trash,
                detail: "the desktop Trash authority is unavailable".into(),
            }
        },
    };
    vec![files, downloads, trash]
}

#[derive(Clone, Debug)]
pub(super) struct GroupedWindow {
    app_id: String,
    window: WindowItem,
}

pub(super) fn catalog_index(
    catalog: &[rmac_apps::Application],
) -> BTreeMap<String, &rmac_apps::Application> {
    let mut index = BTreeMap::new();
    for application in catalog {
        index
            .entry(canonical_app_id(&application.id))
            .or_insert(application);
    }
    index
}

pub(super) fn window_groups(
    compositor: &rmac_compositor::Snapshot,
) -> BTreeMap<String, Vec<GroupedWindow>> {
    let focused_id = compositor.focus.window;
    let mut groups: BTreeMap<String, Vec<GroupedWindow>> = BTreeMap::new();
    for window in &compositor.windows {
        let Some(app_id) = window
            .app_id
            .as_ref()
            .filter(|app_id| !app_id.trim().is_empty())
        else {
            continue;
        };
        let canonical = canonical_app_id(app_id);
        if canonical.is_empty() {
            continue;
        }
        groups.entry(canonical).or_default().push(GroupedWindow {
            app_id: app_id.clone(),
            window: WindowItem {
                id: window.id,
                title: window.title.clone(),
                focused: window.focused || focused_id == Some(window.id),
                urgent: window.urgent,
                focus_timestamp: window.focus_timestamp,
            },
        });
    }
    for grouped in groups.values_mut() {
        grouped.sort_by_key(|entry| {
            (
                Reverse(entry.window.focused),
                Reverse(timestamp_key(entry.window.focus_timestamp)),
                entry.window.id,
            )
        });
    }
    groups
}

pub(super) fn build_item(
    source_id: &str,
    application: Option<&rmac_apps::Application>,
    pinned: bool,
    grouped: Vec<GroupedWindow>,
) -> Item {
    let windows: Vec<_> = grouped.into_iter().map(|entry| entry.window).collect();
    let running = !windows.is_empty();
    Item {
        id: application
            .map(|application| application.id.clone())
            .unwrap_or_else(|| source_id.to_owned()),
        name: application
            .map(|application| application.name.clone())
            .unwrap_or_else(|| fallback_name(source_id)),
        icon: application.and_then(|application| application.icon.clone()),
        pinned,
        running,
        active: windows.iter().any(|window| window.focused),
        urgent: windows.iter().any(|window| window.urgent),
        launchable: application.is_some(),
        windows,
        launch: application.map(|application| application.launch.clone()),
    }
}

pub(super) fn canonical_app_id(app_id: &str) -> String {
    app_id
        .trim()
        .strip_suffix(".desktop")
        .unwrap_or(app_id.trim())
        .to_lowercase()
}

pub(super) fn fallback_name(app_id: &str) -> String {
    let canonical = app_id
        .trim()
        .strip_suffix(".desktop")
        .unwrap_or(app_id.trim());
    let name = canonical
        .rsplit(['.', '/', '-'])
        .find(|component| !component.is_empty())
        .unwrap_or("Application");
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Application".into(),
    }
}

pub(super) fn timestamp_key(timestamp: Option<rmac_compositor::Timestamp>) -> (u64, u32) {
    timestamp
        .map(|timestamp| (timestamp.seconds, timestamp.nanoseconds))
        .unwrap_or_default()
}
