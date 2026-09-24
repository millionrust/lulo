//! Spotlight Actions (⌘3): things rmac can really do, grouped by the app
//! that owns them, plus the frontmost first-party app's menu commands.
//!
//! macOS also lists Siri "Suggestions" and offers "Add quick keys"; rmac
//! has no usage ranking or Quick Keys store, so neither is shown.

use std::path::PathBuf;
use std::sync::Arc;

use rmac_launcher_system::{Backend as _, SystemBackend};
use rmac_quick_settings_system::Backend as _;

use super::surface::SurfaceBridge;

pub(crate) const NOTES_SECTION: &str = "Notes";
pub(crate) const CONTROL_CENTER_SECTION: &str = "Control Center";
pub(crate) const SYSTEM_SECTION: &str = "System";
pub(crate) const SETTINGS_SECTION: &str = "System Settings";
const COMPOSE_NOTE_ACTION: &str = "notes::ComposeNote";
/// Notes registers its menu endpoint shortly after launch; wait this long.
const NOTE_READY_ATTEMPTS: u32 = 25;
const NOTE_READY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);
/// Settings panes offered as actions once the user types.
const SETTINGS_ACTIONS: [&str; 10] = [
    "wifi",
    "bluetooth",
    "network",
    "displays",
    "sound",
    "appearance",
    "desktop-dock",
    "focus",
    "lock-screen",
    "keyboard",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    CreateNote,
    SetWifi(bool),
    SetBluetooth(bool),
    SetDoNotDisturb(bool),
    LockScreen,
    Sleep,
    OpenSetting(String),
    AppMenu { app_id: String, action: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActionItem {
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) section: String,
    pub(crate) icon: Option<PathBuf>,
    keywords: &'static [&'static str],
    /// Shown only for a matching query, like macOS menu commands.
    query_only: bool,
    pub(crate) command: Command,
}

/// Live state the toggles depend on. `None` hides the toggle: an absent
/// adapter or an unreachable service has no honest "Turn … On".
#[derive(Clone, Debug, Default)]
pub(crate) struct SystemState {
    pub(crate) wifi: Option<bool>,
    pub(crate) bluetooth: Option<bool>,
    pub(crate) do_not_disturb: Option<bool>,
}

/// The frontmost first-party app and its exported menus.
#[derive(Clone, Debug)]
pub(crate) struct Frontmost {
    pub(crate) app_id: String,
    pub(crate) menus: Vec<rmac_app_menu::Menu>,
}

/// Blocking: query NetworkManager, BlueZ and the Focus service.
pub(crate) fn load_system_state() -> SystemState {
    let backend = rmac_quick_settings_system::SystemBackend;
    SystemState {
        wifi: backend
            .wifi()
            .ok()
            .filter(|wifi| wifi.available)
            .map(|wifi| wifi.enabled),
        bluetooth: backend
            .bluetooth()
            .ok()
            .filter(|bluetooth| bluetooth.available)
            .map(|bluetooth| bluetooth.powered),
        do_not_disturb: backend.focus().ok().map(|focus| focus.enabled),
    }
}

/// The app whose window had focus before Spotlight opened, if it exports
/// a menu (first-party rmac apps).
pub(crate) async fn load_frontmost() -> Option<Frontmost> {
    let snapshot = rmac_compositor_niri::snapshot().await.ok()?;
    let focused = snapshot.focus.window;
    let window = snapshot
        .windows
        .iter()
        .filter(|window| window.app_id.is_some())
        .find(|window| focused.as_ref() == Some(&window.id) || window.focused)
        .or_else(|| {
            snapshot
                .windows
                .iter()
                .filter(|window| window.app_id.is_some())
                .max_by_key(|window| {
                    window
                        .focus_timestamp
                        .as_ref()
                        .map(|stamp| (stamp.seconds, stamp.nanoseconds))
                })
        })?;
    let app_id = window.app_id.clone()?;
    rmac_app_menu::bus_name(&app_id)?;
    let menus = rmac_app_menu::fetch(&app_id).await.ok()?;
    Some(Frontmost { app_id, menus })
}

fn icon(applications: &rmac_launcher_providers::ApplicationProvider, id: &str) -> Option<PathBuf> {
    applications.application(id).and_then(|app| app.icon)
}

fn item(
    title: impl Into<String>,
    section: &str,
    icon: Option<PathBuf>,
    keywords: &'static [&'static str],
    command: Command,
) -> ActionItem {
    ActionItem {
        title: title.into(),
        subtitle: section.to_owned(),
        section: section.to_owned(),
        icon,
        keywords,
        query_only: false,
        command,
    }
}

fn turn(what: &str, on: bool) -> String {
    format!("Turn {what} {}", if on { "On" } else { "Off" })
}

/// Every action rmac can perform right now, in display order.
pub(crate) fn catalog(
    state: &SystemState,
    frontmost: Option<&Frontmost>,
    applications: &rmac_launcher_providers::ApplicationProvider,
) -> Vec<ActionItem> {
    use rmac_apps::identity::{NOTES, SYSTEM_SETTINGS};

    let settings_icon = icon(applications, SYSTEM_SETTINGS);
    let mut items = Vec::new();
    if let Some(frontmost) = frontmost {
        let app_name = applications
            .application(&frontmost.app_id)
            .map(|app| app.name)
            .or_else(|| rmac_apps::identity::window_title(&frontmost.app_id).map(str::to_owned))
            .unwrap_or_else(|| frontmost.app_id.clone());
        let app_icon = icon(applications, &frontmost.app_id);
        for menu in &frontmost.menus {
            for entry in menu.items.iter().filter(|entry| entry.enabled) {
                items.push(ActionItem {
                    title: entry.label.trim_end_matches('…').to_owned(),
                    subtitle: format!("{app_name} > {}", menu.label),
                    section: app_name.clone(),
                    icon: app_icon.clone(),
                    keywords: &[],
                    query_only: true,
                    command: Command::AppMenu {
                        app_id: frontmost.app_id.clone(),
                        action: entry.action.clone(),
                    },
                });
            }
        }
    }
    if applications.application(NOTES).is_some() {
        items.push(item(
            "Create Note",
            NOTES_SECTION,
            icon(applications, NOTES),
            &["new note", "write"],
            Command::CreateNote,
        ));
    }
    if let Some(enabled) = state.wifi {
        items.push(item(
            turn("Wi-Fi", !enabled),
            CONTROL_CENTER_SECTION,
            settings_icon.clone(),
            &["wireless", "network", "internet"],
            Command::SetWifi(!enabled),
        ));
    }
    if let Some(powered) = state.bluetooth {
        items.push(item(
            turn("Bluetooth", !powered),
            CONTROL_CENTER_SECTION,
            settings_icon.clone(),
            &["wireless", "headphones"],
            Command::SetBluetooth(!powered),
        ));
    }
    if let Some(enabled) = state.do_not_disturb {
        items.push(item(
            turn("Do Not Disturb", !enabled),
            CONTROL_CENTER_SECTION,
            settings_icon.clone(),
            &["focus", "quiet", "notifications"],
            Command::SetDoNotDisturb(!enabled),
        ));
    }
    items.push(item(
        "Lock Screen",
        SYSTEM_SECTION,
        settings_icon.clone(),
        &["lock", "away"],
        Command::LockScreen,
    ));
    items.push(item(
        "Sleep",
        SYSTEM_SECTION,
        settings_icon.clone(),
        &["suspend"],
        Command::Sleep,
    ));
    let entries = rmac_launcher_providers::system_settings_entries();
    for pane in SETTINGS_ACTIONS {
        let Some(entry) = entries.iter().find(|entry| entry.pane_id == pane) else {
            continue;
        };
        items.push(ActionItem {
            title: format!("Open {} Settings", entry.title),
            subtitle: SETTINGS_SECTION.to_owned(),
            section: SETTINGS_SECTION.to_owned(),
            icon: settings_icon.clone(),
            keywords: &["settings", "preferences"],
            query_only: true,
            command: Command::OpenSetting(pane.to_owned()),
        });
    }
    items
}

/// The rows for `query`: everything browsable when empty, otherwise the
/// matches with title-prefix matches first (the top hit).
pub(crate) fn filter(items: &[ActionItem], query: &str) -> Vec<ActionItem> {
    let query = query.trim();
    if query.is_empty() {
        return items
            .iter()
            .filter(|item| !item.query_only)
            .cloned()
            .collect();
    }
    let lowered = query.to_lowercase();
    let mut matches = items
        .iter()
        .filter(|item| {
            rmac_launcher::query_matches(query, &item.title, Some(&item.subtitle))
                || item
                    .keywords
                    .iter()
                    .any(|keyword| rmac_launcher::query_matches(query, keyword, None))
        })
        .cloned()
        .collect::<Vec<_>>();
    matches.sort_by_key(|item| !item.title.to_lowercase().starts_with(&lowered));
    matches
}

/// Perform an action. Runs on the UI thread's executor; blocking system
/// calls go to a worker.
pub(super) async fn execute(
    command: Command,
    backend: Arc<SystemBackend<SurfaceBridge>>,
    notes: Option<rmac_apps::LaunchSpec>,
) -> Result<(), String> {
    let quick = rmac_quick_settings_system::SystemBackend;
    match command {
        Command::SetWifi(enabled) => {
            blocking::unblock(move || quick.set_wifi_enabled(enabled)).await
        }
        Command::SetBluetooth(powered) => {
            blocking::unblock(move || quick.set_bluetooth_powered(powered)).await
        }
        Command::SetDoNotDisturb(enabled) => {
            blocking::unblock(move || quick.set_focus_enabled(enabled)).await
        }
        Command::LockScreen => {
            blocking::unblock(|| rmac_shortcuts::lock::request().map_err(|error| error.to_string()))
                .await
        }
        Command::Sleep => {
            blocking::unblock(|| {
                let status = std::process::Command::new("systemctl")
                    .arg("suspend")
                    .status()
                    .map_err(|error| error.to_string())?;
                status
                    .success()
                    .then_some(())
                    .ok_or_else(|| "the system refused to sleep".to_owned())
            })
            .await
        }
        Command::OpenSetting(pane) => backend
            .open_setting(&pane)
            .await
            .map_err(|error| error.detail().to_owned()),
        Command::AppMenu { app_id, action } => rmac_app_menu::activate(&app_id, &action)
            .await
            .map_err(|_| "the app did not accept the command".to_owned()),
        Command::CreateNote => {
            let spec = notes.ok_or_else(|| "Notes is not installed".to_owned())?;
            if rmac_app_menu::activate(rmac_apps::identity::NOTES, COMPOSE_NOTE_ACTION)
                .await
                .is_ok()
            {
                return Ok(());
            }
            backend
                .launch(&spec)
                .await
                .map_err(|error| error.detail().to_owned())?;
            for _ in 0..NOTE_READY_ATTEMPTS {
                async_io::Timer::after(NOTE_READY_INTERVAL).await;
                if rmac_app_menu::activate(rmac_apps::identity::NOTES, COMPOSE_NOTE_ACTION)
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
            Err("Notes opened but did not create a note".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> SystemState {
        SystemState {
            wifi: Some(true),
            bluetooth: Some(false),
            do_not_disturb: None,
        }
    }

    #[test]
    fn toggles_name_the_opposite_of_the_live_state_and_hide_unknowns() {
        let items = catalog(
            &state(),
            None,
            &rmac_launcher_providers::ApplicationProvider::default(),
        );
        let titles = filter(&items, "")
            .into_iter()
            .map(|item| item.title)
            .collect::<Vec<_>>();
        assert!(titles.contains(&"Turn Wi-Fi Off".to_owned()));
        assert!(titles.contains(&"Turn Bluetooth On".to_owned()));
        assert!(!titles.iter().any(|title| title.contains("Do Not Disturb")));
        // Notes is not installed in an empty catalog, so it is not offered.
        assert!(!titles.contains(&"Create Note".to_owned()));
        assert!(titles.contains(&"Lock Screen".to_owned()));
        // Settings panes only appear for a query.
        assert!(!titles.iter().any(|title| title.starts_with("Open ")));
    }

    #[test]
    fn menu_commands_appear_only_for_a_query_and_prefix_matches_lead() {
        let frontmost = Frontmost {
            app_id: rmac_apps::identity::TERMINAL.to_owned(),
            menus: vec![rmac_app_menu::Menu {
                label: "Edit".into(),
                items: vec![rmac_app_menu::Item {
                    label: "Clear Scrollback".into(),
                    action: "terminal::ClearScrollback".into(),
                    shortcut: String::new(),
                    enabled: true,
                    separator_before: false,
                }],
            }],
        };
        let items = catalog(
            &state(),
            Some(&frontmost),
            &rmac_launcher_providers::ApplicationProvider::default(),
        );
        assert!(!filter(&items, "")
            .iter()
            .any(|item| item.title == "Clear Scrollback"));
        let matches = filter(&items, "clear");
        assert_eq!(matches[0].title, "Clear Scrollback");
        assert!(matches[0].subtitle.ends_with("> Edit"));
        let lock = filter(&items, "lock");
        assert_eq!(lock[0].title, "Lock Screen");
        assert!(lock
            .iter()
            .any(|item| item.command == Command::OpenSetting("lock-screen".into())));
    }
}
