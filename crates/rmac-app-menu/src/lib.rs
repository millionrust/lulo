//! Bounded first-party application menu export for the rmac menu bar.

use std::collections::BTreeSet;
use std::future;
use std::sync::Mutex;

use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::{interface, Connection};

pub const OBJECT_PATH: &str = "/org/rmac/AppMenu1";
pub const INTERFACE_NAME: &str = "org.rmac.AppMenu1";
/// Served beside the menu by apps that run as one process with many
/// windows: a second launch asks the running process for a new window.
pub const INSTANCE_INTERFACE_NAME: &str = "org.rmac.AppInstance1";
const MAX_MENUS: usize = 8;
const MAX_ITEMS_PER_MENU: usize = 32;
const MAX_LABEL_BYTES: usize = 64;
const MAX_ACTION_BYTES: usize = 96;
const MAX_SHORTCUT_BYTES: usize = 32;
const ACTIVATION_CAPACITY: usize = 16;
const WINDOW_REQUEST_CAPACITY: usize = 4;
const MAX_WINDOW_ARGUMENTS: usize = 8;
const MAX_WINDOW_ARGUMENT_BYTES: usize = 4096;
const INSTANCE_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
        label: "Format",
        items: &[item!("Checklist", "notes::InsertChecklist", "⇧⌘L")],
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
            item!("Rename", "finder::RenameItem", ""),
            item!("Compress", "finder::Compress", ""),
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
            item!("as Icons", "finder::ViewAsIcons", "⌘1"),
            item!("as List", "finder::ViewAsList", "⌘2"),
            item!("as Columns", "finder::ViewAsColumns", "⌘3"),
            item!("as Gallery", "finder::ViewAsGallery", "⌘4"),
            item!("Sort by Name", "finder::SortByName", "", separator),
            item!("Sort by Date Modified", "finder::SortByDate", ""),
            item!("Sort by Size", "finder::SortBySize", ""),
            item!("Sort by Kind", "finder::SortByKind", ""),
            item!(
                "Show Hidden Files",
                "finder::ToggleHidden",
                "⇧⌘.",
                separator
            ),
            item!("Quick Look", "finder::QuickLook", "Space"),
        ],
    },
    MenuSpec {
        label: "Go",
        items: &[
            item!("Back", "finder::GoBack", "⌘["),
            item!("Forward", "finder::GoForward", "⌘]"),
            item!("Enclosing Folder", "finder::GoUp", "⌘↑"),
            item!("Home", "finder::GoHome", "⇧⌘H", separator),
            item!("Applications", "finder::GoApplications", "⇧⌘A"),
            item!("Downloads", "finder::GoDownloads", "⌥⌘L"),
            item!("Trash", "finder::GoTrash", ""),
        ],
    },
    MenuSpec {
        label: "Window",
        items: &[
            item!("Show Previous Tab", "finder::PreviousTab", "⌃⇧⇥"),
            item!("Show Next Tab", "finder::NextTab", "⌃⇥"),
        ],
    },
    MenuSpec {
        label: "Help",
        items: &[item!("Files Help", "finder::ShowHelp", "")],
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

const CALCULATOR_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Copy", "calculator::Copy", "⌘C"),
            item!("Paste", "calculator::Paste", "⌘V"),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[item!("Basic", "calculator::ShowBasic", "⌘1")],
    },
];

const PREVIEW_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("Open…", "preview::OpenFile", "⌘O"),
            item!("Close Window", "preview::CloseWindow", "⌘W", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Copy", "preview::Copy", "⌘C"),
            item!("Find", "preview::Find", "⌘F", separator),
            item!("Find Next", "preview::FindNext", "⌘G"),
            item!("Find Previous", "preview::FindPrevious", "⇧⌘G"),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Hide Sidebar", "preview::HideSidebar", "⌥⌘1"),
            item!("Thumbnails", "preview::ShowThumbnails", "⌥⌘2"),
            item!("Actual Size", "preview::ActualSize", "⌘0", separator),
            item!("Zoom to Fit", "preview::ZoomToFit", "⌘9"),
            item!("Zoom In", "preview::ZoomIn", "⌘+"),
            item!("Zoom Out", "preview::ZoomOut", "⌘−"),
        ],
    },
    MenuSpec {
        label: "Go",
        items: &[
            item!("Previous Item", "preview::PreviousItem", "⌥↑"),
            item!("Next Item", "preview::NextItem", "⌥↓"),
        ],
    },
    MenuSpec {
        label: "Tools",
        items: &[
            item!("Show Inspector", "preview::ShowInspector", "⌘I"),
            item!("Rotate Left", "preview::RotateLeft", "⌘L", separator),
            item!("Rotate Right", "preview::RotateRight", "⌘R"),
        ],
    },
];

const CLOCK_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("New", "clock::NewItem", "⌘N"),
            item!("Close Window", "clock::CloseWindow", "⌘W", separator),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("World Clock", "clock::ShowWorldClock", "⌘1"),
            item!("Alarms", "clock::ShowAlarms", "⌘2"),
            item!("Stopwatch", "clock::ShowStopwatch", "⌘3"),
            item!("Timers", "clock::ShowTimers", "⌘4"),
            item!("Start or Stop", "clock::StartStop", "", separator),
            item!("Lap or Reset", "clock::LapReset", ""),
        ],
    },
];

const WEATHER_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[item!("Close Window", "weather::CloseWindow", "⌘W")],
    },
    MenuSpec {
        label: "Edit",
        items: &[item!("Find", "weather::FindCity", "⌘F")],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Celsius", "weather::UseCelsius", ""),
            item!("Fahrenheit", "weather::UseFahrenheit", ""),
            item!("Refresh", "weather::Refresh", "⌘R", separator),
        ],
    },
];

const PLAYER_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("Open File…", "player::OpenFile", "⌘O"),
            item!("Close", "player::CloseWindow", "⌘W", separator),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[item!(
            "Enter Full Screen",
            "player::ToggleFullScreen",
            "⌃⌘F"
        )],
    },
    MenuSpec {
        label: "Playback",
        items: &[
            item!("Play", "player::PlayPause", "Space"),
            item!("Skip Back", "player::SkipBack", "←", separator),
            item!("Skip Forward", "player::SkipForward", "→"),
            item!("Previous", "player::PreviousItem", "⌘←", separator),
            item!("Next", "player::NextItem", "⌘→"),
            item!("Increase Volume", "player::VolumeUp", "⌘↑", separator),
            item!("Decrease Volume", "player::VolumeDown", "⌘↓"),
            item!("Mute", "player::ToggleMute", "M"),
        ],
    },
];

fn specs(app_id: &str) -> Option<&'static [MenuSpec]> {
    match app_id {
        rmac_apps::identity::FILES => Some(FILES_MENUS),
        rmac_apps::identity::TERMINAL => Some(TERMINAL_MENUS),
        rmac_apps::identity::NOTES => Some(NOTES_MENUS),
        rmac_apps::identity::TEXT_EDITOR => Some(TEXT_EDITOR_MENUS),
        rmac_apps::identity::SYSTEM_MONITOR => Some(MONITOR_MENUS),
        rmac_apps::identity::SYSTEM_SETTINGS => Some(SETTINGS_MENUS),
        rmac_apps::identity::CALCULATOR => Some(CALCULATOR_MENUS),
        rmac_apps::identity::PREVIEW => Some(PREVIEW_MENUS),
        rmac_apps::identity::CLOCK => Some(CLOCK_MENUS),
        rmac_apps::identity::WEATHER => Some(WEATHER_MENUS),
        rmac_apps::identity::PLAYER => Some(PLAYER_MENUS),
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
        rmac_apps::identity::CALCULATOR => Some("org.rmac.Calculator.Menu"),
        rmac_apps::identity::PREVIEW => Some("org.rmac.Preview.Menu"),
        rmac_apps::identity::CLOCK => Some("org.rmac.Clock.Menu"),
        rmac_apps::identity::WEATHER => Some("org.rmac.Weather.Menu"),
        rmac_apps::identity::PLAYER => Some("org.rmac.Player.Menu"),
        _ => None,
    }
}

/// Every first-party app that can publish a menu, for mapping a bus name back
/// to its app.
const MENU_APPS: &[&str] = &[
    rmac_apps::identity::FILES,
    rmac_apps::identity::TERMINAL,
    rmac_apps::identity::NOTES,
    rmac_apps::identity::TEXT_EDITOR,
    rmac_apps::identity::SYSTEM_MONITOR,
    rmac_apps::identity::SYSTEM_SETTINGS,
    rmac_apps::identity::CALCULATOR,
    rmac_apps::identity::PREVIEW,
    rmac_apps::identity::CLOCK,
    rmac_apps::identity::WEATHER,
    rmac_apps::identity::PLAYER,
];

/// The app whose menu endpoint owns `name`, if it is one.
pub fn app_for_bus_name(name: &str) -> Option<&'static str> {
    MENU_APPS
        .iter()
        .copied()
        .find(|app_id| bus_name(app_id) == Some(name))
}

/// Resolve only commands registered in this exact GPUI application binary.
pub fn definition(app_id: &str, registered_actions: &[&str]) -> Option<Vec<Menu>> {
    definition_for_vocabulary(
        app_id,
        registered_actions,
        rmac_locale::FileVocabulary::from_environment(),
    )
}

fn definition_for_vocabulary(
    app_id: &str,
    registered_actions: &[&str],
    file_words: rmac_locale::FileVocabulary,
) -> Option<Vec<Menu>> {
    let registered = registered_actions.iter().copied().collect::<BTreeSet<_>>();
    let menus = specs(app_id)?
        .iter()
        .filter_map(|menu| {
            let mut items = menu
                .items
                .iter()
                .filter(|item| registered.contains(item.action))
                .map(|item| Item {
                    label: match item.action {
                        "finder::MoveToTrash" => format!("Move to {}", file_words.bin()),
                        "finder::GoTrash" => file_words.bin().to_owned(),
                        _ => item.label.to_owned(),
                    },
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

#[derive(Clone)]
struct InstanceInterface {
    windows: async_channel::Sender<Vec<String>>,
}

#[interface(name = "org.rmac.AppInstance1")]
impl InstanceInterface {
    fn open_window(
        &self,
        arguments: Vec<String>,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        if !valid_window_arguments(&arguments) {
            return Err(fdo::Error::InvalidArgs(
                "window arguments are invalid".into(),
            ));
        }
        self.windows
            .try_send(arguments)
            .map_err(|_| fdo::Error::Failed("new-window queue is unavailable".into()))
    }
}

/// Own the application-specific menu endpoint until the application exits.
pub async fn serve(
    app_id: &str,
    menus: Vec<Menu>,
    activation: async_channel::Sender<String>,
) -> Result<(), Error> {
    serve_endpoint(app_id, menus, activation, None).await
}

/// [`serve`] for an app that keeps every window in one process: the same
/// bus name also accepts `OpenWindow` requests from later launches, which
/// arrive on `windows` as the launch's command-line arguments.
pub async fn serve_instance(
    app_id: &str,
    menus: Vec<Menu>,
    activation: async_channel::Sender<String>,
    windows: async_channel::Sender<Vec<String>>,
) -> Result<(), Error> {
    serve_endpoint(app_id, menus, activation, Some(windows)).await
}

async fn serve_endpoint(
    app_id: &str,
    menus: Vec<Menu>,
    activation: async_channel::Sender<String>,
    windows: Option<async_channel::Sender<Vec<String>>>,
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
    let mut builder = Builder::session()
        .map_err(bus_error("connect to the session bus"))?
        .name(name)
        .map_err(bus_error("request the menu bus name"))?
        .serve_at(OBJECT_PATH, interface)
        .map_err(bus_error("export the menu object"))?;
    if let Some(windows) = windows {
        builder = builder
            .serve_at(OBJECT_PATH, InstanceInterface { windows })
            .map_err(bus_error("export the app instance object"))?;
    }
    let _connection = builder
        .build()
        .await
        .map_err(bus_error("publish the menu"))?;
    future::pending::<()>().await;
    Ok(())
}

/// Ask this app's running process, if there is one, to open a window for
/// `arguments`. `Ok(true)` means it did and this launch should exit;
/// `Ok(false)` means no process owns the app's name, so this launch becomes
/// the running process. An error means one exists but did not answer.
pub async fn open_window_in_running_instance(
    app_id: &str,
    arguments: &[String],
) -> Result<bool, Error> {
    let name = bus_name(app_id).ok_or(Error::Unsupported)?;
    if !valid_window_arguments(arguments) {
        return Err(Error::Protocol);
    }
    // A launch asks once, with its own short timeout, then either exits or
    // becomes the running process; it does not share the menu connection.
    let connection = Builder::session()
        .map_err(bus_error("connect to the session bus"))?
        .method_timeout(INSTANCE_CALL_TIMEOUT)
        .build()
        .await
        .map_err(bus_error("connect to the session bus"))?;
    let bus = fdo::DBusProxy::new(&connection)
        .await
        .map_err(bus_error("reach the bus daemon"))?;
    let bus_name = zbus::names::BusName::try_from(name).map_err(|_| Error::Protocol)?;
    if !bus
        .name_has_owner(bus_name)
        .await
        .map_err(|error| Error::Bus(format!("could not look up {name}: {error}")))?
    {
        return Ok(false);
    }
    let proxy = zbus::Proxy::new(&connection, name, OBJECT_PATH, INSTANCE_INTERFACE_NAME)
        .await
        .map_err(bus_error("reach the running app"))?;
    proxy
        .call::<_, _, ()>("OpenWindow", &(arguments.to_vec(),))
        .await
        .map_err(|error| Error::Bus(format!("{name} OpenWindow: {error}")))?;
    Ok(true)
}

fn valid_window_arguments(arguments: &[String]) -> bool {
    arguments.len() <= MAX_WINDOW_ARGUMENTS
        && arguments
            .iter()
            .all(|argument| argument.len() <= MAX_WINDOW_ARGUMENT_BYTES && !argument.contains('\0'))
}

/// The menu bar and Spotlight ask for menus on every focus change, so every
/// caller in a process shares one session-bus connection instead of opening
/// and authenticating a new one per call.
static SESSION: Mutex<Option<Connection>> = Mutex::new(None);

async fn session() -> Result<Connection, Error> {
    if let Some(connection) = SESSION.lock().ok().and_then(|slot| slot.clone()) {
        return Ok(connection);
    }
    let connection = Connection::session()
        .await
        .map_err(bus_error("connect to the session bus"))?;
    let Ok(mut slot) = SESSION.lock() else {
        return Ok(connection);
    };
    // A concurrent first call may have connected as well; keep one so later
    // calls all reuse it.
    Ok(slot.get_or_insert(connection).clone())
}

/// Drop a connection the bus has closed so the next call reconnects.
fn forget_closed_session(error: &zbus::Error) {
    if matches!(error, zbus::Error::InputOutput(_)) {
        if let Ok(mut slot) = SESSION.lock() {
            *slot = None;
        }
    }
}

fn call_error(name: &'static str, method: &'static str) -> impl Fn(zbus::Error) -> Error {
    move |error| {
        forget_closed_session(&error);
        if is_unowned(&error) {
            Error::NotPublished
        } else {
            Error::Bus(format!("{name} {method}: {error}"))
        }
    }
}

pub async fn fetch(app_id: &str) -> Result<Vec<Menu>, Error> {
    let name = bus_name(app_id).ok_or(Error::Unsupported)?;
    let reply = session()
        .await?
        .call_method(Some(name), OBJECT_PATH, Some(INTERFACE_NAME), "Menus", &())
        .await
        .map_err(call_error(name, "Menus"))?;
    let wire: WireMenus = reply
        .body()
        .deserialize()
        .map_err(|error| Error::Bus(format!("{name} Menus reply: {error}")))?;
    decode(wire)
}

pub async fn activate(app_id: &str, action: &str) -> Result<(), Error> {
    let name = bus_name(app_id).ok_or(Error::Unsupported)?;
    if !valid_action(action) {
        return Err(Error::Protocol);
    }
    session()
        .await?
        .call_method(
            Some(name),
            OBJECT_PATH,
            Some(INTERFACE_NAME),
            "Activate",
            &action,
        )
        .await
        .map_err(call_error(name, "Activate"))?;
    Ok(())
}

/// Menu endpoints appearing and disappearing on the session bus, so a menu bar
/// can fetch an app's menus once the app has published them and drop them when
/// the app exits, without polling.
pub struct MenuOwners {
    stream: zbus::MessageStream,
}

/// Subscribe to owner changes of the `org.rmac.*` names on the shared
/// connection.
pub async fn watch_menu_owners() -> Result<MenuOwners, Error> {
    let rule_error =
        |error: zbus::Error| Error::Bus(format!("could not build the menu owner match: {error}"));
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(rule_error)?
        .interface("org.freedesktop.DBus")
        .map_err(rule_error)?
        .member("NameOwnerChanged")
        .map_err(rule_error)?
        .arg0ns("org.rmac")
        .map_err(rule_error)?
        .build();
    let connection = session().await?;
    let stream = zbus::MessageStream::for_match_rule(rule, &connection, Some(64))
        .await
        .map_err(bus_error("watch menu owners"))?;
    Ok(MenuOwners { stream })
}

impl MenuOwners {
    /// The next app whose menu endpoint appeared (`true`) or went away
    /// (`false`); `None` once the bus connection closes.
    pub async fn next(&mut self) -> Option<(&'static str, bool)> {
        use futures_util::StreamExt;
        loop {
            let message = match self.stream.next().await? {
                Ok(message) => message,
                Err(error) => {
                    forget_closed_session(&error);
                    return None;
                }
            };
            let body = message.body();
            let Ok((name, _old, new)) = body.deserialize::<(&str, &str, &str)>() else {
                continue;
            };
            if let Some(app_id) = app_for_bus_name(name) {
                return Some((app_id, !new.is_empty()));
            }
        }
    }
}

/// The bus has no owner for the app's menu name: the app is not running or
/// has not published its menu yet.
fn is_unowned(error: &zbus::Error) -> bool {
    const UNOWNED: [&str; 2] = [
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
    ];
    match error {
        zbus::Error::MethodError(name, _, _) => UNOWNED.contains(&name.as_str()),
        zbus::Error::FDO(error) => matches!(
            **error,
            fdo::Error::ServiceUnknown(_) | fdo::Error::NameHasNoOwner(_)
        ),
        _ => false,
    }
}

fn bus_error(context: &'static str) -> impl Fn(zbus::Error) -> Error {
    move |error| Error::Bus(format!("could not {context}: {error}"))
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Unsupported,
    /// Nothing owns the app's menu name: it is not running or has not
    /// published its menu yet.
    NotPublished,
    /// A D-Bus failure, carrying zbus's description so logs say why.
    Bus(String),
    Protocol,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("application menus are not supported"),
            Self::NotPublished => {
                formatter.write_str("the application is not running or has not published its menu")
            }
            Self::Bus(detail) => write!(formatter, "application menu D-Bus call failed: {detail}"),
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

pub fn window_request_channel() -> (
    async_channel::Sender<Vec<String>>,
    async_channel::Receiver<Vec<String>>,
) {
    async_channel::bounded(WINDOW_REQUEST_CAPACITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bus_names_map_back_to_their_app() {
        for app_id in MENU_APPS {
            let name = bus_name(app_id).expect("every menu app has a bus name");
            assert_eq!(app_for_bus_name(name), Some(*app_id));
            assert!(specs(app_id).is_some());
        }
        assert_eq!(app_for_bus_name("org.rmac.Focus1"), None);
        assert_eq!(app_for_bus_name("org.rmac.Files"), None);
    }

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

    #[test]
    fn files_exports_working_view_go_and_window_menus() {
        let menus = definition(
            rmac_apps::identity::FILES,
            &[
                "finder::ViewAsIcons",
                "finder::SortByName",
                "finder::GoBack",
                "finder::GoHome",
                "finder::PreviousTab",
                "finder::NextTab",
                "finder::ShowHelp",
            ],
        )
        .unwrap();
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.label.as_str())
                .collect::<Vec<_>>(),
            ["View", "Go", "Window", "Help"]
        );
        assert_eq!(menus[0].items[0].label, "as Icons");
        assert_eq!(menus[1].items[0].action, "finder::GoBack");
        assert_eq!(menus[2].items[1].shortcut, "⌃⇥");
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn preview_exports_its_measured_menus() {
        assert_eq!(
            bus_name(rmac_apps::identity::PREVIEW),
            Some("org.rmac.Preview.Menu")
        );
        let actions = PREVIEW_MENUS
            .iter()
            .flat_map(|menu| menu.items.iter().map(|item| item.action))
            .collect::<Vec<_>>();
        let menus = definition(rmac_apps::identity::PREVIEW, &actions).unwrap();
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.label.as_str())
                .collect::<Vec<_>>(),
            ["File", "Edit", "View", "Go", "Tools"]
        );
        let shortcut = |label: &str| {
            menus
                .iter()
                .flat_map(|menu| &menu.items)
                .find(|item| item.label == label)
                .map(|item| item.shortcut.clone())
        };
        assert_eq!(shortcut("Hide Sidebar").as_deref(), Some("⌥⌘1"));
        assert_eq!(shortcut("Actual Size").as_deref(), Some("⌘0"));
        assert_eq!(shortcut("Rotate Right").as_deref(), Some("⌘R"));
        assert_eq!(shortcut("Next Item").as_deref(), Some("⌥↓"));
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn clock_exports_file_and_view_menus() {
        assert_eq!(
            bus_name(rmac_apps::identity::CLOCK),
            Some("org.rmac.Clock.Menu")
        );
        let actions = CLOCK_MENUS
            .iter()
            .flat_map(|menu| menu.items.iter().map(|item| item.action))
            .collect::<Vec<_>>();
        let menus = definition(rmac_apps::identity::CLOCK, &actions).unwrap();
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.label.as_str())
                .collect::<Vec<_>>(),
            ["File", "View"]
        );
        assert_eq!(menus[1].items[2].label, "Stopwatch");
        assert_eq!(menus[1].items[2].shortcut, "⌘3");
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn weather_exports_its_menus() {
        let actions = WEATHER_MENUS
            .iter()
            .flat_map(|menu| menu.items.iter().map(|item| item.action))
            .collect::<Vec<_>>();
        let menus = definition(rmac_apps::identity::WEATHER, &actions).unwrap();
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.label.as_str())
                .collect::<Vec<_>>(),
            ["File", "Edit", "View"]
        );
        assert_eq!(menus[2].items[2].shortcut, "⌘R");
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn player_exports_playback_menus() {
        let actions = PLAYER_MENUS
            .iter()
            .flat_map(|menu| menu.items.iter().map(|item| item.action))
            .collect::<Vec<_>>();
        let menus = definition(rmac_apps::identity::PLAYER, &actions).unwrap();
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.label.as_str())
                .collect::<Vec<_>>(),
            ["File", "View", "Playback"]
        );
        assert_eq!(menus[1].items[0].shortcut, "⌃⌘F");
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn calculator_exports_edit_and_view_menus() {
        assert_eq!(
            bus_name(rmac_apps::identity::CALCULATOR),
            Some("org.rmac.Calculator.Menu")
        );
        let menus = definition(
            rmac_apps::identity::CALCULATOR,
            &[
                "calculator::Copy",
                "calculator::Paste",
                "calculator::ShowBasic",
            ],
        )
        .unwrap();
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.label.as_str())
                .collect::<Vec<_>>(),
            ["Edit", "View"]
        );
        assert_eq!(menus[0].items[0].shortcut, "⌘C");
        assert_eq!(menus[0].items[1].shortcut, "⌘V");
        assert_eq!(menus[1].items[0].label, "Basic");
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn files_menu_uses_the_message_locale_file_vocabulary() {
        let menus = definition_for_vocabulary(
            rmac_apps::identity::FILES,
            &["finder::MoveToTrash", "finder::GoTrash"],
            rmac_locale::FileVocabulary::for_locale("en_GB.UTF-8"),
        )
        .unwrap();
        assert_eq!(menus[0].items[0].label, "Move to Bin");
        assert_eq!(menus[1].items[0].label, "Bin");
    }

    #[test]
    fn window_requests_are_bounded() {
        assert!(valid_window_arguments(&[]));
        assert!(valid_window_arguments(&[
            "--path".to_owned(),
            "/home/user/Documents".to_owned()
        ]));
        assert!(!valid_window_arguments(&vec![
            String::new();
            MAX_WINDOW_ARGUMENTS + 1
        ]));
        assert!(!valid_window_arguments(&[
            "x".repeat(MAX_WINDOW_ARGUMENT_BYTES + 1)
        ]));
        assert!(!valid_window_arguments(&["a\0b".to_owned()]));
    }

    #[test]
    fn files_file_menu_offers_rename_after_get_info() {
        let menus = definition_for_vocabulary(
            rmac_apps::identity::FILES,
            &["finder::GetInfo", "finder::RenameItem", "finder::Compress"],
            rmac_locale::FileVocabulary::for_locale("en_US.UTF-8"),
        )
        .unwrap();
        let labels = menus[0]
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(menus[0].label, "File");
        assert_eq!(labels, ["Get Info", "Rename", "Compress"]);
        assert_eq!(menus[0].items[1].action, "finder::RenameItem");
    }
}
