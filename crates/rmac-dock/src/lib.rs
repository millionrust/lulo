//! Framework-neutral Dock application and activation model.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Model {
    pub items: Vec<Item>,
    repeated_click: rmac_shell_settings::RepeatedClickBehavior,
}

impl Model {
    pub fn build(
        pinned: &[rmac_shell_settings::AppId],
        settings: &rmac_shell_settings::DockSettings,
        catalog: &[rmac_apps::Application],
        compositor: &rmac_compositor::Snapshot,
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
            repeated_click: settings.repeated_click,
        }
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
    use super::*;

    fn application(id: &str, name: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: name.into(),
            source: PathBuf::from(format!("/apps/{id}")),
            icon: Some(PathBuf::from(format!("/icons/{id}.svg"))),
            categories: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: id.trim_end_matches(".desktop").into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
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
}
