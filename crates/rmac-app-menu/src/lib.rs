//! Bounded first-party application menu export for the rmac menu bar.

use std::collections::BTreeSet;
use std::future;

use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::{interface, Connection, Proxy};

pub const OBJECT_PATH: &str = "/org/rmac/AppMenu1";
pub const INTERFACE_NAME: &str = "org.rmac.AppMenu1";
const MAX_MENUS: usize = 8;
const MAX_ITEMS_PER_MENU: usize = 32;
const MAX_LABEL_BYTES: usize = 64;
const MAX_ACTION_BYTES: usize = 96;
const MAX_SHORTCUT_BYTES: usize = 32;
const ACTIVATION_CAPACITY: usize = 16;

pub type WireItem = (String, String, String, bool, bool);
pub type WireMenu = (String, Vec<WireItem>);
pub type WireMenus = Vec<WireMenu>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub label: String,
    pub action: String,
    pub shortcut: String,
    pub enabled: bool,
    pub separator_before: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Menu {
    pub label: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Copy)]
struct ItemSpec {
    label: &'static str,
    action: &'static str,
    shortcut: &'static str,
    separator_before: bool,
}

#[derive(Clone, Copy)]
struct MenuSpec {
    label: &'static str,
    items: &'static [ItemSpec],
}

macro_rules! item {
    ($label:literal, $action:literal, $shortcut:literal) => {
        ItemSpec {
            label: $label,
            action: $action,
            shortcut: $shortcut,
            separator_before: false,
        }
    };
    ($label:literal, $action:literal, $shortcut:literal, separator) => {
        ItemSpec {
            label: $label,
            action: $action,
            shortcut: $shortcut,
            separator_before: true,
        }
    };
}

const TEXT_EDITOR_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("New", "text_editor::NewFile", "⌘N"),
            item!("Open…", "text_editor::OpenFile", "⌘O"),
            item!("Save", "text_editor::SaveFile", "⌘S", separator),
            item!("Save As…", "text_editor::SaveFileAs", "⇧⌘S"),
            item!("Print…", "text_editor::PrintFile", "⌘P", separator),
            item!("Close Window", "text_editor::CloseWindow", "⌘W", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Find…", "text_editor::ToggleFind", "⌘F"),
            item!("Find Next", "text_editor::FindNext", "⌘G"),
            item!("Find Previous", "text_editor::FindPrev", "⇧⌘G"),
            item!("Replace…", "text_editor::ToggleReplace", "⌥⌘F", separator),
        ],
    },
    MenuSpec {
        label: "Format",
        items: &[
            item!("Bigger", "text_editor::IncreaseFont", "⌘+"),
            item!("Smaller", "text_editor::DecreaseFont", "⌘−"),
            item!("Monospaced", "text_editor::ToggleMono", "", separator),
        ],
    },
];

const TERMINAL_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "Shell",
        items: &[
            item!("New Tab", "terminal::NewTab", "⌘T"),
            item!("Close Tab", "terminal::CloseTab", "⌘W"),
            item!("Next Tab", "terminal::NextTab", "⇧⌘]", separator),
            item!("Previous Tab", "terminal::PrevTab", "⇧⌘["),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Copy", "terminal::Copy", "⌘C"),
            item!("Paste", "terminal::Paste", "⌘V"),
            item!("Select All", "terminal::SelectAll", "⌘A", separator),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Find…", "terminal::Find", "⌘F"),
            item!("Clear", "terminal::Clear", "⌘K"),
            item!("Bigger", "terminal::ZoomIn", "⌘+", separator),
            item!("Smaller", "terminal::ZoomOut", "⌘−"),
            item!("Actual Size", "terminal::ZoomReset", "⌘0"),
        ],
    },
];

const NOTES_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("New Note", "notes::ComposeNote", "⌘N"),
            item!("New Folder", "notes::CreateFolder", "⇧⌘N"),
            item!("Export Notes…", "notes::ExportNotes", "⇧⌘E", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[item!("Find…", "notes::FocusSearch", "⌘F")],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Sort by Date Edited", "notes::SortByEdited", ""),
            item!("Sort by Date Created", "notes::SortByCreated", ""),
            item!("Sort by Title", "notes::SortByTitle", ""),
        ],
    },
];

const FILES_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("New Folder", "finder::NewFolder", "⇧⌘N"),
            item!("New Tab", "finder::NewTab", "⌘T", separator),
            item!("Close Tab", "finder::CloseTab", "⌘W"),
            item!("Move to Trash", "finder::MoveToTrash", "⌘⌫", separator),
            item!("Get Info", "finder::GetInfo", "⌘I"),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "finder::UndoOperation", "⌘Z"),
            item!("Cut", "finder::CutItems", "⌘X", separator),
            item!("Copy", "finder::CopyItems", "⌘C"),
            item!("Paste", "finder::PasteItems", "⌘V"),
            item!("Select All", "finder::SelectAll", "⌘A", separator),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Show Hidden Files", "finder::ToggleHidden", "⇧⌘."),
            item!("Quick Look", "finder::QuickLook", "Space"),
            item!("Enclosing Folder", "finder::GoUp", "⌘↑"),
        ],
    },
];

const MONITOR_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "View",
        items: &[item!(
            "Find Process…",
            "activity_monitor::FocusSearch",
            "⌘F"
        )],
    },
    MenuSpec {
        label: "Process",
        items: &[
            item!("Quit Process…", "activity_monitor::QuitProcess", "Delete"),
            item!(
                "Force Quit Process…",
                "activity_monitor::ForceQuitProcess",
                "⇧⌘⌫"
            ),
        ],
    },
];

const SETTINGS_MENUS: &[MenuSpec] = &[MenuSpec {
    label: "View",
    items: &[item!("Back", "system_settings::GoBack", "⌘[")],
}];

fn specs(app_id: &str) -> Option<&'static [MenuSpec]> {
    match app_id {
        rmac_apps::identity::FILES => Some(FILES_MENUS),
        rmac_apps::identity::TERMINAL => Some(TERMINAL_MENUS),
        rmac_apps::identity::NOTES => Some(NOTES_MENUS),
        rmac_apps::identity::TEXT_EDITOR => Some(TEXT_EDITOR_MENUS),
        rmac_apps::identity::SYSTEM_MONITOR => Some(MONITOR_MENUS),
        rmac_apps::identity::SYSTEM_SETTINGS => Some(SETTINGS_MENUS),
        _ => None,
    }
}

pub fn bus_name(app_id: &str) -> Option<&'static str> {
    match app_id {
        rmac_apps::identity::FILES => Some("org.rmac.Files.Menu"),
        rmac_apps::identity::TERMINAL => Some("org.rmac.Terminal.Menu"),
        rmac_apps::identity::NOTES => Some("org.rmac.Notes.Menu"),
        rmac_apps::identity::TEXT_EDITOR => Some("org.rmac.TextEditor.Menu"),
        rmac_apps::identity::SYSTEM_MONITOR => Some("org.rmac.SystemMonitor.Menu"),
        rmac_apps::identity::SYSTEM_SETTINGS => Some("org.rmac.SystemSettings.Menu"),
        _ => None,
    }
}

/// Resolve only commands registered in this exact GPUI application binary.
pub fn definition(app_id: &str, registered_actions: &[&str]) -> Option<Vec<Menu>> {
    let registered = registered_actions.iter().copied().collect::<BTreeSet<_>>();
    let menus = specs(app_id)?
        .iter()
        .filter_map(|menu| {
            let mut items = menu
                .items
                .iter()
                .filter(|item| registered.contains(item.action))
                .map(|item| Item {
                    label: item.label.to_owned(),
                    action: item.action.to_owned(),
                    shortcut: item.shortcut.to_owned(),
                    enabled: true,
                    separator_before: item.separator_before,
                })
                .collect::<Vec<_>>();
            if let Some(first) = items.first_mut() {
                first.separator_before = false;
            }
            (!items.is_empty()).then(|| Menu {
                label: menu.label.to_owned(),
                items,
            })
        })
        .collect::<Vec<_>>();
    (!menus.is_empty()).then_some(menus)
}

#[derive(Clone)]
struct MenuInterface {
    menus: WireMenus,
    allowed: BTreeSet<String>,
    activation: async_channel::Sender<String>,
}

#[interface(name = "org.rmac.AppMenu1")]
impl MenuInterface {
    fn menus(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<WireMenus> {
        authenticated_sender(&header)?;
        Ok(self.menus.clone())
    }

    fn activate(&self, action: &str, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        if !valid_action(action) || !self.allowed.contains(action) {
            return Err(fdo::Error::InvalidArgs("menu action is unavailable".into()));
        }
        self.activation
            .try_send(action.to_owned())
            .map_err(|_| fdo::Error::Failed("menu activation queue is unavailable".into()))
    }
}

/// Own the application-specific menu endpoint until the application exits.
pub async fn serve(
    app_id: &str,
    menus: Vec<Menu>,
    activation: async_channel::Sender<String>,
) -> Result<(), Error> {
    let name = bus_name(app_id).ok_or(Error::Unsupported)?;
    validate_menus(&menus)?;
    let allowed = menus
        .iter()
        .flat_map(|menu| menu.items.iter().map(|item| item.action.clone()))
        .collect();
    let interface = MenuInterface {
        menus: encode(&menus),
        allowed,
        activation,
    };
    let _connection = Builder::session()
        .map_err(|_| Error::Bus)?
        .name(name)
        .map_err(|_| Error::Bus)?
        .serve_at(OBJECT_PATH, interface)
        .map_err(|_| Error::Bus)?
        .build()
        .await
        .map_err(|_| Error::Bus)?;
    future::pending::<()>().await;
    Ok(())
}

pub async fn fetch(app_id: &str) -> Result<Vec<Menu>, Error> {
    let name = bus_name(app_id).ok_or(Error::Unsupported)?;
    let connection = Connection::session().await.map_err(|_| Error::Bus)?;
    let proxy = Proxy::new(&connection, name, OBJECT_PATH, INTERFACE_NAME)
        .await
        .map_err(|_| Error::Bus)?;
    let wire: WireMenus = proxy.call("Menus", &()).await.map_err(|_| Error::Bus)?;
    decode(wire)
}

pub async fn activate(app_id: &str, action: &str) -> Result<(), Error> {
    let name = bus_name(app_id).ok_or(Error::Unsupported)?;
    if !valid_action(action) {
        return Err(Error::Protocol);
    }
    let connection = Connection::session().await.map_err(|_| Error::Bus)?;
    let proxy = Proxy::new(&connection, name, OBJECT_PATH, INTERFACE_NAME)
        .await
        .map_err(|_| Error::Bus)?;
    proxy
        .call::<_, _, ()>("Activate", &action)
        .await
        .map_err(|_| Error::Bus)
}

fn authenticated_sender(header: &Header<'_>) -> fdo::Result<()> {
    header
        .sender()
        .map(|_| ())
        .ok_or_else(|| fdo::Error::AccessDenied("menu caller identity is unavailable".into()))
}

fn encode(menus: &[Menu]) -> WireMenus {
    menus
        .iter()
        .map(|menu| {
            (
                menu.label.clone(),
                menu.items
                    .iter()
                    .map(|item| {
                        (
                            item.label.clone(),
                            item.action.clone(),
                            item.shortcut.clone(),
                            item.enabled,
                            item.separator_before,
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

fn decode(wire: WireMenus) -> Result<Vec<Menu>, Error> {
    let menus = wire
        .into_iter()
        .map(|(label, items)| Menu {
            label,
            items: items
                .into_iter()
                .map(
                    |(label, action, shortcut, enabled, separator_before)| Item {
                        label,
                        action,
                        shortcut,
                        enabled,
                        separator_before,
                    },
                )
                .collect(),
        })
        .collect::<Vec<_>>();
    validate_menus(&menus)?;
    Ok(menus)
}

fn validate_menus(menus: &[Menu]) -> Result<(), Error> {
    if menus.is_empty() || menus.len() > MAX_MENUS {
        return Err(Error::Protocol);
    }
    let mut actions = BTreeSet::new();
    for menu in menus {
        if !valid_label(&menu.label)
            || menu.items.is_empty()
            || menu.items.len() > MAX_ITEMS_PER_MENU
        {
            return Err(Error::Protocol);
        }
        for item in &menu.items {
            if !valid_label(&item.label)
                || !valid_action(&item.action)
                || item.shortcut.len() > MAX_SHORTCUT_BYTES
                || item.shortcut.chars().any(char::is_control)
                || !actions.insert(item.action.as_str())
            {
                return Err(Error::Protocol);
            }
        }
    }
    Ok(())
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_LABEL_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_action(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ACTION_BYTES
        && value.contains("::")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'-' | b'.'))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Unsupported,
    Bus,
    Protocol,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("application menus are not supported"),
            Self::Bus => formatter.write_str("application menu bus is unavailable"),
            Self::Protocol => formatter.write_str("application menu data is invalid"),
        }
    }
}

impl std::error::Error for Error {}

pub fn activation_channel() -> (
    async_channel::Sender<String>,
    async_channel::Receiver<String>,
) {
    async_channel::bounded(ACTIVATION_CAPACITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn third_party_and_app_drawer_cannot_publish_global_menus() {
        assert_eq!(bus_name("org.example.Editor"), None);
        assert_eq!(bus_name(rmac_apps::identity::APP_DRAWER), None);
        assert!(definition("org.example.Editor", &["editor::Save"]).is_none());
    }

    #[test]
    fn definitions_export_only_registered_real_actions() {
        let menus = definition(
            rmac_apps::identity::TEXT_EDITOR,
            &["text_editor::SaveFile", "text_editor::ToggleFind"],
        )
        .unwrap();
        assert_eq!(menus.len(), 2);
        assert_eq!(menus[0].items[0].label, "Save");
        assert_eq!(menus[1].items[0].action, "text_editor::ToggleFind");
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn wire_data_is_bounded_and_round_trips() {
        let menus = definition(
            rmac_apps::identity::TERMINAL,
            &["terminal::Copy", "terminal::Paste"],
        )
        .unwrap();
        assert_eq!(decode(encode(&menus)).unwrap(), menus);

        let oversized = vec![Menu {
            label: "x".repeat(MAX_LABEL_BYTES + 1),
            items: vec![Item {
                label: "Item".into(),
                action: "terminal::Copy".into(),
                shortcut: String::new(),
                enabled: true,
                separator_before: false,
            }],
        }];
        assert_eq!(validate_menus(&oversized), Err(Error::Protocol));
    }

    #[test]
    fn exported_hints_preserve_standard_macos_shortcuts() {
        let terminal = definition(
            rmac_apps::identity::TERMINAL,
            &[
                "terminal::NewTab",
                "terminal::CloseTab",
                "terminal::NextTab",
                "terminal::PrevTab",
                "terminal::Copy",
                "terminal::Paste",
                "terminal::SelectAll",
                "terminal::Find",
            ],
        )
        .unwrap();
        let terminal_hints = terminal
            .iter()
            .flat_map(|menu| menu.items.iter())
            .map(|item| (item.action.as_str(), item.shortcut.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(terminal_hints["terminal::NewTab"], "⌘T");
        assert_eq!(terminal_hints["terminal::CloseTab"], "⌘W");
        assert_eq!(terminal_hints["terminal::NextTab"], "⇧⌘]");
        assert_eq!(terminal_hints["terminal::PrevTab"], "⇧⌘[");
        assert_eq!(terminal_hints["terminal::Copy"], "⌘C");
        assert_eq!(terminal_hints["terminal::Paste"], "⌘V");
        assert_eq!(terminal_hints["terminal::SelectAll"], "⌘A");
        assert_eq!(terminal_hints["terminal::Find"], "⌘F");

        let editor = definition(
            rmac_apps::identity::TEXT_EDITOR,
            &["text_editor::FindPrev", "text_editor::ToggleReplace"],
        )
        .unwrap();
        let editor_hints = editor
            .iter()
            .flat_map(|menu| menu.items.iter())
            .map(|item| (item.action.as_str(), item.shortcut.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(editor_hints["text_editor::FindPrev"], "⇧⌘G");
        assert_eq!(editor_hints["text_editor::ToggleReplace"], "⌥⌘F");
    }
}
