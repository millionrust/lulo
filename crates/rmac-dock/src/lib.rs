//! Framework-neutral Dock application and activation model.

pub mod motion;

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowItem {
    pub id: rmac_compositor::WindowId,
    pub title: Option<String>,
    pub focused: bool,
    pub urgent: bool,
    pub focus_timestamp: Option<rmac_compositor::Timestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    /// Catalog identity when known, otherwise the compositor-provided app ID.
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
    pub pinned: bool,
    pub running: bool,
    pub active: bool,
    pub urgent: bool,
    pub launchable: bool,
    pub windows: Vec<WindowItem>,
    launch: Option<rmac_apps::LaunchSpec>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SpecialItemKind {
    Files,
    Downloads,
    Trash,
}

#[derive(Clone, Eq, PartialEq)]
pub enum SpecialActivation {
    OpenDirectory {
        kind: SpecialItemKind,
        path: PathBuf,
    },
    OpenTrash,
    Unavailable {
        kind: SpecialItemKind,
        detail: String,
    },
}

impl fmt::Debug for SpecialActivation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenDirectory { kind, .. } => formatter
                .debug_struct("OpenDirectory")
                .field("kind", kind)
                .field("path", &"<private>")
                .finish(),
            Self::OpenTrash => formatter.write_str("OpenTrash"),
            Self::Unavailable { kind, detail } => formatter
                .debug_struct("Unavailable")
                .field("kind", kind)
                .field("detail", detail)
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpecialItem {
    pub kind: SpecialItemKind,
    pub name: &'static str,
    pub available: bool,
    /// Present only for an authoritative Trash snapshot. Renderers may use it
    /// for an item-count badge but must not infer availability from the count.
    pub item_count: Option<usize>,
    activation: SpecialActivation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Activation {
    Launch {
        app_id: String,
        spec: rmac_apps::LaunchSpec,
    },
    FocusWindow(rmac_compositor::WindowId),
    NoAction,
    Unavailable {
        app_id: String,
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveDirection {
    Left,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PinCommand {
    Pin {
        app_id: String,
    },
    Unpin {
        app_id: String,
    },
    Move {
        app_id: String,
        direction: MoveDirection,
    },
    MoveTo {
        app_id: String,
        index: usize,
    },
}

impl PinCommand {
    pub fn app_id(&self) -> &str {
        match self {
            Self::Pin { app_id }
            | Self::Unpin { app_id }
            | Self::Move { app_id, .. }
            | Self::MoveTo { app_id, .. } => app_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextAction {
    LaunchNew {
        app_id: String,
        spec: rmac_apps::LaunchSpec,
    },
    FocusWindow {
        app_id: String,
        window: rmac_compositor::WindowId,
    },
    CloseWindow {
        app_id: String,
        window: rmac_compositor::WindowId,
    },
    UpdatePins(PinCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowMenu {
    pub id: rmac_compositor::WindowId,
    pub title: String,
    pub focused: bool,
    pub urgent: bool,
    pub focus: ContextAction,
    pub close: ContextAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextMenu {
    pub app_id: String,
    pub application_name: String,
    pub launch_new: Option<ContextAction>,
    pub windows: Vec<WindowMenu>,
    pub pin: PinCommand,
    pub move_left: Option<PinCommand>,
    pub move_right: Option<PinCommand>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PinError {
    InvalidIdentity,
    NotPinned { app_id: String },
}

impl fmt::Display for PinError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity => formatter.write_str("application identity is empty"),
            Self::NotPinned { app_id } => write!(formatter, "{app_id} is not pinned"),
        }
    }
}

impl std::error::Error for PinError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Model {
    pub items: Vec<Item>,
    /// Files, Downloads, and Trash are kept after a renderer-owned separator;
    /// they are not application identities and cannot enter pinned ordering.
    pub special_items: Vec<SpecialItem>,
    repeated_click: rmac_shell_settings::RepeatedClickBehavior,
}

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
        running.sort_by_key(|item| {
            (
                Reverse(latest_focus_timestamp(&item.windows)),
                item.name.to_lowercase(),
                item.id.clone(),
            )
        });
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

fn project_special_items(places: &rmac_places::Snapshot) -> Vec<SpecialItem> {
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

pub fn apply_pin_command(
    pinned: &[rmac_shell_settings::AppId],
    command: &PinCommand,
) -> Result<Vec<rmac_shell_settings::AppId>, PinError> {
    let canonical = canonical_app_id(command.app_id());
    if canonical.is_empty() {
        return Err(PinError::InvalidIdentity);
    }
    let mut result = pinned.to_vec();
    let existing = result
        .iter()
        .position(|app_id| canonical_app_id(&app_id.0) == canonical);
    match command {
        PinCommand::Pin { app_id } => {
            if existing.is_none() {
                result.push(rmac_shell_settings::AppId(app_id.clone()));
            }
        }
        PinCommand::Unpin { app_id } => {
            let Some(index) = existing else {
                return Err(PinError::NotPinned {
                    app_id: app_id.clone(),
                });
            };
            result.remove(index);
        }
        PinCommand::Move { app_id, direction } => {
            let Some(index) = existing else {
                return Err(PinError::NotPinned {
                    app_id: app_id.clone(),
                });
            };
            let destination = match direction {
                MoveDirection::Left => index.saturating_sub(1),
                MoveDirection::Right => (index + 1).min(result.len() - 1),
            };
            if destination != index {
                result.swap(index, destination);
            }
        }
        PinCommand::MoveTo {
            app_id,
            index: destination,
        } => {
            let Some(index) = existing else {
                return Err(PinError::NotPinned {
                    app_id: app_id.clone(),
                });
            };
            let destination = (*destination).min(result.len() - 1);
            if destination != index {
                let moved = result.remove(index);
                result.insert(destination, moved);
            }
        }
    }
    Ok(result)
}

#[derive(Clone, Debug)]
struct GroupedWindow {
    app_id: String,
    window: WindowItem,
}

fn catalog_index(catalog: &[rmac_apps::Application]) -> BTreeMap<String, &rmac_apps::Application> {
    let mut index = BTreeMap::new();
    for application in catalog {
        index
            .entry(canonical_app_id(&application.id))
            .or_insert(application);
    }
    index
}

fn window_groups(compositor: &rmac_compositor::Snapshot) -> BTreeMap<String, Vec<GroupedWindow>> {
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

fn build_item(
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

fn canonical_app_id(app_id: &str) -> String {
    app_id
        .trim()
        .strip_suffix(".desktop")
        .unwrap_or(app_id.trim())
        .to_lowercase()
}

fn fallback_name(app_id: &str) -> String {
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

fn timestamp_key(timestamp: Option<rmac_compositor::Timestamp>) -> (u64, u32) {
    timestamp
        .map(|timestamp| (timestamp.seconds, timestamp.nanoseconds))
        .unwrap_or_default()
}

fn latest_focus_timestamp(windows: &[WindowItem]) -> (u64, u32) {
    windows
        .iter()
        .map(|window| timestamp_key(window.focus_timestamp))
        .max()
        .unwrap_or_default()
}

pub fn surface_outputs(
    compositor: &rmac_compositor::Snapshot,
    scope: &rmac_shell_settings::OutputScope,
    primary: Option<&rmac_compositor::OutputId>,
) -> Vec<rmac_compositor::OutputId> {
    let enabled: BTreeSet<_> = compositor
        .outputs
        .iter()
        .filter(|output| output.enabled())
        .map(|output| output.id.clone())
        .collect();
    match scope {
        rmac_shell_settings::OutputScope::All => enabled.into_iter().collect(),
        rmac_shell_settings::OutputScope::Primary => primary
            .filter(|output| enabled.contains(*output))
            .cloned()
            .into_iter()
            .collect(),
        rmac_shell_settings::OutputScope::Named(name) => {
            let output = rmac_compositor::OutputId::from(name.as_str());
            enabled
                .contains(&output)
                .then_some(output)
                .into_iter()
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn application(id: &str, name: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from(format!("/apps/{id}")),
            icon: Some(PathBuf::from(format!("/icons/{id}.svg"))),
            categories: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: id.trim_end_matches(".desktop").into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    fn window(
        id: u64,
        app_id: &str,
        focused: bool,
        urgent: bool,
        seconds: u64,
    ) -> rmac_compositor::Window {
        rmac_compositor::Window {
            id: rmac_compositor::WindowId(id),
            title: Some(format!("Window {id}")),
            app_id: Some(app_id.into()),
            pid: None,
            workspace: None,
            focused,
            floating: false,
            urgent,
            focus_timestamp: Some(rmac_compositor::Timestamp {
                seconds,
                nanoseconds: 0,
            }),
            layout: rmac_compositor::WindowLayout::default(),
        }
    }

    #[test]
    fn pinned_order_leads_and_running_windows_group_by_desktop_identity() {
        let catalog = [
            application("finder.desktop", "Finder"),
            application("terminal.desktop", "Terminal"),
        ];
        let compositor = rmac_compositor::Snapshot {
            windows: vec![
                window(1, "terminal", false, false, 20),
                window(2, "terminal.desktop", true, true, 30),
                window(3, "org.example.music", false, false, 40),
            ],
            ..Default::default()
        };
        let model = Model::build(
            &[
                rmac_shell_settings::AppId("finder.desktop".into()),
                rmac_shell_settings::AppId("terminal.desktop".into()),
            ],
            &Default::default(),
            &catalog,
            &compositor,
        );
        assert_eq!(
            model
                .items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["Finder", "Terminal", "Music"]
        );
        assert!(!model.items[0].running);
        assert_eq!(model.items[1].windows.len(), 2);
        assert!(model.items[1].active);
        assert!(model.items[1].urgent);
        assert!(!model.items[2].launchable);
    }

    #[test]
    fn click_launches_or_focuses_without_optimistic_state() {
        let catalog = [application("finder.desktop", "Finder")];
        let pinned = [rmac_shell_settings::AppId("finder.desktop".into())];
        let model = Model::build(&pinned, &Default::default(), &catalog, &Default::default());
        assert!(matches!(
            model.activate("finder.desktop"),
            Activation::Launch { .. }
        ));

        let compositor = rmac_compositor::Snapshot {
            windows: vec![window(7, "finder", false, false, 8)],
            ..Default::default()
        };
        let model = Model::build(&pinned, &Default::default(), &catalog, &compositor);
        assert_eq!(
            model.activate("finder.desktop"),
            Activation::FocusWindow(rmac_compositor::WindowId(7))
        );
    }

    #[test]
    fn repeated_click_cycles_recent_windows_and_single_window_is_a_noop() {
        let catalog = [application("terminal.desktop", "Terminal")];
        let pinned = [rmac_shell_settings::AppId("terminal.desktop".into())];
        let compositor = rmac_compositor::Snapshot {
            windows: vec![
                window(1, "terminal", false, false, 10),
                window(2, "terminal", true, false, 20),
                window(3, "terminal", false, false, 30),
            ],
            ..Default::default()
        };
        let model = Model::build(&pinned, &Default::default(), &catalog, &compositor);
        assert_eq!(
            model.activate("terminal"),
            Activation::FocusWindow(rmac_compositor::WindowId(3))
        );

        let compositor = rmac_compositor::Snapshot {
            windows: vec![window(2, "terminal", true, false, 20)],
            ..Default::default()
        };
        let model = Model::build(&pinned, &Default::default(), &catalog, &compositor);
        assert_eq!(model.activate("terminal"), Activation::NoAction);
    }

    #[test]
    fn unsupported_hide_and_missing_pinned_app_are_truthful() {
        let settings = rmac_shell_settings::DockSettings {
            repeated_click: rmac_shell_settings::RepeatedClickBehavior::HideApplication,
            ..Default::default()
        };
        let compositor = rmac_compositor::Snapshot {
            windows: vec![window(1, "missing", true, false, 1)],
            ..Default::default()
        };
        let model = Model::build(
            &[rmac_shell_settings::AppId("missing.desktop".into())],
            &settings,
            &[],
            &compositor,
        );
        assert!(!model.items[0].launchable);
        assert!(matches!(
            model.activate("missing"),
            Activation::Unavailable { .. }
        ));

        let model = Model::build(
            &[rmac_shell_settings::AppId("gone.desktop".into())],
            &Default::default(),
            &[],
            &Default::default(),
        );
        assert!(matches!(
            model.activate("gone.desktop"),
            Activation::Unavailable { .. }
        ));
    }

    fn output(id: &str, enabled: bool) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: enabled.then_some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: enabled.then_some(rmac_compositor::LogicalOutput {
                position: Default::default(),
                size: rmac_compositor::LogicalSize {
                    width: 1920.0,
                    height: 1080.0,
                },
                scale: 1.0,
                transform: "normal".into(),
            }),
        }
    }

    #[test]
    fn output_scope_never_invents_a_primary_or_disabled_surface() {
        let compositor = rmac_compositor::Snapshot {
            outputs: vec![output("eDP-1", true), output("HDMI-A-1", false)],
            ..Default::default()
        };
        assert_eq!(
            surface_outputs(&compositor, &rmac_shell_settings::OutputScope::All, None,),
            [rmac_compositor::OutputId::from("eDP-1")]
        );
        assert!(surface_outputs(
            &compositor,
            &rmac_shell_settings::OutputScope::Primary,
            None,
        )
        .is_empty());
        assert!(surface_outputs(
            &compositor,
            &rmac_shell_settings::OutputScope::Named("HDMI-A-1".into()),
            None,
        )
        .is_empty());
        assert_eq!(
            surface_outputs(
                &compositor,
                &rmac_shell_settings::OutputScope::Primary,
                Some(&rmac_compositor::OutputId::from("eDP-1")),
            ),
            [rmac_compositor::OutputId::from("eDP-1")]
        );
    }

    #[test]
    fn context_menu_exposes_real_windows_without_inventing_quit() {
        let catalog = [application("terminal.desktop", "Terminal")];
        let pinned = [rmac_shell_settings::AppId("terminal.desktop".into())];
        let compositor = rmac_compositor::Snapshot {
            windows: vec![
                window(1, "terminal", true, false, 20),
                window(2, "terminal", false, true, 10),
            ],
            ..Default::default()
        };
        let model = Model::build(&pinned, &Default::default(), &catalog, &compositor);
        let menu = model.context_menu("terminal").expect("Dock item exists");
        assert_eq!(menu.application_name, "Terminal");
        assert!(matches!(
            menu.launch_new,
            Some(ContextAction::LaunchNew { .. })
        ));
        assert_eq!(menu.windows.len(), 2);
        assert!(menu.windows[0].focused);
        assert!(matches!(
            menu.windows[0].close,
            ContextAction::CloseWindow {
                window: rmac_compositor::WindowId(1),
                ..
            }
        ));
        assert_eq!(
            menu.pin,
            PinCommand::Unpin {
                app_id: "terminal.desktop".into()
            }
        );
        assert!(menu.move_left.is_none());
        assert!(menu.move_right.is_none());
    }

    fn places(
        downloads: &str,
        downloads_exists: bool,
        trash_count: usize,
    ) -> rmac_places::Snapshot {
        rmac_places::Snapshot {
            home: rmac_places::Place {
                path: PathBuf::from("/home/alex"),
                exists: true,
            },
            downloads: rmac_places::Place {
                path: PathBuf::from(downloads),
                exists: downloads_exists,
            },
            downloads_configured: true,
            trash: rmac_places::TrashSnapshot {
                available: true,
                empty: trash_count == 0,
                item_count: trash_count,
            },
        }
    }

    #[test]
    fn places_project_after_applications_without_becoming_pins() {
        let places = places("/home/alex/Transfers", true, 7);
        let model = Model::build_with_places(
            &[rmac_shell_settings::AppId("finder.desktop".into())],
            &Default::default(),
            &[application("finder.desktop", "Finder")],
            &Default::default(),
            &places,
        );

        assert_eq!(model.items.len(), 1);
        assert_eq!(model.items[0].id, "finder.desktop");
        assert_eq!(
            model
                .special_items
                .iter()
                .map(|item| (item.kind, item.name, item.available, item.item_count))
                .collect::<Vec<_>>(),
            [
                (SpecialItemKind::Files, "Files", true, None),
                (SpecialItemKind::Downloads, "Downloads", true, None),
                (SpecialItemKind::Trash, "Trash", true, Some(7)),
            ]
        );
        assert!(matches!(
            model.activate_special(SpecialItemKind::Downloads),
            SpecialActivation::OpenDirectory {
                kind: SpecialItemKind::Downloads,
                path
            } if path == Path::new("/home/alex/Transfers")
        ));
    }

    #[test]
    fn disabled_or_missing_places_remain_visible_and_truthfully_unavailable() {
        let mut places = places("/home/alex", true, 0);
        places.home.exists = false;
        let model =
            Model::build_with_places(&[], &Default::default(), &[], &Default::default(), &places);

        assert!(!model.special_items[0].available);
        assert!(matches!(
            model.activate_special(SpecialItemKind::Files),
            SpecialActivation::Unavailable {
                kind: SpecialItemKind::Files,
                ..
            }
        ));
        assert!(!model.special_items[1].available);
        assert!(matches!(
            model.activate_special(SpecialItemKind::Downloads),
            SpecialActivation::Unavailable {
                kind: SpecialItemKind::Downloads,
                ..
            }
        ));
        assert_eq!(model.special_items[2].item_count, Some(0));
    }

    #[test]
    fn special_activation_debug_never_discloses_private_paths() {
        let activation = SpecialActivation::OpenDirectory {
            kind: SpecialItemKind::Files,
            path: PathBuf::from("/home/alex/Private/Tax"),
        };
        let debug = format!("{activation:?}");
        assert!(debug.contains("<private>"));
        assert!(!debug.contains("alex"));
        assert!(!debug.contains("Tax"));
    }

    #[test]
    fn context_menu_offers_pin_for_an_unpinned_running_app() {
        let catalog = [application("music.desktop", "Music")];
        let compositor = rmac_compositor::Snapshot {
            windows: vec![window(4, "music", false, false, 1)],
            ..Default::default()
        };
        let model = Model::build(&[], &Default::default(), &catalog, &compositor);
        let menu = model
            .context_menu("music.desktop")
            .expect("running item exists");
        assert_eq!(
            menu.pin,
            PinCommand::Pin {
                app_id: "music.desktop".into()
            }
        );
    }

    #[test]
    fn pin_mutations_are_idempotent_bounded_and_preserve_exact_ids() {
        let pinned = vec![
            rmac_shell_settings::AppId("finder.desktop".into()),
            rmac_shell_settings::AppId("terminal.desktop".into()),
        ];
        assert_eq!(
            apply_pin_command(
                &pinned,
                &PinCommand::Pin {
                    app_id: "Finder".into()
                }
            )
            .expect("duplicate pin is a no-op"),
            pinned
        );
        let moved = apply_pin_command(
            &pinned,
            &PinCommand::Move {
                app_id: "terminal".into(),
                direction: MoveDirection::Left,
            },
        )
        .expect("pinned app moves");
        assert_eq!(moved[0].0, "terminal.desktop");
        assert_eq!(moved[1].0, "finder.desktop");
        let bounded = apply_pin_command(
            &moved,
            &PinCommand::Move {
                app_id: "terminal.desktop".into(),
                direction: MoveDirection::Left,
            },
        )
        .expect("edge move is a no-op");
        assert_eq!(bounded, moved);
        let unpinned = apply_pin_command(
            &bounded,
            &PinCommand::Unpin {
                app_id: "finder".into(),
            },
        )
        .expect("pin is removed");
        assert_eq!(
            unpinned,
            [rmac_shell_settings::AppId("terminal.desktop".into())]
        );
    }

    #[test]
    fn reorder_rejects_an_app_that_is_not_pinned() {
        let error = apply_pin_command(
            &[],
            &PinCommand::Move {
                app_id: "terminal.desktop".into(),
                direction: MoveDirection::Right,
            },
        )
        .expect_err("missing pin cannot move");
        assert_eq!(
            error,
            PinError::NotPinned {
                app_id: "terminal.desktop".into()
            }
        );
    }

    #[test]
    fn drag_reorder_moves_to_a_bounded_persisted_index() {
        let pinned = vec![
            rmac_shell_settings::AppId("finder.desktop".into()),
            rmac_shell_settings::AppId("terminal.desktop".into()),
            rmac_shell_settings::AppId("notes.desktop".into()),
        ];
        let moved = apply_pin_command(
            &pinned,
            &PinCommand::MoveTo {
                app_id: "finder".into(),
                index: usize::MAX,
            },
        )
        .expect("drag target is bounded");
        assert_eq!(
            moved
                .iter()
                .map(|app_id| app_id.0.as_str())
                .collect::<Vec<_>>(),
            ["terminal.desktop", "notes.desktop", "finder.desktop"]
        );
    }
}
