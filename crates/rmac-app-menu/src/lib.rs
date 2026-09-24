//! Bounded first-party application menu export for the rmac menu bar.
//!
//! An app publishes its menus on the session bus under its own name
//! ([`bus_name`]) at [`OBJECT_PATH`], through two interfaces:
//!
//! - `org.rmac.AppMenu1`, unchanged since the first release: flat menus and
//!   `Activate`. Older menu bars and Spotlight read this one.
//! - `org.rmac.AppMenu2` ([`INTERFACE_V2_NAME`]): the full tree, with
//!   submenus, checkmarks and live enabled state, a `LayoutChanged`
//!   signal when the app's state changes its menus, and the same
//!   `Activate`. `Layout` also asks the app to re-validate its items first,
//!   as AppKit validates a menu as it opens.
//!
//! [`fetch`] prefers version 2 and falls back to version 1, so a new menu
//! bar still shows an app that has not been updated yet.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use zbus::connection::Builder;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{interface, Connection};

pub mod unsaved;
mod wire;

pub use wire::{flags, WireItem, WireItemV2, WireLayout, WireMenu, WireMenuV2, WireMenus};

pub const OBJECT_PATH: &str = "/org/rmac/AppMenu1";
pub const INTERFACE_NAME: &str = "org.rmac.AppMenu1";
/// The tree-shaped successor of [`INTERFACE_NAME`], served beside it at the
/// same path.
pub const INTERFACE_V2_NAME: &str = "org.rmac.AppMenu2";
/// Served beside the menu by apps that run as one process with many
/// windows: a second launch asks the running process for a new window.
pub const INSTANCE_INTERFACE_NAME: &str = "org.rmac.AppInstance1";
const MAX_LABEL_BYTES: usize = 64;
const MAX_ACTION_BYTES: usize = 96;
const MAX_SHORTCUT_BYTES: usize = 32;
const ACTIVATION_CAPACITY: usize = 16;
const WINDOW_REQUEST_CAPACITY: usize = 4;
const VALIDATION_CAPACITY: usize = 4;
const MAX_WINDOW_ARGUMENTS: usize = 8;
const MAX_WINDOW_ARGUMENT_BYTES: usize = 4096;
const INSTANCE_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// How long `Layout` waits for a busy app to re-validate its items before
/// it answers with the last published state instead. The menu is drawn at
/// once either way; this bounds how late a greyed-out row can be corrected.
const VALIDATION_TIMEOUT: Duration = Duration::from_millis(120);

/// The label of an exported menu whose items belong in the bold app-name
/// menu (Finder ▸ Empty Trash…). The menu bar folds them in under "About"
/// instead of showing a menu with this label.
pub const APPLICATION_MENU: &str = "Application";

/// The standard app-menu item every rmac-ui app answers with its About
/// panel. The menu bar shows it as "About <App>".
pub const ABOUT_ACTION: &str = "rmac::ShowAboutPanel";

/// The label of an exported menu whose items join the menu bar's standard
/// Window menu, after its window commands and before the list of windows.
pub const WINDOW_MENU: &str = "Window";

/// A checkmark column state, as `NSControl.StateValue`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CheckState {
    #[default]
    Off,
    On,
    /// The dash shown when a choice applies to only part of the selection.
    Mixed,
}

impl From<bool> for CheckState {
    fn from(on: bool) -> Self {
        if on {
            Self::On
        } else {
            Self::Off
        }
    }
}

/// One menu row. An item with `children` opens a submenu and is never
/// activated itself; its `action` only names it.
///
/// Build items with [`Item::new`] and the builder methods, or with a
/// struct literal ending in `..Item::default()`, so fields added later do
/// not break callers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Item {
    pub label: String,
    pub action: String,
    pub shortcut: String,
    pub enabled: bool,
    pub separator_before: bool,
    pub checked: CheckState,
    pub children: Vec<Item>,
}

impl Item {
    /// An enabled, unchecked command row.
    pub fn new(
        label: impl Into<String>,
        action: impl Into<String>,
        shortcut: impl Into<String>,
    ) -> Self {
        Self {
            label: label.into(),
            action: action.into(),
            shortcut: shortcut.into(),
            enabled: true,
            ..Self::default()
        }
    }

    /// A row that opens `children` as a submenu.
    pub fn submenu(
        label: impl Into<String>,
        action: impl Into<String>,
        children: Vec<Item>,
    ) -> Self {
        Self {
            children,
            ..Self::new(label, action, "")
        }
    }

    pub fn separated(mut self) -> Self {
        self.separator_before = true;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn checked(mut self, checked: impl Into<CheckState>) -> Self {
        self.checked = checked.into();
        self
    }

    pub fn is_submenu(&self) -> bool {
        !self.children.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Menu {
    pub label: String,
    pub items: Vec<Item>,
}

impl Menu {
    /// Every activatable row, depth first, with the titles of the submenus
    /// leading to it (for Spotlight's "App > Edit > Find > Find Next").
    pub fn leaves(&self) -> Vec<(Vec<&str>, &Item)> {
        fn walk<'a>(
            items: &'a [Item],
            path: &mut Vec<&'a str>,
            out: &mut Vec<(Vec<&'a str>, &'a Item)>,
        ) {
            for item in items {
                if item.children.is_empty() {
                    out.push((path.clone(), item));
                } else {
                    path.push(item.label.as_str());
                    walk(&item.children, path, out);
                    path.pop();
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.items, &mut Vec::new(), &mut out);
        out
    }
}

/// App-supplied live state for one item, applied over the static definition
/// by [`apply_state`]. `None` keeps the definition's value.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ItemState {
    pub enabled: Option<bool>,
    pub checked: Option<CheckState>,
    pub label: Option<String>,
}

/// The menus with each command's live state from `state`. A submenu is
/// enabled while any of its items is, as in AppKit.
pub fn apply_state(menus: &[Menu], state: impl Fn(&Item) -> Option<ItemState>) -> Vec<Menu> {
    fn apply(items: &[Item], state: &dyn Fn(&Item) -> Option<ItemState>) -> Vec<Item> {
        items
            .iter()
            .map(|item| {
                let mut next = item.clone();
                if item.children.is_empty() {
                    if let Some(update) = state(item) {
                        if let Some(enabled) = update.enabled {
                            next.enabled = enabled;
                        }
                        if let Some(checked) = update.checked {
                            next.checked = checked;
                        }
                        if let Some(label) = update.label.filter(|label| valid_label(label)) {
                            next.label = label;
                        }
                    }
                } else {
                    next.children = apply(&item.children, state);
                    next.enabled = next.children.iter().any(|child| child.enabled);
                }
                next
            })
            .collect()
    }
    menus
        .iter()
        .map(|menu| Menu {
            label: menu.label.clone(),
            items: apply(&menu.items, &state),
        })
        .collect()
}

#[derive(Clone, Copy)]
struct ItemSpec {
    label: &'static str,
    action: &'static str,
    shortcut: &'static str,
    separator_before: bool,
    /// Non-empty for a submenu, whose `action` then only names it.
    children: &'static [ItemSpec],
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
            children: &[],
        }
    };
    ($label:literal, $action:literal, $shortcut:literal, separator) => {
        ItemSpec {
            label: $label,
            action: $action,
            shortcut: $shortcut,
            separator_before: true,
            children: &[],
        }
    };
}

/// A submenu row. Its `action` is a stable name, never a command, and the
/// row is left out when none of its items is available.
macro_rules! submenu {
    ($label:literal, $action:literal, [$($child:expr),* $(,)?]) => {
        ItemSpec {
            label: $label,
            action: $action,
            shortcut: "",
            separator_before: false,
            children: &[$($child),*],
        }
    };
    ($label:literal, $action:literal, [$($child:expr),* $(,)?], separator) => {
        ItemSpec {
            label: $label,
            action: $action,
            shortcut: "",
            separator_before: true,
            children: &[$($child),*],
        }
    };
}

/// The text-field Edit commands, answered by whichever rmac text field has
/// keyboard focus (gpui-component's `input::` actions). The app side greys
/// them out while no text field is focused.
pub const TEXT_FIELD_ACTION_PREFIX: &str = "input::";

// Each app's menus follow the Mac app it stands for, as read from macOS
// 26.2's Accessibility tree (docs/parity-audit-2026-09-24-apps.md), minus
// the commands the app does not have. Every shortcut hint is the key the
// app binds. The app menu and the Window menu come from the menu bar.

const TEXT_EDITOR_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("New", "text_editor::NewFile", "⌘N"),
            item!("Open…", "text_editor::OpenFile", "⌘O"),
            item!("Close", "text_editor::CloseWindow", "⌘W", separator),
            item!("Save", "text_editor::SaveFile", "⌘S", separator),
            item!("Save As…", "text_editor::SaveFileAs", "⇧⌘S"),
            item!("Export as PDF…", "text_editor::ExportPdf", "", separator),
            item!("Print…", "text_editor::PrintFile", "⌘P", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "input::Undo", "⌘Z"),
            item!("Redo", "input::Redo", "⇧⌘Z"),
            item!("Cut", "input::Cut", "⌘X", separator),
            item!("Copy", "input::Copy", "⌘C"),
            item!("Paste", "input::Paste", "⌘V"),
            item!("Delete", "input::Delete", ""),
            item!("Select All", "input::SelectAll", "⌘A"),
            submenu!(
                "Find",
                "text_editor::FindMenu",
                [
                    item!("Find…", "text_editor::ToggleFind", "⌘F"),
                    item!("Find and Replace…", "text_editor::ToggleReplace", "⌥⌘F"),
                    item!("Find Next", "text_editor::FindNext", "⌘G"),
                    item!("Find Previous", "text_editor::FindPrev", "⇧⌘G"),
                ],
                separator
            ),
        ],
    },
    MenuSpec {
        label: "Format",
        items: &[
            submenu!(
                "Font",
                "text_editor::FontMenu",
                [
                    item!("Bigger", "text_editor::IncreaseFont", "⌘+"),
                    item!("Smaller", "text_editor::DecreaseFont", "⌘−"),
                ]
            ),
            item!("Monospaced", "text_editor::ToggleMono", "⇧⌘M", separator),
        ],
    },
];

const TERMINAL_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[item!("Settings…", "terminal::ShowSettings", "⌘,")],
    },
    MenuSpec {
        label: "Shell",
        items: &[
            item!("New Window", "terminal::NewWindow", "⌘N"),
            item!("New Tab", "terminal::NewTab", "⌘T"),
            item!("Close Tab", "terminal::CloseTab", "⌘W", separator),
            item!("Reset", "terminal::ResetTerminal", "⌥⌘R", separator),
            item!("Hard Reset", "terminal::HardResetTerminal", "⌃⌥⌘R"),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Copy", "terminal::Copy", "⌘C"),
            item!("Paste", "terminal::Paste", "⌘V"),
            item!("Select All", "terminal::SelectAll", "⌘A", separator),
            item!(
                "Select Between Marks",
                "terminal::SelectCommandOutput",
                "⇧⌘A"
            ),
            submenu!(
                "Navigate",
                "terminal::NavigateMenu",
                [
                    item!("Jump to Previous Mark", "terminal::PreviousPrompt", "⌘↑"),
                    item!("Jump to Next Mark", "terminal::NextPrompt", "⌘↓"),
                ],
                separator
            ),
            item!("Clear to Start", "terminal::Clear", "⌘K", separator),
            submenu!(
                "Find",
                "terminal::FindMenu",
                [
                    item!("Find…", "terminal::Find", "⌘F"),
                    item!("Find Next", "terminal::FindNext", "⌘G"),
                    item!("Find Previous", "terminal::FindPrevious", "⇧⌘G"),
                ],
                separator
            ),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Default Font Size", "terminal::ZoomReset", "⌘0"),
            item!("Bigger", "terminal::ZoomIn", "⌘+"),
            item!("Smaller", "terminal::ZoomOut", "⌘−"),
            item!("Next Profile", "terminal::CycleProfile", "⇧⌘P", separator),
        ],
    },
    MenuSpec {
        label: WINDOW_MENU,
        items: &[
            item!("Show Previous Tab", "terminal::PrevTab", "⇧⌘["),
            item!("Show Next Tab", "terminal::NextTab", "⇧⌘]"),
        ],
    },
];

const NOTES_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("New Note", "notes::ComposeNote", "⌘N"),
            item!("New Folder", "notes::CreateFolder", "⇧⌘N"),
            item!("Close", "rmac_ui::RequestClose", "⌘W", separator),
            item!("Import to Notes…", "notes::ImportNote", "", separator),
            item!("Import Notes Bundle…", "notes::ImportNotesBundle", ""),
            submenu!(
                "Export as",
                "notes::ExportAsMenu",
                [item!("PDF…", "notes::ExportNotePdf", "")],
                separator
            ),
            item!("Export Notes…", "notes::ExportNotes", "⇧⌘E"),
            item!("Pin Note", "notes::TogglePin", "", separator),
            item!("Print…", "notes::PrintNote", "⌘P", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "input::Undo", "⌘Z"),
            item!("Redo", "input::Redo", "⇧⌘Z"),
            item!("Cut", "input::Cut", "⌘X", separator),
            item!("Copy", "input::Copy", "⌘C"),
            item!("Paste", "input::Paste", "⌘V"),
            item!("Select All", "input::SelectAll", "⌘A"),
            item!("Add Photo…", "notes::AddPhoto", "", separator),
            submenu!(
                "Find",
                "notes::FindMenu",
                [item!("Note List Search…", "notes::FocusSearch", "⌘F")],
                separator
            ),
        ],
    },
    MenuSpec {
        label: "Format",
        items: &[item!("Checklist", "notes::InsertChecklist", "⇧⌘L")],
    },
    MenuSpec {
        label: "View",
        items: &[
            submenu!(
                "Sort By",
                "notes::SortByMenu",
                [
                    item!("Date Edited", "notes::SortByEdited", ""),
                    item!("Date Created", "notes::SortByCreated", ""),
                    item!("Title", "notes::SortByTitle", ""),
                ]
            ),
            item!(
                "Markdown Preview",
                "notes::ToggleMarkdownPreview",
                "",
                separator
            ),
        ],
    },
];

const FILES_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[item!("Empty Trash…", "finder::EmptyTrash", "⇧⌘⌫")],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("New Finder Window", "finder::NewWindow", "⌘N"),
            item!("New Folder", "finder::NewFolder", "⇧⌘N"),
            item!("New Tab", "finder::NewTab", "⌘T"),
            item!("Open", "finder::OpenItems", "⌘O"),
            item!("Close Tab", "finder::CloseTab", "⌘W"),
            item!("Get Info", "finder::GetInfo", "⌘I", separator),
            item!("Rename", "finder::RenameItem", ""),
            item!("Compress", "finder::Compress", ""),
            item!("Duplicate", "finder::Duplicate", "⌘D"),
            item!("Move to Trash", "finder::MoveToTrash", "⌘⌫", separator),
            // The Mac shows this as Move to Bin's ⌥ alternate; the menu bar
            // has no alternates yet, so it is listed after it.
            item!("Delete Immediately…", "finder::DeletePermanently", "⌥⌘⌫"),
            item!("Find", "finder::Find", "⌘F", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "finder::UndoOperation", "⌘Z"),
            item!("Cut", "finder::CutItems", "⌘X", separator),
            item!("Copy", "finder::CopyItems", "⌘C"),
            // ⌥ alternates of Copy and Paste on the Mac.
            item!("Copy as Pathname", "finder::CopyAsPathname", "⌥⌘C"),
            item!("Paste", "finder::PasteItems", "⌘V"),
            item!("Move Item Here", "finder::MoveItemHere", "⌥⌘V"),
            item!("Select All", "finder::SelectAll", "⌘A"),
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
            item!("Recents", "finder::GoRecents", "⇧⌘F", separator),
            item!("Documents", "finder::GoDocuments", "⇧⌘O"),
            item!("Desktop", "finder::GoDesktop", "⇧⌘D"),
            item!("Downloads", "finder::GoDownloads", "⌥⌘L"),
            item!("Home", "finder::GoHome", "⇧⌘H"),
            item!("Computer", "finder::GoComputer", "⇧⌘C"),
            item!("Applications", "finder::GoApplications", "⇧⌘A"),
            item!("Trash", "finder::GoTrash", ""),
            item!("Go to Folder…", "finder::GoToFolder", "⇧⌘G", separator),
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
        label: "Edit",
        items: &[
            item!("Undo", "input::Undo", "⌘Z"),
            item!("Redo", "input::Redo", "⇧⌘Z"),
            item!("Cut", "input::Cut", "⌘X", separator),
            item!("Copy", "input::Copy", "⌘C"),
            item!("Paste", "input::Paste", "⌘V"),
            item!("Select All", "input::SelectAll", "⌘A"),
            submenu!(
                "Find",
                "activity_monitor::FindMenu",
                [item!("Find…", "activity_monitor::FocusSearch", "⌘F")],
                separator
            ),
        ],
    },
    MenuSpec {
        label: "View",
        // Activity Monitor keeps its process commands in View. The Mac
        // binds Quit Process to ⌥⌘Q; here ⌘⌫ does it, and the hint says so.
        items: &[
            item!("Quit Process", "activity_monitor::QuitProcess", "⌘⌫"),
            item!(
                "Force Quit Process…",
                "activity_monitor::ForceQuitProcess",
                "⇧⌘⌫"
            ),
        ],
    },
];

const SETTINGS_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "input::Undo", "⌘Z"),
            item!("Redo", "input::Redo", "⇧⌘Z"),
            item!("Cut", "input::Cut", "⌘X", separator),
            item!("Copy", "input::Copy", "⌘C"),
            item!("Paste", "input::Paste", "⌘V"),
            item!("Select All", "input::SelectAll", "⌘A"),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Back", "system_settings::GoBack", "⌘["),
            item!("Forward", "system_settings::GoForward", "⌘]"),
            item!("Search", "system_settings::FocusSearch", "⌘F", separator),
        ],
    },
];

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
    MenuSpec {
        // Calculator has no File menu; Close is in its Window menu.
        label: WINDOW_MENU,
        items: &[item!("Close", "calculator::CloseWindow", "⌘W")],
    },
];

const PREVIEW_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "File",
        items: &[
            item!("Open…", "preview::OpenFile", "⌘O"),
            item!("Close Window", "preview::CloseWindow", "⌘W", separator),
            item!("Print…", "preview::PrintDocument", "⌘P", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "input::Undo", "⌘Z"),
            item!("Redo", "input::Redo", "⇧⌘Z"),
            item!("Cut", "input::Cut", "⌘X", separator),
            // Copy and Select All work on a PDF's text as well as on
            // an image.
            item!("Copy", "preview::Copy", "⌘C"),
            item!("Paste", "input::Paste", "⌘V"),
            item!("Select All", "preview::SelectAll", "⌘A"),
            submenu!(
                "Find",
                "preview::FindMenu",
                [
                    item!("Find…", "preview::Find", "⌘F"),
                    item!("Find Next", "preview::FindNext", "⌘G"),
                    item!("Find Previous", "preview::FindPrevious", "⇧⌘G"),
                ],
                separator
            ),
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
            item!("Go to Page…", "preview::GoToPage", "⌥⌘G", separator),
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
            item!("Close", "clock::CloseWindow", "⌘W", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "input::Undo", "⌘Z"),
            item!("Redo", "input::Redo", "⇧⌘Z"),
            item!("Cut", "input::Cut", "⌘X", separator),
            item!("Copy", "input::Copy", "⌘C"),
            item!("Paste", "input::Paste", "⌘V"),
            item!("Select All", "input::SelectAll", "⌘A"),
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
    let mut menus = specs(app_id)?
        .iter()
        .filter_map(|menu| {
            let items = resolve_items(
                menu.items,
                &|action| registered.contains(action),
                file_words,
            );
            (!items.is_empty()).then(|| Menu {
                label: menu.label.to_owned(),
                items,
            })
        })
        .collect::<Vec<_>>();
    if menus.is_empty() {
        return None;
    }
    // Every rmac-ui app answers About with its panel.
    if registered.contains(ABOUT_ACTION) {
        let about = Item::new("About", ABOUT_ACTION, "");
        match menus.iter_mut().find(|menu| menu.label == APPLICATION_MENU) {
            Some(menu) => menu.items.insert(0, about),
            None => menus.insert(
                0,
                Menu {
                    label: APPLICATION_MENU.to_owned(),
                    items: vec![about],
                },
            ),
        }
    }
    Some(menus)
}

/// A first-party app's menus as its binary declares them, for showing them
/// before the app runs (Files owns the desktop's menus). Every command is
/// listed; the running app's own export replaces this once it starts.
pub fn static_definition(app_id: &str) -> Option<Vec<Menu>> {
    let file_words = rmac_locale::FileVocabulary::from_environment();
    let menus = specs(app_id)?
        .iter()
        .filter_map(|menu| {
            let items = resolve_items(menu.items, &|_| true, file_words);
            (!items.is_empty()).then(|| Menu {
                label: menu.label.to_owned(),
                items,
            })
        })
        .collect::<Vec<_>>();
    (!menus.is_empty()).then_some(menus)
}

/// The spec rows whose commands `available` accepts. A submenu stays while
/// any of its rows does, and a separator stays with its group even when the
/// group's first row is gone.
fn resolve_items(
    specs: &[ItemSpec],
    available: &dyn Fn(&str) -> bool,
    file_words: rmac_locale::FileVocabulary,
) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut separate = false;
    for spec in specs {
        separate |= spec.separator_before;
        let children = if spec.children.is_empty() {
            if !available(spec.action) {
                continue;
            }
            Vec::new()
        } else {
            let children = resolve_items(spec.children, available, file_words);
            if children.is_empty() {
                continue;
            }
            children
        };
        items.push(Item {
            label: match spec.action {
                "finder::MoveToTrash" => format!("Move to {}", file_words.bin()),
                "finder::GoTrash" => file_words.bin().to_owned(),
                "finder::EmptyTrash" => format!("Empty {}…", file_words.bin()),
                _ => spec.label.to_owned(),
            },
            action: spec.action.to_owned(),
            shortcut: spec.shortcut.to_owned(),
            enabled: true,
            separator_before: separate && !items.is_empty(),
            checked: CheckState::Off,
            children,
        });
        separate = false;
    }
    items
}

/// Removes the exported [`APPLICATION_MENU`] from `menus` and returns its
/// items, the first one marked to follow a separator.
pub fn take_application_items(menus: &mut Vec<Menu>) -> Vec<Item> {
    let Some(index) = menus.iter().position(|menu| menu.label == APPLICATION_MENU) else {
        return Vec::new();
    };
    let mut items = menus.remove(index).items;
    if let Some(first) = items.first_mut() {
        first.separator_before = true;
    }
    items
}

/// Removes the exported [`WINDOW_MENU`] from `menus` and returns its items
/// for the menu bar's standard Window menu.
pub fn take_window_items(menus: &mut Vec<Menu>) -> Vec<Item> {
    let Some(index) = menus.iter().position(|menu| menu.label == WINDOW_MENU) else {
        return Vec::new();
    };
    menus.remove(index).items
}

/// Removes the exported "Help" menu from `menus` and returns its items for
/// the menu bar's standard Help menu.
pub fn take_help_items(menus: &mut Vec<Menu>) -> Vec<Item> {
    let Some(index) = menus.iter().position(|menu| menu.label == "Help") else {
        return Vec::new();
    };
    menus.remove(index).items
}

/// The menus an endpoint serves right now.
struct Published {
    menus: Vec<Menu>,
    revision: u32,
    /// The commands `Activate` accepts: every row that is not a submenu.
    activatable: BTreeSet<String>,
}

impl Published {
    fn new(menus: Vec<Menu>) -> Self {
        Self {
            activatable: activatable_actions(&menus),
            menus,
            revision: 1,
        }
    }

    /// Serve `menus` from now on; the new revision when they differ.
    fn replace(&mut self, menus: Vec<Menu>) -> Option<u32> {
        if self.menus == menus {
            return None;
        }
        self.activatable = activatable_actions(&menus);
        self.menus = menus;
        self.revision = self.revision.wrapping_add(1).max(1);
        Some(self.revision)
    }
}

fn activatable_actions(menus: &[Menu]) -> BTreeSet<String> {
    menus
        .iter()
        .flat_map(Menu::leaves)
        .map(|(_, item)| item.action.clone())
        .collect()
}

type SharedMenus = Arc<Mutex<Published>>;

fn published(shared: &SharedMenus) -> MutexGuard<'_, Published> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How an app answers a validation request: the channel carries the reply
/// channel for its freshly validated menus.
pub type ValidationReply = async_channel::Sender<Vec<Menu>>;

fn activate_published(
    shared: &SharedMenus,
    activation: &async_channel::Sender<String>,
    action: &str,
) -> fdo::Result<()> {
    if !valid_action(action) || !published(shared).activatable.contains(action) {
        return Err(fdo::Error::InvalidArgs("menu action is unavailable".into()));
    }
    activation
        .try_send(action.to_owned())
        .map_err(|_| fdo::Error::Failed("menu activation queue is unavailable".into()))
}

#[derive(Clone)]
struct MenuInterface {
    shared: SharedMenus,
    activation: async_channel::Sender<String>,
}

#[interface(name = "org.rmac.AppMenu1")]
impl MenuInterface {
    fn menus(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<WireMenus> {
        authenticated_sender(&header)?;
        let flat = wire::flatten_for_v1(&published(&self.shared).menus);
        Ok(wire::encode_v1(&flat))
    }

    fn activate(&self, action: &str, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        activate_published(&self.shared, &self.activation, action)
    }
}

#[derive(Clone)]
struct MenuInterfaceV2 {
    shared: SharedMenus,
    activation: async_channel::Sender<String>,
    validation: Option<async_channel::Sender<ValidationReply>>,
}

#[interface(name = "org.rmac.AppMenu2")]
impl MenuInterfaceV2 {
    /// The menu tree with each item's state validated now, the way AppKit
    /// validates a menu as it opens. An app too busy to answer within
    /// [`VALIDATION_TIMEOUT`] gets its last published state served.
    async fn layout(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<WireLayout> {
        authenticated_sender(&header)?;
        if let Some(validation) = &self.validation {
            if let Some(menus) = request_validation(validation).await {
                if wire::validate(&menus, wire::V2_LIMITS).is_ok() {
                    published(&self.shared).replace(menus);
                }
            }
        }
        let published = published(&self.shared);
        Ok((published.revision, wire::encode_v2(&published.menus)))
    }

    fn activate(&self, action: &str, #[zbus(header)] header: Header<'_>) -> fdo::Result<()> {
        authenticated_sender(&header)?;
        activate_published(&self.shared, &self.activation, action)
    }

    /// The app's state changed its menus; `Layout` returns them.
    #[zbus(signal)]
    async fn layout_changed(emitter: &SignalEmitter<'_>, revision: u32) -> zbus::Result<()>;
}

async fn request_validation(
    validation: &async_channel::Sender<ValidationReply>,
) -> Option<Vec<Menu>> {
    use futures_util::future::{select, Either};

    let (reply, answers) = async_channel::bounded(1);
    validation.try_send(reply).ok()?;
    let answer = std::pin::pin!(answers.recv());
    let timeout = std::pin::pin!(async_io::Timer::after(VALIDATION_TIMEOUT));
    match select(answer, timeout).await {
        Either::Left((Ok(menus), _)) => Some(menus),
        _ => None,
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

/// Publish the application-specific menu endpoint. It stays on the bus until
/// the process exits; `Ok` means the name is owned and the menus are served.
pub async fn serve(
    app_id: &str,
    menus: Vec<Menu>,
    activation: async_channel::Sender<String>,
) -> Result<(), Error> {
    serve_menus(app_id, menus, activation, EndpointOptions::default())
        .await
        .map(|_| ())
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
    let options = EndpointOptions {
        windows: Some(windows),
        ..EndpointOptions::default()
    };
    serve_menus(app_id, menus, activation, options)
        .await
        .map(|_| ())
}

/// What an endpoint serves beside its menus.
#[derive(Default)]
pub struct EndpointOptions {
    /// Accept `OpenWindow` from later launches (see [`serve_instance`]).
    pub windows: Option<async_channel::Sender<Vec<String>>>,
    /// Asked for freshly validated menus whenever a reader calls `Layout`.
    /// The app replies on the channel it receives.
    pub validation: Option<async_channel::Sender<ValidationReply>>,
}

/// Publish the menu endpoint and keep a [`Publisher`] for announcing later
/// state changes. The endpoint stays on the bus until the process exits.
pub async fn serve_menus(
    app_id: &str,
    menus: Vec<Menu>,
    activation: async_channel::Sender<String>,
    options: EndpointOptions,
) -> Result<Publisher, Error> {
    let name = bus_name(app_id).ok_or(Error::Unsupported)?;
    validate_menus(&menus)?;
    let shared = Arc::new(Mutex::new(Published::new(menus)));
    let mut builder = Builder::session()
        .map_err(bus_error("connect to the session bus"))?
        .name(name)
        .map_err(bus_error("request the menu bus name"))?
        .serve_at(
            OBJECT_PATH,
            MenuInterface {
                shared: shared.clone(),
                activation: activation.clone(),
            },
        )
        .map_err(bus_error("export the menu object"))?
        .serve_at(
            OBJECT_PATH,
            MenuInterfaceV2 {
                shared: shared.clone(),
                activation,
                validation: options.validation,
            },
        )
        .map_err(bus_error("export the menu tree"))?;
    if let Some(windows) = options.windows {
        builder = builder
            .serve_at(OBJECT_PATH, InstanceInterface { windows })
            .map_err(bus_error("export the app instance object"))?;
    }
    let connection = builder
        .build()
        .await
        .map_err(bus_error("publish the menu"))?;
    keep_for_process(connection.clone());
    Ok(Publisher { connection, shared })
}

/// Announces an app's menu changes on its published endpoint.
#[derive(Clone)]
pub struct Publisher {
    connection: Connection,
    shared: SharedMenus,
}

impl Publisher {
    /// Serve `menus` from now on and, when they differ from what was
    /// served, tell readers with `LayoutChanged`.
    pub async fn publish(&self, menus: Vec<Menu>) -> Result<(), Error> {
        validate_menus(&menus)?;
        let Some(revision) = published(&self.shared).replace(menus) else {
            return Ok(());
        };
        let emitter = SignalEmitter::new(&self.connection, OBJECT_PATH)
            .map_err(bus_error("address the menu signal"))?;
        MenuInterfaceV2::layout_changed(&emitter, revision)
            .await
            .map_err(bus_error("announce the changed menus"))
    }
}

/// Published endpoints, kept for the life of the process.
static ENDPOINTS: Mutex<Vec<Connection>> = Mutex::new(Vec::new());

/// Keep `connection` -- and the names it owns -- until the process exits.
///
/// Parking it in a task that awaits a never-ready future is not enough: such
/// a future registers no waker, and once the executor drops its last waker a
/// detached task is cancelled, which closed the connection and released the
/// menu name a few milliseconds after it was acquired (about one launch in
/// three on the reference laptop).
fn keep_for_process(connection: Connection) {
    ENDPOINTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(connection);
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

/// An app's menus as it serves them now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layout {
    /// `None` from an app that serves only version 1, which never changes
    /// its menus.
    pub revision: Option<u32>,
    pub menus: Vec<Menu>,
}

pub async fn fetch(app_id: &str) -> Result<Vec<Menu>, Error> {
    fetch_layout(app_id).await.map(|layout| layout.menus)
}

/// The app's validated menu tree, from `org.rmac.AppMenu2`, or its flat
/// menus from `org.rmac.AppMenu1` when it predates version 2.
pub async fn fetch_layout(app_id: &str) -> Result<Layout, Error> {
    let name = bus_name(app_id).ok_or(Error::Unsupported)?;
    let connection = session().await?;
    let reply = match connection
        .call_method(
            Some(name),
            OBJECT_PATH,
            Some(INTERFACE_V2_NAME),
            "Layout",
            &(),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) if is_unknown_interface(&error) => {
            return fetch_v1(&connection, name).await.map(|menus| Layout {
                revision: None,
                menus,
            });
        }
        Err(error) => return Err(call_error(name, "Layout")(error)),
    };
    let (revision, wire): WireLayout = reply
        .body()
        .deserialize()
        .map_err(|error| Error::Bus(format!("{name} Layout reply: {error}")))?;
    Ok(Layout {
        revision: Some(revision),
        menus: wire::decode_v2(wire)?,
    })
}

async fn fetch_v1(connection: &Connection, name: &'static str) -> Result<Vec<Menu>, Error> {
    let reply = connection
        .call_method(Some(name), OBJECT_PATH, Some(INTERFACE_NAME), "Menus", &())
        .await
        .map_err(call_error(name, "Menus"))?;
    let wire: WireMenus = reply
        .body()
        .deserialize()
        .map_err(|error| Error::Bus(format!("{name} Menus reply: {error}")))?;
    wire::decode_v1(wire)
}

/// The app is running but does not serve `org.rmac.AppMenu2`.
fn is_unknown_interface(error: &zbus::Error) -> bool {
    const UNKNOWN: [&str; 3] = [
        "org.freedesktop.DBus.Error.UnknownInterface",
        "org.freedesktop.DBus.Error.UnknownMethod",
        "org.freedesktop.DBus.Error.UnknownObject",
    ];
    match error {
        zbus::Error::MethodError(name, _, _) => UNKNOWN.contains(&name.as_str()),
        zbus::Error::FDO(error) => matches!(
            **error,
            fdo::Error::UnknownInterface(_)
                | fdo::Error::UnknownMethod(_)
                | fdo::Error::UnknownObject(_)
        ),
        _ => false,
    }
}

/// `LayoutChanged` signals from every first-party app, so a menu bar can
/// re-read the active app's menus when its state changes them.
pub struct LayoutChanges {
    stream: zbus::MessageStream,
}

pub async fn watch_layout_changes() -> Result<LayoutChanges, Error> {
    let rule_error =
        |error: zbus::Error| Error::Bus(format!("could not build the menu change match: {error}"));
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(INTERFACE_V2_NAME)
        .map_err(rule_error)?
        .member("LayoutChanged")
        .map_err(rule_error)?
        .path(OBJECT_PATH)
        .map_err(rule_error)?
        .build();
    let connection = session().await?;
    let stream = zbus::MessageStream::for_match_rule(rule, &connection, Some(64))
        .await
        .map_err(bus_error("watch menu changes"))?;
    Ok(LayoutChanges { stream })
}

impl LayoutChanges {
    /// Waits for the next change; `false` once the bus connection closes.
    pub async fn next(&mut self) -> bool {
        use futures_util::StreamExt;
        match self.stream.next().await {
            Some(Ok(_)) => true,
            Some(Err(error)) => {
                forget_closed_session(&error);
                false
            }
            None => false,
        }
    }
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

/// Whether `menus` fit the version 2 limits: labels, actions and shortcuts
/// well formed and bounded, every action unique, submenus at most three
/// levels deep.
pub fn validate_menus(menus: &[Menu]) -> Result<(), Error> {
    wire::validate(menus, wire::V2_LIMITS)
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

/// The channel an endpoint sends validation requests on
/// ([`EndpointOptions::validation`]).
pub fn validation_channel() -> (
    async_channel::Sender<ValidationReply>,
    async_channel::Receiver<ValidationReply>,
) {
    async_channel::bounded(VALIDATION_CAPACITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command a spec table names, submenu rows included.
    fn spec_actions(specs: &[MenuSpec]) -> Vec<&'static str> {
        fn walk(items: &[ItemSpec], out: &mut Vec<&'static str>) {
            for item in items {
                out.push(item.action);
                walk(item.children, out);
            }
        }
        let mut out = Vec::new();
        for menu in specs {
            walk(menu.items, &mut out);
        }
        out
    }

    /// action → shortcut hint for every command row.
    fn hints(menus: &[Menu]) -> std::collections::BTreeMap<String, String> {
        menus
            .iter()
            .flat_map(Menu::leaves)
            .map(|(_, item)| (item.action.clone(), item.shortcut.clone()))
            .collect()
    }

    fn labels(menus: &[Menu]) -> Vec<&str> {
        menus.iter().map(|menu| menu.label.as_str()).collect()
    }

    #[test]
    fn every_app_fits_the_version_2_limits_with_every_command_present() {
        for app_id in MENU_APPS {
            let specs = specs(app_id).unwrap();
            let menus = definition(app_id, &spec_actions(specs)).unwrap();
            assert_eq!(validate_menus(&menus), Ok(()), "{app_id}");
            assert_eq!(static_definition(app_id).unwrap().len(), menus.len());
            // And an older menu bar still gets a menu it accepts.
            let flat = wire::flatten_for_v1(&menus);
            assert_eq!(wire::validate(&flat, wire::V1_LIMITS), Ok(()), "{app_id}");
        }
    }

    #[test]
    fn submenus_survive_only_with_an_available_command() {
        let menus = definition(
            rmac_apps::identity::TEXT_EDITOR,
            &["text_editor::FindNext", "text_editor::IncreaseFont"],
        )
        .unwrap();
        assert_eq!(labels(&menus), ["Edit", "Format"]);
        let find = &menus[0].items[0];
        assert_eq!(find.label, "Find");
        assert!(find.is_submenu());
        assert!(!find.separator_before, "a menu never starts with a line");
        assert_eq!(find.children.len(), 1);
        assert_eq!(find.children[0].shortcut, "⌘G");
        // Format keeps Font ▸ but not the absent Monospaced row.
        assert_eq!(menus[1].items.len(), 1);
        assert_eq!(menus[1].items[0].children[0].label, "Bigger");
    }

    #[test]
    fn a_group_keeps_its_separator_when_its_first_row_is_missing() {
        let menus = definition(
            rmac_apps::identity::TEXT_EDITOR,
            &["input::Undo", "input::Copy", "input::Paste"],
        )
        .unwrap();
        let rows = menus[0]
            .items
            .iter()
            .map(|item| (item.label.as_str(), item.separator_before))
            .collect::<Vec<_>>();
        assert_eq!(rows, [("Undo", false), ("Copy", true), ("Paste", false)]);
    }

    #[test]
    fn rmac_apps_get_an_about_row_first_in_the_app_menu() {
        let mut menus = definition(
            rmac_apps::identity::TERMINAL,
            &[ABOUT_ACTION, "terminal::ShowSettings", "terminal::Copy"],
        )
        .unwrap();
        let items = take_application_items(&mut menus);
        assert_eq!(items[0].action, ABOUT_ACTION);
        assert_eq!(items[1].label, "Settings…");
        assert_eq!(items[1].shortcut, "⌘,");
        let calculator = definition(
            rmac_apps::identity::CALCULATOR,
            &[ABOUT_ACTION, "calculator::Copy"],
        )
        .unwrap();
        assert_eq!(labels(&calculator), [APPLICATION_MENU, "Edit"]);
    }

    #[test]
    fn live_state_checks_greys_and_renames_rows() {
        let menus = definition(
            rmac_apps::identity::NOTES,
            &[
                "notes::SortByEdited",
                "notes::SortByTitle",
                "notes::TogglePin",
            ],
        )
        .unwrap();
        let live = apply_state(&menus, |item| match item.action.as_str() {
            "notes::SortByTitle" => Some(ItemState {
                checked: Some(CheckState::On),
                ..ItemState::default()
            }),
            "notes::SortByEdited" => Some(ItemState {
                enabled: Some(false),
                ..ItemState::default()
            }),
            "notes::TogglePin" => Some(ItemState {
                label: Some("Unpin Note".into()),
                ..ItemState::default()
            }),
            _ => None,
        });
        let sort = &live[1].items[0];
        assert!(sort.enabled, "one sort order is still available");
        assert!(!sort.children[0].enabled);
        assert_eq!(sort.children[1].checked, CheckState::On);
        assert_eq!(live[0].items[0].label, "Unpin Note");

        let none_left = apply_state(&menus, |item| {
            item.action.starts_with("notes::SortBy").then(|| ItemState {
                enabled: Some(false),
                ..ItemState::default()
            })
        });
        assert!(!none_left[1].items[0].enabled);
    }

    #[test]
    fn a_new_menu_tree_gets_a_new_revision_and_submenus_are_not_commands() {
        let menus = definition(
            rmac_apps::identity::NOTES,
            &["notes::SortByEdited", "notes::SortByTitle"],
        )
        .unwrap();
        let mut published = Published::new(menus.clone());
        assert!(published.activatable.contains("notes::SortByTitle"));
        assert!(!published.activatable.contains("notes::SortByMenu"));
        assert_eq!(published.replace(menus.clone()), None);
        let checked = apply_state(&menus, |_| {
            Some(ItemState {
                checked: Some(CheckState::On),
                ..ItemState::default()
            })
        });
        assert_eq!(published.replace(checked), Some(2));
    }

    #[test]
    fn window_and_help_items_join_the_standard_menus() {
        let mut menus = definition(
            rmac_apps::identity::FILES,
            &["finder::NextTab", "finder::ShowHelp", "finder::GoBack"],
        )
        .unwrap();
        assert_eq!(take_window_items(&mut menus)[0].action, "finder::NextTab");
        assert_eq!(take_help_items(&mut menus)[0].action, "finder::ShowHelp");
        assert_eq!(labels(&menus), ["Go"]);
    }

    #[test]
    fn spotlight_reaches_commands_inside_submenus() {
        let menus = definition(
            rmac_apps::identity::TERMINAL,
            &["terminal::FindNext", "terminal::Copy"],
        )
        .unwrap();
        let leaves = menus[0].leaves();
        assert_eq!(leaves[0].1.action, "terminal::Copy");
        assert!(leaves[0].0.is_empty());
        assert_eq!(leaves[1].0, ["Find"]);
        assert_eq!(leaves[1].1.label, "Find Next");
    }

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
        // Edit ▸ Find ▸ Find…
        assert_eq!(
            menus[1].items[0].children[0].action,
            "text_editor::ToggleFind"
        );
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn wire_data_is_bounded_and_round_trips() {
        let menus = definition(
            rmac_apps::identity::TERMINAL,
            &["terminal::Copy", "terminal::Paste"],
        )
        .unwrap();
        assert_eq!(
            wire::decode_v1(wire::encode_v1(&wire::flatten_for_v1(&menus))).unwrap(),
            menus
        );
        assert_eq!(wire::decode_v2(wire::encode_v2(&menus)).unwrap(), menus);

        let oversized = vec![Menu {
            label: "x".repeat(MAX_LABEL_BYTES + 1),
            items: vec![Item::new("Item", "terminal::Copy", "")],
        }];
        assert_eq!(validate_menus(&oversized), Err(Error::Protocol));
        let duplicate = vec![Menu {
            label: "Edit".into(),
            items: vec![Item::submenu(
                "Find",
                "terminal::Copy",
                vec![Item::new("Copy", "terminal::Copy", "")],
            )],
        }];
        assert_eq!(validate_menus(&duplicate), Err(Error::Protocol));
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
                "terminal::FindNext",
                "terminal::FindPrevious",
            ],
        )
        .unwrap();
        let terminal_hints = hints(&terminal);
        assert_eq!(terminal_hints["terminal::NewTab"], "⌘T");
        assert_eq!(terminal_hints["terminal::CloseTab"], "⌘W");
        assert_eq!(terminal_hints["terminal::NextTab"], "⇧⌘]");
        assert_eq!(terminal_hints["terminal::PrevTab"], "⇧⌘[");
        assert_eq!(terminal_hints["terminal::Copy"], "⌘C");
        assert_eq!(terminal_hints["terminal::Paste"], "⌘V");
        assert_eq!(terminal_hints["terminal::SelectAll"], "⌘A");
        assert_eq!(terminal_hints["terminal::Find"], "⌘F");
        assert_eq!(terminal_hints["terminal::FindNext"], "⌘G");
        assert_eq!(terminal_hints["terminal::FindPrevious"], "⇧⌘G");

        let notes = definition(rmac_apps::identity::NOTES, &["notes::PrintNote"]).unwrap();
        assert_eq!(hints(&notes)["notes::PrintNote"], "⌘P");

        let editor = definition(
            rmac_apps::identity::TEXT_EDITOR,
            &[
                "text_editor::FindPrev",
                "text_editor::ToggleReplace",
                "text_editor::ToggleMono",
                "input::Undo",
                "input::Redo",
            ],
        )
        .unwrap();
        let editor_hints = hints(&editor);
        assert_eq!(editor_hints["text_editor::FindPrev"], "⇧⌘G");
        assert_eq!(editor_hints["text_editor::ToggleReplace"], "⌥⌘F");
        // Bound to ⇧⌘M, which the menu now says (MENU-07).
        assert_eq!(editor_hints["text_editor::ToggleMono"], "⇧⌘M");
        assert_eq!(editor_hints["input::Undo"], "⌘Z");
        assert_eq!(editor_hints["input::Redo"], "⇧⌘Z");

        let monitor = definition(
            rmac_apps::identity::SYSTEM_MONITOR,
            &["activity_monitor::QuitProcess"],
        )
        .unwrap();
        // The hint matches the ⌘⌫ binding instead of the word "Delete".
        assert_eq!(hints(&monitor)["activity_monitor::QuitProcess"], "⌘⌫");
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
        let actions = spec_actions(PREVIEW_MENUS);
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
        let actions = spec_actions(CLOCK_MENUS);
        let menus = definition(rmac_apps::identity::CLOCK, &actions).unwrap();
        assert_eq!(labels(&menus), ["File", "Edit", "View"]);
        assert_eq!(menus[2].items[2].label, "Stopwatch");
        assert_eq!(menus[2].items[2].shortcut, "⌘3");
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
    fn files_empty_trash_joins_the_app_menu() {
        let mut menus = definition_for_vocabulary(
            rmac_apps::identity::FILES,
            &[
                "finder::EmptyTrash",
                "finder::NewWindow",
                "finder::GoToFolder",
            ],
            rmac_locale::FileVocabulary::for_locale("en_US.UTF-8"),
        )
        .unwrap();
        assert!(validate_menus(&menus).is_ok());
        let items = take_application_items(&mut menus);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Empty Trash…");
        assert_eq!(items[0].shortcut, "⇧⌘⌫");
        assert!(items[0].separator_before);
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.label.as_str())
                .collect::<Vec<_>>(),
            ["File", "Go"]
        );
        assert_eq!(menus[0].items[0].label, "New Finder Window");
        assert_eq!(menus[1].items[0].label, "Go to Folder…");
        assert!(take_application_items(&mut menus).is_empty());
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

    #[test]
    fn terminal_and_preview_list_their_new_commands() {
        let terminal = hints(&definition(TERMINAL_ID, &spec_actions(TERMINAL_MENUS)).unwrap());
        assert_eq!(terminal["terminal::NewWindow"], "⌘N");
        assert_eq!(terminal["terminal::ResetTerminal"], "⌥⌘R");
        assert_eq!(terminal["terminal::HardResetTerminal"], "⌃⌥⌘R");
        assert_eq!(terminal["terminal::ShowSettings"], "⌘,");
        let preview = hints(
            &definition(
                rmac_apps::identity::PREVIEW,
                &spec_actions(PREVIEW_MENUS),
            )
            .unwrap(),
        );
        assert_eq!(preview["preview::GoToPage"], "⌥⌘G");
        assert_eq!(preview["preview::PrintDocument"], "⌘P");
        assert_eq!(preview["preview::SelectAll"], "⌘A");
    }

    const TERMINAL_ID: &str = rmac_apps::identity::TERMINAL;

    #[test]
    fn files_menus_carry_finders_keyboard_commands() {
        let menus = definition_for_vocabulary(
            rmac_apps::identity::FILES,
            &spec_actions(FILES_MENUS),
            rmac_locale::FileVocabulary::for_locale("en_GB.UTF-8"),
        )
        .unwrap();
        let hints = hints(&menus);
        for (action, shortcut) in [
            ("finder::OpenItems", "⌘O"),
            ("finder::Find", "⌘F"),
            ("finder::Duplicate", "⌘D"),
            ("finder::DeletePermanently", "⌥⌘⌫"),
            ("finder::CopyAsPathname", "⌥⌘C"),
            ("finder::MoveItemHere", "⌥⌘V"),
            ("finder::GoRecents", "⇧⌘F"),
            ("finder::GoDocuments", "⇧⌘O"),
            ("finder::GoDesktop", "⇧⌘D"),
        ] {
            assert_eq!(hints[action], shortcut, "{action}");
        }
        let go = menus.iter().find(|menu| menu.label == "Go").unwrap();
        let labels = go
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            &labels[3..6],
            ["Recents", "Documents", "Desktop"],
            "Finder's Go order"
        );
        assert!(validate_menus(&menus).is_ok());
    }
}
