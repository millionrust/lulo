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

pub mod recent;
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
    /// A trailing count capsule, like the Mac's "System Settings…, 1
    /// update". Drawn by the shell's own menus only; never sent over the
    /// menu wire.
    pub badge: String,
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

    /// Show `badge` in a trailing capsule (empty for none).
    pub fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = badge.into();
        self
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

/// The menus with each command's live state from `state`. A submenu defaults
/// to enabled while any child is enabled, but an app may override the parent
/// (for example, Start Recent Timer stays open over its disabled empty row).
pub fn apply_state(menus: &[Menu], state: impl Fn(&Item) -> Option<ItemState>) -> Vec<Menu> {
    fn apply(items: &[Item], state: &dyn Fn(&Item) -> Option<ItemState>) -> Vec<Item> {
        items
            .iter()
            .map(|item| {
                let mut next = item.clone();
                if !item.children.is_empty() {
                    next.children = apply(&item.children, state);
                    next.enabled = next.children.iter().any(|child| child.enabled);
                }
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
/// Plain-text fields already discard source styling when they paste.
pub const PASTE_MATCH_STYLE_ACTION: &str = "rmac_ui::PasteAndMatchStyle";

// Each app's menus follow the Mac app it stands for, as read from macOS
// 26.2's Accessibility tree (docs/parity-audit-2026-09-24-apps.md), minus
// the commands the app does not have. Every shortcut hint is the key the
// app binds. The app menu and the Window menu come from the menu bar.

const TEXT_EDITOR_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: "Application",
        items: &[
            item!("Settings…", "text_editor::ShowSettings", "⌘,", separator),
            item!(
                "Quit and Keep Windows",
                "text_editor::QuitAndKeepWindows",
                "⌥⌘Q"
            ),
        ],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("New", "text_editor::NewFile", "⌘N"),
            item!("Open…", "text_editor::OpenFile", "⌘O"),
            // TE-02: the static child is only a placeholder that keeps this
            // submenu registered; `recent::refresh` replaces it with the
            // real, up-to-ten-document list every time the menu opens.
            submenu!(
                "Open Recent",
                "text_editor::OpenRecentMenu",
                [item!("Clear Menu", "text_editor::ClearRecentMenu", "")]
            ),
            item!("Close", "text_editor::CloseWindow", "⌘W", separator),
            item!("Close All", "text_editor::CloseAll", "⌥⌘W"),
            item!("Save…", "text_editor::SaveFile", "⌘S", separator),
            item!("Duplicate", "text_editor::DuplicateDocument", "⇧⌘S"),
            item!("Save As…", "text_editor::SaveFileAs", "⌥⇧⌘S"),
            item!("Rename…", "text_editor::RenameDocument", "", separator),
            item!("Move To…", "text_editor::MoveToFolder", ""),
            submenu!(
                "Revert To",
                "text_editor::RevertToMenu",
                [item!("Last Saved", "text_editor::RevertToLastSaved", "")]
            ),
            item!("Export as PDF…", "text_editor::ExportPdf", "", separator),
            item!("Page Setup…", "text_editor::OpenPageSetup", "⇧⌘P"),
            item!("Print…", "text_editor::PrintFile", "⌘P"),
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
            item!(
                "Paste and Match Style",
                "rmac_ui::PasteAndMatchStyle",
                "⌥⇧⌘V"
            ),
            item!("Delete", "input::Delete", ""),
            item!("Select All", "input::SelectAll", "⌘A"),
            submenu!(
                "Insert",
                "text_editor::InsertMenu",
                [
                    item!("Line Break", "text_editor::InsertLineBreak", ""),
                    item!("Paragraph Break", "text_editor::InsertParagraphBreak", ""),
                    item!("Page Break", "text_editor::InsertPageBreak", ""),
                ],
                separator
            ),
            submenu!(
                "Find",
                "text_editor::FindMenu",
                [
                    item!("Find…", "text_editor::ToggleFind", "⌘F"),
                    item!("Find and Replace…", "text_editor::ToggleReplace", "⌥⌘F"),
                    item!("Find Next", "text_editor::FindNext", "⌘G"),
                    item!("Find Previous", "text_editor::FindPrev", "⇧⌘G"),
                    item!(
                        "Use Selection for Find",
                        "text_editor::UseSelectionForFind",
                        "⌘E"
                    ),
                    item!("Jump to Selection", "text_editor::JumpToSelection", "⌘J"),
                    item!("Select Line…", "text_editor::SelectLine", "⌘L"),
                ],
                separator
            ),
            submenu!(
                "Transformations",
                "text_editor::TransformationsMenu",
                [
                    item!("Make Uppercase", "text_editor::TransformUppercase", ""),
                    item!("Make Lowercase", "text_editor::TransformLowercase", ""),
                    item!("Capitalise", "text_editor::TransformCapitalise", ""),
                ]
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
                    item!("Smaller", "text_editor::DecreaseFont", "⌘-"),
                ]
            ),
            // Lulo's document model has no per-paragraph attributes
            // (TE-03/TE-14): these alignment/ruler/spacing commands apply
            // to the whole document at once, gated by Make Rich Text below.
            submenu!(
                "Text",
                "text_editor::TextFormatMenu",
                [
                    item!("Align Left", "text_editor::AlignLeft", "⌘{"),
                    item!("Centre", "text_editor::AlignCentre", "⌘|"),
                    item!("Align Right", "text_editor::AlignRight", "⌘}"),
                    item!("Show Ruler", "text_editor::ShowRuler", "⌘R", separator),
                    item!("Copy Ruler", "text_editor::CopyRuler", "⌃⌘C"),
                    item!("Paste Ruler", "text_editor::PasteRuler", "⌃⌘V"),
                    item!("Spacing…", "text_editor::OpenSpacing", ""),
                ]
            ),
            item!("Make Rich Text", "text_editor::ToggleRichText", "⇧⌘T"),
            item!("Wrap to Page", "text_editor::ToggleWrapToPage", "⇧⌘W"),
            item!("Prevent Editing", "text_editor::PreventEditing", ""),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Actual Size", "text_editor::ActualSize", "⌘0"),
            item!("Zoom In", "text_editor::ZoomIn", "⇧⌘."),
            item!("Zoom Out", "text_editor::ZoomOut", "⇧⌘,"),
            item!(
                "Use Dark Background for Windows",
                "text_editor::ToggleDarkBackground",
                "",
                separator
            ),
            item!(
                "Enter Full Screen",
                "text_editor::EnterFullScreen",
                "F",
                separator
            ),
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
            submenu!(
                "New Window",
                "terminal::NewWindowMenu",
                [
                    item!(
                        "New Window with Profile - Basic",
                        "terminal::WindowBasicDefault",
                        "⌘N"
                    ),
                    item!("Basic", "terminal::WindowBasic", ""),
                    item!("Clear Dark", "terminal::WindowClearDark", ""),
                    item!("Clear Light", "terminal::WindowClearLight", ""),
                    item!("Grass", "terminal::WindowGrass", ""),
                    item!("Homebrew", "terminal::WindowHomebrew", ""),
                    item!("Man Page", "terminal::WindowManPage", ""),
                    item!("Novel", "terminal::WindowNovel", ""),
                    item!("Ocean", "terminal::WindowOcean", ""),
                    item!("Pro", "terminal::WindowPro", ""),
                    item!("Red Sands", "terminal::WindowRedSands", ""),
                    item!("Silver Aerogel", "terminal::WindowSilverAerogel", ""),
                    item!("Solid Colors", "terminal::WindowSolidColors", ""),
                ]
            ),
            submenu!(
                "New Tab",
                "terminal::NewTabMenu",
                [
                    item!(
                        "New Tab with Profile – Basic",
                        "terminal::TabBasicDefault",
                        "⌘T"
                    ),
                    item!("Basic", "terminal::TabBasic", ""),
                    item!("Clear Dark", "terminal::TabClearDark", ""),
                    item!("Clear Light", "terminal::TabClearLight", ""),
                    item!("Grass", "terminal::TabGrass", ""),
                    item!("Homebrew", "terminal::TabHomebrew", ""),
                    item!("Man Page", "terminal::TabManPage", ""),
                    item!("Novel", "terminal::TabNovel", ""),
                    item!("Ocean", "terminal::TabOcean", ""),
                    item!("Pro", "terminal::TabPro", ""),
                    item!("Red Sands", "terminal::TabRedSands", ""),
                    item!("Silver Aerogel", "terminal::TabSilverAerogel", ""),
                    item!("Solid Colors", "terminal::TabSolidColors", ""),
                ]
            ),
            item!("Close Window", "terminal::CloseTab", "⌘W", separator),
            item!("Close All", "terminal::CloseAll", "⌥⌘W"),
            item!("Reset", "terminal::ResetTerminal", "⌥⌘R", separator),
            item!("Hard Reset", "terminal::HardResetTerminal", "⌃⌥⌘R"),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "input::Undo", "⌘Z"),
            item!("Redo", "input::Redo", "⇧⌘Z"),
            item!("Cut", "input::Cut", "⌘X", separator),
            item!("Copy", "terminal::Copy", "⌘C"),
            submenu!(
                "Copy Special",
                "terminal::CopySpecialMenu",
                [
                    item!("Copy Plain Text", "terminal::CopyPlainText", "⌥⇧⌘C"),
                    item!(
                        "Copy Without Background Colour",
                        "terminal::CopyWithoutBackgroundColour",
                        "⌃⇧⌘C"
                    ),
                ]
            ),
            item!("Paste", "terminal::Paste", "⌘V"),
            item!("Paste Selection", "terminal::PasteSelection", "⇧⌘V"),
            item!("Paste Escaped Text", "terminal::PasteEscapedText", "⌃⌘V"),
            item!(
                "Paste Escaped Selection",
                "terminal::PasteEscapedSelection",
                "⌃⇧⌘V"
            ),
            item!("Select All", "terminal::SelectAll", "⌘A", separator),
            item!(
                "Select Between Marks",
                "terminal::SelectCommandOutput",
                "⇧⌘A"
            ),
            submenu!(
                "Marks",
                "terminal::MarksMenu",
                [
                    item!("Mark", "terminal::Mark", "⌘U"),
                    item!("Mark as Bookmark", "terminal::MarkAsBookmark", "⌥⌘U"),
                    item!("Unmark", "terminal::Unmark", "⇧⌘U"),
                ]
            ),
            submenu!(
                "Navigate",
                "terminal::NavigateMenu",
                [
                    item!("Jump to Previous Mark", "terminal::PreviousPrompt", "⌘↑"),
                    item!("Jump to Next Mark", "terminal::NextPrompt", "⌘↓"),
                    item!(
                        "Jump to Previous Bookmark",
                        "terminal::PreviousBookmark",
                        "⌥⌘"
                    ),
                    item!("Jump to Next Bookmark", "terminal::NextBookmark", "⌥⌘"),
                    item!(
                        "Select to Previous Mark",
                        "terminal::SelectToPreviousMark",
                        "⇧⌘",
                        separator
                    ),
                    item!("Select to Next Mark", "terminal::SelectToNextMark", "⇧⌘"),
                    item!(
                        "Select to Previous Bookmark",
                        "terminal::SelectToPreviousBookmark",
                        "⌥⇧⌘"
                    ),
                    item!(
                        "Select to Next Bookmark",
                        "terminal::SelectToNextBookmark",
                        "⌥⇧⌘"
                    ),
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
                    item!("Hide Find Bar", "terminal::HideFindBar", "⇧⌘F"),
                    item!(
                        "Use Selection for Find",
                        "terminal::UseSelectionForFind",
                        "⌘E"
                    ),
                    item!("Jump to Selection", "terminal::JumpToSelection", "⌘J"),
                ],
                separator
            ),
            item!("Clear Screen", "terminal::ClearScreen", "⌃⌘L", separator),
            item!("Clear Scrollback", "terminal::ClearScrollback", "⌥⌘K"),
            item!(
                "Use Option as Meta Key",
                "terminal::ToggleOptionAsMeta",
                "⌥⌘O"
            ),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Show Tab Bar", "terminal::ShowTabBar", "⇧⌘T"),
            item!(
                "Allow Mouse Reporting",
                "terminal::AllowMouseReporting",
                "⌘R",
                separator
            ),
            item!("Default Font Size", "terminal::ZoomReset", "⌘0", separator),
            item!("Bigger", "terminal::ZoomIn", "⌘+"),
            item!("Smaller", "terminal::ZoomOut", "⌘-"),
            item!("Scroll to Top", "terminal::ScrollToTop", "⌘", separator),
            item!("Scroll to Bottom", "terminal::ScrollToBottom", "⌘"),
            item!("Page Up", "terminal::PageUp", "⌘"),
            item!("Page Down", "terminal::PageDown", "⌘"),
            item!("Line Up", "terminal::LineUp", "⌥⌘"),
            item!("Line Down", "terminal::LineDown", "⌥⌘"),
            item!(
                "Enter Full Screen",
                "terminal::EnterFullScreen",
                "F",
                separator
            ),
        ],
    },
    MenuSpec {
        label: WINDOW_MENU,
        items: &[
            item!("Show Previous Tab", "terminal::PrevTab", "⇧⌘["),
            item!("Show Next Tab", "terminal::NextTab", "⇧⌘]"),
        ],
    },
    MenuSpec {
        label: "Help",
        items: &[
            item!(
                "Open man Page for Selection",
                "terminal::OpenManPageForSelection",
                "⌃⌘?"
            ),
            item!(
                "Search man Page Index for Selection",
                "terminal::SearchManPageIndexForSelection",
                "⌃⌥⌘/"
            ),
        ],
    },
];

const NOTES_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[
            item!("Settings…", "notes::ShowSettings", "⌘,", separator),
            item!("Quit and Keep Windows", "notes::QuitAndKeepWindows", "⌥⌘Q"),
        ],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("New Note", "notes::ComposeNote", "⌘N"),
            item!("New Folder", "notes::CreateFolder", "⇧⌘N"),
            item!("Close", "rmac_ui::RequestClose", "⌘W", separator),
            item!("Close All", "notes::CloseAll", "⌥⌘W"),
            item!("Import to Notes…", "notes::ImportNote", "", separator),
            item!("Import Markdown...", "notes::ImportMarkdown", ""),
            submenu!(
                "Export as",
                "notes::ExportAsMenu",
                [
                    item!("PDF", "notes::ExportNotePdf", ""),
                    item!("Markdown", "notes::ExportNoteMarkdown", ""),
                ],
                separator
            ),
            item!("Unpin Note", "notes::TogglePin", "", separator),
            item!("Duplicate Note", "notes::DuplicateNote", "⌘D"),
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
            item!("Paste and Match Style", "notes::PastePlainText", "⌥⇧⌘V"),
            item!("Delete Note", "notes::DeleteSelectedNote", "⌫", separator),
            item!("Rename", "notes::RenameSelectedFolder", ""),
            item!("Select All", "input::SelectAll", "⌘A"),
            item!("Add Link…", "notes::InsertLink", "⌘K"),
            submenu!(
                "Find",
                "notes::FindMenu",
                [
                    item!("Find…", "notes::FindInNote", "⌘F"),
                    item!("Note List Search…", "notes::FocusSearch", "⌥⌘F"),
                    item!("Find and Replace…", "notes::FindAndReplace", "⇧⌘F"),
                    item!("Find Next", "notes::FindInNoteNext", "⌘G"),
                    item!("Find Previous", "notes::FindInNotePrevious", "⇧⌘G"),
                    item!("Use Selection for Find", "notes::UseSelectionForFind", "⌘E"),
                    item!("Jump to Selection", "notes::JumpToSelection", "⌘J"),
                ],
                separator
            ),
            submenu!(
                "Transformations",
                "notes::TransformationsMenu",
                [
                    item!("Make Uppercase", "notes::MakeUppercase", ""),
                    item!("Make Lowercase", "notes::MakeLowercase", ""),
                    item!("Capitalise", "notes::Capitalise", ""),
                ]
            ),
        ],
    },
    MenuSpec {
        label: "Format",
        items: &[
            item!("Title", "notes::SetStyleTitle", "⇧⌘T"),
            item!("Heading", "notes::SetStyleHeading", "⇧⌘H"),
            item!("Subheading", "notes::SetStyleSubheading", "⇧⌘J"),
            item!("Body", "notes::SetStyleBody", "⇧⌘B"),
            item!("Monostyled", "notes::SetStyleMonospaced", "⇧⌘M"),
            item!(
                "Bulleted List",
                "notes::InsertBulletedList",
                "⇧⌘7",
                separator
            ),
            item!("Dashed List", "notes::InsertDashedList", "⇧⌘8"),
            item!("Numbered List", "notes::InsertNumberedList", "⇧⌘9"),
            item!("Block Quote", "notes::InsertBlockQuote", "⌘'"),
            item!("Checklist", "notes::InsertChecklist", "⇧⌘L"),
            item!("Mark as Ticked", "notes::ToggleChecklistDone", "⇧⌘U"),
            submenu!(
                "More",
                "notes::ChecklistMoreMenu",
                [
                    item!("Tick All", "notes::TickAll", ""),
                    item!("Untick All", "notes::UntickAll", ""),
                    item!("Move Ticked to Bottom", "notes::MoveTickedToBottom", ""),
                    item!("Delete Ticked", "notes::DeleteTicked", ""),
                ]
            ),
            submenu!(
                "Move Item",
                "notes::MoveItemMenu",
                [
                    item!("Up", "notes::MoveItemUp", "⌃⌘"),
                    item!("Down", "notes::MoveItemDown", "⌃⌘"),
                ]
            ),
            item!("Table", "notes::InsertTable", "⌥⌘T"),
            item!("Convert to Text", "notes::ConvertToText", ""),
            item!(
                "Show Note with Light Background",
                "notes::ToggleLightBackground",
                "",
                separator
            ),
            submenu!(
                "Font",
                "notes::FontMenu",
                [
                    item!("Bold", "notes::ToggleBold", "⌘B"),
                    item!("Italic", "notes::ToggleItalic", "⌘I"),
                    item!("Strikethrough", "notes::ToggleStrikethrough", ""),
                ],
                separator
            ),
            submenu!(
                "Indentation",
                "notes::IndentationMenu",
                [
                    item!("Increase", "notes::IncreaseIndent", "⌘]"),
                    item!("Decrease", "notes::DecreaseIndent", "⌘["),
                ]
            ),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("as List", "notes::ShowListView", "⌘1"),
            item!("as Gallery", "notes::ShowGalleryView", "⌘2"),
            submenu!(
                "Recent Notes",
                "notes::RecentNotesMenu",
                [
                    item!("Previous Note", "notes::PreviousRecentNote", "⌥⌘["),
                    item!("Next Note", "notes::NextRecentNote", "⌥⌘]"),
                    item!("Clear Menu", "notes::ClearRecentNotes", "", separator),
                ]
            ),
            item!("Hide Folders", "notes::ToggleFolders", "⌃⌘S"),
            item!("Hide Note Count", "notes::ToggleNoteCount", ""),
            submenu!(
                "Attachment View",
                "notes::AttachmentViewMenu",
                [
                    item!("Set All to Small", "notes::SetAllAttachmentsSmall", ""),
                    item!("Set All to Large", "notes::SetAllAttachmentsLarge", ""),
                ]
            ),
            item!(
                "Show Attachments Browser",
                "notes::ToggleAttachmentsBrowser",
                "⌘3"
            ),
            item!("Show in Note", "notes::ShowAttachmentInNote", ""),
            item!("Hide Toolbar", "notes::ToggleToolbar", ""),
            item!("Enter Full Screen", "notes::ToggleFullScreen", "F"),
            item!("Zoom In", "notes::ZoomIn", "⇧⌘."),
            item!("Zoom Out", "notes::ZoomOut", "⇧⌘,"),
            item!("Actual Size", "notes::ZoomReset", "⇧⌘0"),
            item!("Expand Section", "notes::ExpandSection", "⌥⌘"),
            item!("Expand All Sections", "notes::ExpandAllSections", "⌥⇧⌘"),
            item!("Collapse Section", "notes::CollapseSection", "⌥⌘"),
            item!(
                "Collapse All Sections",
                "notes::CollapseAllSections",
                "⌥⇧⌘"
            ),
        ],
    },
    MenuSpec {
        label: WINDOW_MENU,
        items: &[item!("Notes", "notes::FocusMainWindow", "⌘0")],
    },
];

const FILES_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[
            item!("Settings…", "finder::ShowSettings", "⌘,", separator),
            item!("Empty Trash…", "finder::EmptyTrash", "⇧⌘⌫"),
            item!("Empty Trash", "finder::EmptyTrashImmediately", "⌥⇧⌘⌫"),
        ],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("New Finder Window", "finder::NewWindow", "⌘N"),
            item!("New Folder", "finder::NewFolder", "⇧⌘N"),
            item!(
                "New Folder with Selection",
                "finder::NewFolderWithSelection",
                "⌃⌘N"
            ),
            item!("New Tab", "finder::NewTab", "⌘T"),
            item!("Open", "finder::OpenItems", "⌘O"),
            item!("Open in New Tab", "finder::OpenSelectionInNewTab", "⌃⌘O"),
            item!(
                "Open in New Window and Close",
                "finder::OpenSelectionInNewWindowAndClose",
                "⌥⌘O"
            ),
            item!("Close Window", "finder::CloseTab", "⌘W"),
            item!("Close All", "finder::CloseAll", "⌥⌘W"),
            item!("Get Info", "finder::GetInfo", "⌘I", separator),
            item!("Quick Look", "finder::QuickLook", "⌘Y"),
            item!("Slideshow", "finder::Slideshow", "⌥⌘Y"),
            item!("Rename", "finder::RenameItem", ""),
            item!("Compress", "finder::Compress", ""),
            item!("Duplicate", "finder::Duplicate", "⌘D"),
            item!("Make Alias", "finder::MakeAlias", "⌘L"),
            item!("Show Original", "finder::ShowOriginal", "⌘R"),
            item!("Add to Sidebar", "finder::AddToSidebar", "⌃⌘T"),
            item!("Add to Dock", "finder::AddToDock", ""),
            item!("Move to Trash", "finder::MoveToTrash", "⌘⌫", separator),
            item!("Eject", "finder::Eject", "⌘E"),
            // The Mac shows this as Move to Bin's ⌥ alternate; the menu bar
            // has no alternates yet, so it is listed after it.
            item!("Delete Immediately…", "finder::DeletePermanently", "⌥⌘⌫"),
            item!("Find", "finder::Find", "⌘F", separator),
            item!("Find by Name…", "finder::FindByName", "⌃⇧⌘F"),
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
            item!("Copy as Link", "finder::CopyAsLink", "⌃⌥⌘C"),
            item!("Paste", "finder::PasteItems", "⌘V"),
            item!("Move Item Here", "finder::MoveItemHere", "⌥⌘V"),
            item!("Select All", "finder::SelectAll", "⌘A"),
            item!("Deselect All", "finder::DeselectAll", "⌥⌘A"),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("as Icons", "finder::ViewAsIcons", "⌘1"),
            item!("as List", "finder::ViewAsList", "⌘2"),
            item!("as Columns", "finder::ViewAsColumns", "⌘3"),
            item!("as Gallery", "finder::ViewAsGallery", "⌘4"),
            item!("Use Groups", "finder::UseGroups", "⌃⌘0"),
            submenu!(
                "Sort By",
                "finder::SortMenu",
                [
                    item!("Name", "finder::SortByName", "⌃⌥⌘1"),
                    item!("Kind", "finder::SortByKind", "⌃⌥⌘2"),
                    item!("Date Modified", "finder::SortByDate", "⌃⌥⌘5"),
                    item!("Size", "finder::SortBySize", "⌃⌥⌘6"),
                ],
                separator
            ),
            item!("Show View Options", "finder::ShowViewOptions", "⌘J"),
            item!("Show Preview", "finder::TogglePreview", "⇧⌘P"),
            item!("Show Tab Bar", "finder::ToggleTabBar", "⇧⌘T"),
            item!("Hide Toolbar", "finder::ToggleToolbar", "⌥⌘T"),
            item!("Enter Full Screen", "finder::EnterFullScreen", "F"),
            item!("Hide Sidebar", "finder::ToggleSidebar", "⌃⌘S", separator),
            item!("Show Path Bar", "finder::TogglePathBar", "⌥⌘P"),
            item!("Hide Status Bar", "finder::ToggleStatusBar", "⌘/"),
        ],
    },
    MenuSpec {
        label: "Go",
        items: &[
            item!("Back", "finder::GoBack", "⌘["),
            item!("Forward", "finder::GoForward", "⌘]"),
            item!("Enclosing Folder", "finder::GoUp", "⌘↑"),
            item!(
                "Enclosing Folder in New Window",
                "finder::GoUpInNewWindow",
                "⌃⌘↑"
            ),
            item!("Recents", "finder::GoRecents", "⇧⌘F", separator),
            item!("Documents", "finder::GoDocuments", "⇧⌘O"),
            item!("Desktop", "finder::GoDesktop", "⇧⌘D"),
            item!("Downloads", "finder::GoDownloads", "⌥⌘L"),
            item!("Home", "finder::GoHome", "⇧⌘H"),
            item!("Computer", "finder::GoComputer", "⇧⌘C"),
            item!("Applications", "finder::GoApplications", "⇧⌘A"),
            item!("Utilities", "finder::GoUtilities", "⇧⌘U"),
            item!("Shared", "finder::GoShared", "⇧⌘S"),
            item!("Trash", "finder::GoTrash", ""),
            item!("Go to Folder…", "finder::GoToFolder", "⇧⌘G", separator),
        ],
    },
    MenuSpec {
        label: "Window",
        items: &[
            item!("Show Previous Tab", "finder::PreviousTab", ""),
            item!("Show Next Tab", "finder::NextTab", ""),
        ],
    },
    MenuSpec {
        label: "Help",
        items: &[item!("Files Help", "finder::ShowHelp", "")],
    },
];

const MONITOR_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: WINDOW_MENU,
        items: &[item!(
            "Activity Monitor",
            "activity_monitor::ShowMainWindow",
            "⌘1"
        )],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("Close", "activity_monitor::Close", "⌘W"),
            item!("Close All", "activity_monitor::CloseAll", "⌥⌘W"),
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
                "activity_monitor::FindMenu",
                [
                    item!("Find…", "activity_monitor::FocusSearch", "⌘F"),
                    item!("Find Next", "activity_monitor::FindNext", "⌘G"),
                    item!("Find Previous", "activity_monitor::FindPrevious", "⇧⌘G"),
                    item!(
                        "Use Selection for Find",
                        "activity_monitor::UseSelectionForFind",
                        "⌘E"
                    ),
                    item!(
                        "Jump to Selection",
                        "activity_monitor::JumpToSelection",
                        "⌘J"
                    ),
                ],
                separator
            ),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            submenu!(
                "Columns",
                "activity_monitor::ColumnsMenu",
                [
                    item!("Process ID", "activity_monitor::TogglePidColumn", ""),
                    item!("User", "activity_monitor::ToggleUserColumn", ""),
                    item!("% CPU", "activity_monitor::ToggleCpuColumn", ""),
                    item!("# Threads", "activity_monitor::ToggleThreadsColumn", ""),
                    item!("Real Memory", "activity_monitor::ToggleMemoryColumn", ""),
                ]
            ),
            submenu!(
                "Update Frequency",
                "activity_monitor::UpdateFrequencyMenu",
                [
                    item!(
                        "Very often (1 sec)",
                        "activity_monitor::RefreshEverySecond",
                        ""
                    ),
                    item!(
                        "Often (2 sec)",
                        "activity_monitor::RefreshEveryTwoSeconds",
                        ""
                    ),
                    item!(
                        "Normally (5 sec)",
                        "activity_monitor::RefreshEveryFiveSeconds",
                        ""
                    ),
                ],
                separator
            ),
            item!("All Processes", "activity_monitor::ShowAllProcesses", ""),
            item!("My Processes", "activity_monitor::ShowMyProcesses", ""),
            item!(
                "System Processes",
                "activity_monitor::ShowSystemProcesses",
                ""
            ),
            item!(
                "Other Users’ Processes",
                "activity_monitor::ShowOtherUsersProcesses",
                ""
            ),
            item!(
                "Active Processes",
                "activity_monitor::ShowActiveProcesses",
                ""
            ),
            item!(
                "Inactive Processes",
                "activity_monitor::ShowInactiveProcesses",
                ""
            ),
            item!(
                "Selected Processes",
                "activity_monitor::ShowSelectedProcesses",
                ""
            ),
            item!(
                "Filter Processes",
                "activity_monitor::FilterProcesses",
                "⌥⌘F",
                separator
            ),
            item!("Inspect Process", "activity_monitor::InspectProcess", "⌘I"),
            item!("Quit Process", "activity_monitor::QuitProcess", "⌥⌘Q"),
            item!(
                "Clear CPU History",
                "activity_monitor::ClearCpuHistory",
                "⌘K",
                separator
            ),
            item!(
                "Enter Full Screen",
                "activity_monitor::EnterFullScreen",
                "F",
                separator
            ),
        ],
    },
];

const SETTINGS_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: WINDOW_MENU,
        items: &[
            item!("Close", "rmac_ui::RequestClose", "⌘W"),
            item!("Close All", "system_settings::CloseAll", "⌥⌘W"),
            item!("Arrange in Front", "system_settings::ArrangeInFront", ""),
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
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Back", "system_settings::GoBack", "⌘["),
            item!("Forward", "system_settings::GoForward", "⌘]"),
            item!("Search", "system_settings::FocusSearch", "⌘F", separator),
            item!("About", "system_settings::ShowAbout", "", separator),
            item!("Accessibility", "system_settings::ShowAccessibility", ""),
            item!("Appearance", "system_settings::ShowAppearance", ""),
            item!("Battery", "system_settings::ShowBattery", ""),
            item!("Bluetooth", "system_settings::ShowBluetooth", ""),
            item!("Date & Time", "system_settings::ShowDateTime", ""),
            item!("Desktop & Dock", "system_settings::ShowDesktopDock", ""),
            item!("Displays", "system_settings::ShowDisplays", ""),
            item!("Focus", "system_settings::ShowFocus", ""),
            item!("Keyboard", "system_settings::ShowKeyboard", ""),
            item!(
                "Language & Region",
                "system_settings::ShowLanguageRegion",
                ""
            ),
            item!("Lock Screen", "system_settings::ShowLockScreen", ""),
            item!(
                "Login Items & Extensions",
                "system_settings::ShowLoginItems",
                ""
            ),
            item!("Menu Bar", "system_settings::ShowMenuBar", ""),
            item!("Network", "system_settings::ShowNetwork", ""),
            item!(
                "Internet Accounts",
                "system_settings::ShowInternetAccounts",
                ""
            ),
            item!("Notifications", "system_settings::ShowNotifications", ""),
            item!(
                "Privacy & Security",
                "system_settings::ShowPrivacySecurity",
                ""
            ),
            item!("Sharing", "system_settings::ShowSharing", ""),
            item!("Software Update", "system_settings::ShowSoftwareUpdate", ""),
            item!("Sound", "system_settings::ShowSound", ""),
            item!("Spotlight", "system_settings::ShowSpotlight", ""),
            item!("Storage", "system_settings::ShowStorage", ""),
            item!("Trackpad", "system_settings::ShowTrackpad", ""),
            item!("Wallpaper", "system_settings::ShowWallpaper", ""),
            item!("Wi-Fi", "system_settings::ShowWifi", ""),
            item!(
                "Enter Full Screen",
                "system_settings::EnterFullScreen",
                "F",
                separator
            ),
        ],
    },
];

const CALENDAR_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[item!("Settings…", "calendar::ShowSettings", "⌘,")],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("New Event", "calendar::NewEvent", "⌘N"),
            item!("New Calendar", "calendar::NewCalendar", "", separator),
            item!(
                "New Calendar Subscription…",
                "calendar::NewCalendarSubscription",
                ""
            ),
            item!("Close Window", "calendar::CloseWindow", "⌘W", separator),
        ],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "calendar::UndoEvent", "⌘Z"),
            item!("Redo", "calendar::RedoEvent", "⇧⌘Z"),
            item!(
                "Show Event Info",
                "calendar::ShowInspector",
                "⌘I",
                separator
            ),
            item!("Delete Event", "calendar::DeleteEvent", "⌫"),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Day", "calendar::ShowDay", "⌘1"),
            item!("Week", "calendar::ShowWeek", "⌘2"),
            item!("Month", "calendar::ShowMonth", "⌘3"),
            item!("Year", "calendar::ShowYear", "⌘4"),
            item!("Go to Today", "calendar::GoToday", "⌘T", separator),
            item!("Previous Period", "calendar::PreviousPeriod", "⌘←"),
            item!("Next Period", "calendar::NextPeriod", "⌘→"),
            item!("Show Sidebar", "calendar::ToggleSidebar", "⌃⌘S", separator),
            item!("Search", "calendar::Search", "⌘F"),
        ],
    },
    MenuSpec {
        label: WINDOW_MENU,
        items: &[item!("Close", "rmac_ui::RequestClose", "⌥⌘W")],
    },
];

const MAIL_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[item!("Settings…", "mail::ShowSettings", "⌘,")],
    },
    MenuSpec {
        label: "Edit",
        items: &[item!("Undo", "mail::Undo", "⌘Z")],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("New Message", "mail::NewMessage", "⌘N"),
            item!("Attach File…", "mail::AttachFile", "⇧⌘A"),
            item!("Close Window", "mail::CloseWindow", "⌘W", separator),
        ],
    },
    MenuSpec {
        label: "Message",
        items: &[
            item!("Send", "mail::SendMessage", "⇧⌘D"),
            item!("Reply", "mail::Reply", "⌘R", separator),
            item!("Reply All", "mail::ReplyAll", "⇧⌘R"),
            item!("Forward", "mail::Forward", "⇧⌘F", separator),
            item!("Mark as Read or Unread", "mail::ToggleRead", "⇧⌘U"),
            item!("Flag", "mail::Flag", "⇧⌘L", separator),
        ],
    },
    MenuSpec {
        label: "Mailbox",
        items: &[
            item!("Archive", "mail::Archive", ""),
            item!("Move to Bin", "mail::Delete", "⌫"),
            item!("Move to Junk", "mail::Junk", "⇧⌘J"),
            item!("Move To…", "mail::Move", "⌃⌘M"),
            item!("Copy To…", "mail::Copy", ""),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Organize by Conversation", "mail::ToggleThreads", ""),
            item!("Filter Unread", "mail::ToggleUnreadFilter", ""),
            item!("Search", "mail::Search", "⌘F"),
        ],
    },
    MenuSpec {
        label: WINDOW_MENU,
        items: &[item!("Close", "rmac_ui::RequestClose", "⌥⌘W")],
    },
];

const CALCULATOR_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[item!(
            "Quit and Keep Windows",
            "calculator::QuitAndKeepWindows",
            "⌥⌘Q"
        )],
    },
    MenuSpec {
        label: "Edit",
        items: &[
            item!("Undo", "input::Undo", "⌘Z"),
            item!("Redo", "input::Redo", "⇧⌘Z"),
            item!("Cut", "input::Cut", "⌘X", separator),
            item!("Copy", "calculator::Copy", "⌘C"),
            item!("Paste", "calculator::Paste", "⌘V"),
            item!("Delete", "input::Delete", ""),
            item!("Select All", "input::SelectAll", "⌘A"),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Basic", "calculator::ShowBasic", "⌘1"),
            item!("Scientific", "calculator::ShowScientific", "⌘2"),
            item!(
                "Hide Thousands Separator",
                "calculator::ToggleThousandsSeparator",
                ""
            ),
            submenu!(
                "Decimal Places",
                "calculator::DecimalPlacesMenu",
                [
                    item!("0", "calculator::DecimalPlaces0", ""),
                    item!("1", "calculator::DecimalPlaces1", ""),
                    item!("2", "calculator::DecimalPlaces2", ""),
                    item!("3", "calculator::DecimalPlaces3", ""),
                    item!("4", "calculator::DecimalPlaces4", ""),
                    item!("5", "calculator::DecimalPlaces5", ""),
                    item!("6", "calculator::DecimalPlaces6", ""),
                    item!("7", "calculator::DecimalPlaces7", ""),
                    item!("8", "calculator::DecimalPlaces8", ""),
                    item!("9", "calculator::DecimalPlaces9", ""),
                    item!("10", "calculator::DecimalPlaces10", ""),
                    item!("11", "calculator::DecimalPlaces11", ""),
                    item!("12", "calculator::DecimalPlaces12", ""),
                    item!("13", "calculator::DecimalPlaces13", ""),
                    item!("14", "calculator::DecimalPlaces14", ""),
                    item!("15", "calculator::DecimalPlaces15", ""),
                ]
            ),
            item!("Show History", "calculator::ShowHistory", "⌃⌘S", separator),
            item!("Enter Full Screen", "calculator::EnterFullScreen", "F"),
        ],
    },
    MenuSpec {
        // Calculator has no File menu; Close is in its Window menu.
        label: WINDOW_MENU,
        items: &[
            item!("Close", "calculator::CloseWindow", "⌘W"),
            item!("Close All", "rmac_ui::RequestClose", "⌥⌘W"),
        ],
    },
];

const PREVIEW_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[item!(
            "Quit and Keep Windows",
            "preview::QuitAndKeepWindows",
            "⌥⌘Q"
        )],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("New from Clipboard", "preview::NewFromClipboard", "⌘N"),
            item!("Open…", "preview::OpenFile", "⌘O"),
            // PREV-08/PREV-15: the static child only keeps this submenu
            // registered; `recent::refresh` replaces it with the real,
            // up-to-ten-document list every time the menu opens.
            submenu!(
                "Open Recent",
                "preview::OpenRecentMenu",
                [item!("Clear Menu", "preview::ClearRecentMenu", "")]
            ),
            item!("Close Window", "preview::CloseWindow", "⌘W", separator),
            item!("Close All", "preview::CloseAll", "⌥⌘W"),
            item!("Close Selected", "preview::CloseSelected", "⇧⌘W"),
            item!("Save", "preview::SaveMarkup", "⌘S"),
            item!("Save As…", "preview::SaveAs", "⌥⇧⌘S"),
            submenu!(
                "Revert To",
                "preview::RevertToMenu",
                [item!("No Document", "preview::RevertMarkup", "")],
                separator
            ),
            // PREV-15: the Mac's File menu also has Duplicate, Rename…, Move To…,
            // Enter Password…, Edit Permissions…, Import from
            // Camera/Scanner, Take Screenshot ▸, Export…, Share ▸ — none of
            // those has a working implementation to wire up yet, so none is
            // listed rather than adding a dead item.
            item!("Export as PDF…", "preview::ExportAsPdf", "", separator),
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
            item!("Delete", "preview::DeleteSelection", ""),
            item!("Select All", "preview::SelectAll", "⌘A"),
            item!("Move to Bin", "preview::MoveToTrash", "⌘⌫"),
            submenu!(
                "Find",
                "preview::FindMenu",
                [
                    item!("Find…", "preview::Find", "⌘F"),
                    item!("Find Next", "preview::FindNext", "⌘G"),
                    item!("Find Previous", "preview::FindPrevious", "⇧⌘G"),
                    item!(
                        "Use Selection for Find",
                        "preview::UseSelectionForFind",
                        "⌘E"
                    ),
                    item!("Jump to Selection", "preview::JumpToSelection", "⌘J"),
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
            item!("Bookmarks", "preview::ShowBookmarks", "⌥⌘5"),
            item!("Actual Size", "preview::ActualSize", "⌘0", separator),
            item!("Actual Size on All", "preview::ActualSizeOnAll", "⌥⌘0"),
            item!("Zoom to Fit", "preview::ZoomToFit", "⌘9"),
            item!("Zoom All to Fit", "preview::ZoomAllToFit", "⌥⌘9"),
            item!("Zoom In", "preview::ZoomIn", "⌘+"),
            item!("Zoom All In", "preview::ZoomAllIn", "⌥⌘+"),
            item!("Zoom Out", "preview::ZoomOut", "⌘−"),
            item!("Zoom All Out", "preview::ZoomAllOut", "⌥⌘−"),
            item!(
                "Show Image Background",
                "preview::ShowImageBackground",
                "⌥⌘B",
                separator
            ),
            item!(
                "Show Markup Toolbar",
                "preview::ToggleMarkup",
                "⇧⌘A",
                separator
            ),
            item!("Enter Full Screen", "preview::EnterFullScreen", "F"),
            item!("Show Toolbar", "preview::ToggleToolbar", "⌥⌘T", separator),
        ],
    },
    MenuSpec {
        label: "Go",
        items: &[
            item!("Back", "preview::Back", "⌘["),
            item!("Forward", "preview::Forward", "⌘]"),
            item!("Up", "preview::PageUp", ""),
            item!("Previous Document", "preview::PreviousDocument", "⌥"),
            item!("Down", "preview::PageDown", ""),
            item!("Next Document", "preview::NextDocument", "⌥"),
            item!("Previous Item", "preview::PreviousItem", "⌥"),
            item!("Next Item", "preview::NextItem", "⌥"),
            item!("Go to Page…", "preview::GoToPage", "⌥⌘G", separator),
        ],
    },
    MenuSpec {
        label: "Tools",
        items: &[
            item!("Show Inspector", "preview::ShowInspector", "⌘I"),
            item!("Add Bookmark", "preview::AddBookmark", "⌘D"),
            submenu!(
                "Annotate",
                "preview::AnnotateMenu",
                [
                    item!("Highlight Text", "preview::AnnotateHighlight", "⌃⌘H"),
                    item!("Underline Text", "preview::AnnotateUnderline", "⌃⌘U"),
                    item!(
                        "Strike Through Text",
                        "preview::AnnotateStrikeThrough",
                        "⌃⌘S"
                    ),
                    item!("Rectangle", "preview::AnnotateRectangle", "⌃⌘R"),
                    item!("Oval", "preview::AnnotateOval", "⌃⌘O"),
                    item!("Line", "preview::AnnotateLine", "⌃⌘I"),
                    item!("Arrow", "preview::AnnotateArrow", "⌃⌘A"),
                    item!("Text", "preview::AnnotateText", "⌃⌘T", separator),
                    item!("Signature", "preview::AnnotateSignature", ""),
                ],
                separator
            ),
            item!("Rotate Left", "preview::RotateLeft", "⌘L", separator),
            item!("Rotate Right", "preview::RotateRight", "⌘R"),
        ],
    },
];

const CLOCK_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[item!(
            "Quit and Keep Windows",
            "clock::QuitAndKeepWindows",
            "⌥⌘Q"
        )],
    },
    MenuSpec {
        label: "File",
        items: &[
            submenu!(
                "Start Recent Timer",
                "clock::RecentTimersMenu",
                [item!("No Recent Timers", "clock::NoRecentTimers", "")]
            ),
            item!("Close", "clock::CloseWindow", "⌘W", separator),
            item!("Close All", "rmac_ui::RequestClose", "⌥⌘W"),
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
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("World Clock", "clock::ShowWorldClock", "⌘1"),
            item!("Alarms", "clock::ShowAlarms", "⌘2"),
            item!("Stopwatch", "clock::ShowStopwatch", "⌘3"),
            item!("Timers", "clock::ShowTimers", "⌘4"),
            item!(
                "View Digital Stopwatch",
                "clock::ShowDigitalStopwatch",
                "",
                separator
            ),
            item!(
                "View Analogue Stopwatch",
                "clock::ShowAnalogueStopwatch",
                ""
            ),
        ],
    },
];

const WEATHER_MENUS: &[MenuSpec] = &[
    MenuSpec {
        label: APPLICATION_MENU,
        items: &[
            item!("Settings…", "weather::ShowSettings", "⌘,", separator),
            item!(
                "Quit and Keep Windows",
                "weather::QuitAndKeepWindows",
                "⌥⌘Q"
            ),
        ],
    },
    MenuSpec {
        label: "File",
        items: &[
            item!("Add Location to List", "weather::AddLocationToList", "⇧⌘L"),
            item!("Close", "weather::CloseWindow", "⌘W"),
            item!("Close All", "rmac_ui::RequestClose", "⌥⌘W"),
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
            item!(
                "Paste and Match Style",
                "rmac_ui::PasteAndMatchStyle",
                "⌥⇧⌘V"
            ),
            item!("Delete", "input::Delete", ""),
            item!("Select All", "input::SelectAll", "⌘A"),
            item!("Search", "weather::FindCity", "⌘F", separator),
        ],
    },
    MenuSpec {
        label: "View",
        items: &[
            item!("Celsius", "weather::UseCelsius", ""),
            item!("Fahrenheit", "weather::UseFahrenheit", ""),
            item!("Hide Sidebar", "weather::ToggleSidebar", "⌃⌘S"),
            item!("Enter Full Screen", "weather::ToggleFullScreen", "F"),
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
        rmac_apps::identity::CALENDAR => Some(CALENDAR_MENUS),
        rmac_apps::identity::MAIL => Some(MAIL_MENUS),
        rmac_apps::identity::PREVIEW => Some(PREVIEW_MENUS),
        rmac_apps::identity::CLOCK => Some(CLOCK_MENUS),
        rmac_apps::identity::WEATHER => Some(WEATHER_MENUS),
        rmac_apps::identity::PLAYER => Some(PLAYER_MENUS),
        _ => None,
    }
}

/// The action-namespace prefix `app_id`'s menu table uses (`"text_editor"`,
/// `"preview"`, …), for [`recent::refresh`] — `None` for an app with no
/// Open Recent submenu.
pub fn recent_documents_prefix(app_id: &str) -> Option<&'static str> {
    match app_id {
        rmac_apps::identity::TEXT_EDITOR => Some("text_editor"),
        rmac_apps::identity::PREVIEW => Some("preview"),
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
        rmac_apps::identity::CALENDAR => Some("org.rmac.Calendar.Menu"),
        rmac_apps::identity::MAIL => Some("org.rmac.Mail.Menu"),
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
    rmac_apps::identity::CALENDAR,
    rmac_apps::identity::MAIL,
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
                "finder::EmptyTrashImmediately" => format!("Empty {}", file_words.bin()),
                _ => spec.label.to_owned(),
            },
            action: spec.action.to_owned(),
            shortcut: spec.shortcut.to_owned(),
            enabled: true,
            separator_before: separate && !items.is_empty(),
            checked: CheckState::Off,
            children,
            badge: String::new(),
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
            Self::Bus(detail) => write!(
                formatter,
                "application menu D-Bus call failed: {}",
                log_safe(detail)
            ),
            Self::Protocol => formatter.write_str("application menu data is invalid"),
        }
    }
}

impl std::error::Error for Error {}

/// Peer-supplied D-Bus error text reaches the journal through this Display:
/// control characters cannot forge extra journal lines or terminal escapes,
/// and the text is bounded.
fn log_safe(detail: &str) -> String {
    const LIMIT: usize = 240;
    let mut safe: String = detail
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(LIMIT)
        .collect();
    if detail.chars().count() > LIMIT {
        safe.push('…');
    }
    safe
}

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

    #[test]
    fn peer_supplied_bus_error_text_cannot_forge_journal_lines() {
        let error = Error::Bus(format!(
            "x\nrmac-files: forged\u{1b}[31m{}",
            "a".repeat(400)
        ));
        let shown = error.to_string();
        assert!(!shown.chars().any(char::is_control));
        assert!(shown.chars().count() < 300);
        assert!(shown.ends_with('…'));
    }

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
                "notes::ToggleBold",
                "notes::ToggleItalic",
                "notes::TogglePin",
            ],
        )
        .unwrap();
        let live = apply_state(&menus, |item| match item.action.as_str() {
            "notes::ToggleItalic" => Some(ItemState {
                checked: Some(CheckState::On),
                ..ItemState::default()
            }),
            "notes::ToggleBold" => Some(ItemState {
                enabled: Some(false),
                ..ItemState::default()
            }),
            "notes::TogglePin" => Some(ItemState {
                label: Some("Unpin Note".into()),
                ..ItemState::default()
            }),
            _ => None,
        });
        let font = &live
            .iter()
            .find(|menu| menu.label == "Format")
            .unwrap()
            .items[0];
        assert!(font.enabled, "one font action is still available");
        assert!(!font.children[0].enabled);
        assert_eq!(font.children[1].checked, CheckState::On);
        assert_eq!(live[0].items[0].label, "Unpin Note");

        let none_left = apply_state(&menus, |item| {
            matches!(
                item.action.as_str(),
                "notes::ToggleBold" | "notes::ToggleItalic"
            )
            .then(|| ItemState {
                enabled: Some(false),
                ..ItemState::default()
            })
        });
        assert!(
            !none_left
                .iter()
                .find(|menu| menu.label == "Format")
                .unwrap()
                .items[0]
                .enabled
        );

        let parent_override = apply_state(&menus, |item| {
            matches!(
                item.action.as_str(),
                "notes::ToggleBold" | "notes::ToggleItalic" | "notes::FontMenu"
            )
            .then(|| ItemState {
                enabled: Some(item.action == "notes::FontMenu"),
                ..ItemState::default()
            })
        });
        assert!(
            parent_override
                .iter()
                .find(|menu| menu.label == "Format")
                .unwrap()
                .items[0]
                .enabled
        );
    }

    #[test]
    fn a_new_menu_tree_gets_a_new_revision_and_submenus_are_not_commands() {
        let menus = definition(
            rmac_apps::identity::NOTES,
            &["notes::ToggleBold", "notes::ToggleItalic"],
        )
        .unwrap();
        let mut published = Published::new(menus.clone());
        assert!(published.activatable.contains("notes::ToggleItalic"));
        assert!(!published.activatable.contains("notes::FontMenu"));
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
        assert_eq!(menus[0].items[0].label, "Save…");
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
                "terminal::TabBasicDefault",
                "terminal::CloseTab",
                "terminal::CloseAll",
                "terminal::NextTab",
                "terminal::PrevTab",
                "terminal::Copy",
                "terminal::CopyPlainText",
                "terminal::ShowTabBar",
                "terminal::AllowMouseReporting",
                "terminal::Paste",
                "terminal::SelectAll",
                "terminal::Find",
                "terminal::FindNext",
                "terminal::FindPrevious",
            ],
        )
        .unwrap();
        let terminal_hints = hints(&terminal);
        assert_eq!(terminal_hints["terminal::TabBasicDefault"], "⌘T");
        assert_eq!(terminal_hints["terminal::CloseTab"], "⌘W");
        assert_eq!(terminal_hints["terminal::CloseAll"], "⌥⌘W");
        assert_eq!(terminal_hints["terminal::NextTab"], "⇧⌘]");
        assert_eq!(terminal_hints["terminal::PrevTab"], "⇧⌘[");
        assert_eq!(terminal_hints["terminal::Copy"], "⌘C");
        assert_eq!(terminal_hints["terminal::CopyPlainText"], "⌥⇧⌘C");
        assert_eq!(terminal_hints["terminal::ShowTabBar"], "⇧⌘T");
        assert_eq!(terminal_hints["terminal::AllowMouseReporting"], "⌘R");
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
        // The app retains its private font toggle, but the Mac has no
        // Monospaced menu row to advertise it.
        assert!(!editor_hints.contains_key("text_editor::ToggleMono"));
        assert_eq!(editor_hints["input::Undo"], "⌘Z");
        assert_eq!(editor_hints["input::Redo"], "⇧⌘Z");

        let monitor = definition(
            rmac_apps::identity::SYSTEM_MONITOR,
            &["activity_monitor::QuitProcess"],
        )
        .unwrap();
        assert_eq!(hints(&monitor)["activity_monitor::QuitProcess"], "⌥⌘Q");
    }

    #[test]
    fn notes_menu_shortcuts_reach_their_format_and_find_commands() {
        let menus = definition(rmac_apps::identity::NOTES, &spec_actions(NOTES_MENUS)).unwrap();
        let shortcuts = hints(&menus);
        for (action, shortcut) in [
            ("notes::ShowSettings", "⌘,"),
            ("notes::FindAndReplace", "⇧⌘F"),
            ("notes::FindInNoteNext", "⌘G"),
            ("notes::UseSelectionForFind", "⌘E"),
            ("notes::SetStyleTitle", "⇧⌘T"),
            ("notes::InsertBulletedList", "⇧⌘7"),
            ("notes::InsertBlockQuote", "⌘'"),
            ("notes::ToggleFolders", "⌃⌘S"),
            ("notes::ZoomReset", "⇧⌘0"),
            ("notes::CloseAll", "⌥⌘W"),
            ("notes::FocusMainWindow", "⌘0"),
        ] {
            assert_eq!(shortcuts[action], shortcut, "{action}");
        }
        let format = menus.iter().find(|menu| menu.label == "Format").unwrap();
        assert!(format.items.iter().any(|item| item.label == "Title"));
        let font = format
            .items
            .iter()
            .find(|item| item.label == "Font")
            .unwrap();
        assert!(font.children.iter().any(|item| item.label == "Bold"));
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
        assert_eq!(menus[2].items[1].shortcut, "");
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
            ["Application", "File", "Edit", "View", "Go", "Tools"]
        );
        let shortcut = |label: &str| {
            menus
                .iter()
                .flat_map(|menu| &menu.items)
                .find(|item| item.label == label)
                .map(|item| item.shortcut.clone())
        };
        assert_eq!(shortcut("Hide Sidebar").as_deref(), Some("⌥⌘1"));
        assert_eq!(shortcut("Bookmarks").as_deref(), Some("⌥⌘5"));
        assert_eq!(shortcut("Add Bookmark").as_deref(), Some("⌘D"));
        assert_eq!(shortcut("Quit and Keep Windows").as_deref(), Some("⌥⌘Q"));
        assert_eq!(shortcut("Close All").as_deref(), Some("⌥⌘W"));
        assert_eq!(shortcut("Close Selected").as_deref(), Some("⇧⌘W"));
        assert_eq!(shortcut("Move to Bin").as_deref(), Some("⌘⌫"));
        assert_eq!(shortcut("Actual Size").as_deref(), Some("⌘0"));
        assert_eq!(shortcut("Rotate Right").as_deref(), Some("⌘R"));
        assert_eq!(shortcut("Next Item").as_deref(), Some("⌥"));
        assert_eq!(shortcut("Zoom All to Fit").as_deref(), Some("⌥⌘9"));
        assert_eq!(shortcut("Show Markup Toolbar").as_deref(), Some("⇧⌘A"));
        let annotate = menus
            .iter()
            .find(|menu| menu.label == "Tools")
            .and_then(|menu| menu.items.iter().find(|item| item.label == "Annotate"))
            .unwrap();
        let rectangle = annotate
            .children
            .iter()
            .find(|item| item.label == "Rectangle")
            .unwrap();
        assert_eq!(rectangle.shortcut, "⌃⌘R");
        assert_eq!(rectangle.action, "preview::AnnotateRectangle");
        assert_eq!(shortcut("Delete").as_deref(), Some(""));
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
        assert_eq!(labels(&menus), ["Application", "File", "Edit", "View"]);
        assert_eq!(menus[0].items[0].label, "Quit and Keep Windows");
        assert_eq!(menus[0].items[0].shortcut, "⌥⌘Q");
        assert_eq!(menus[3].items[2].label, "Stopwatch");
        assert_eq!(menus[3].items[2].shortcut, "⌘3");
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
            ["Application", "File", "Edit", "View"]
        );
        assert_eq!(menus[0].items[0].shortcut, "⌘,");
        assert_eq!(menus[0].items[1].label, "Quit and Keep Windows");
        assert_eq!(menus[0].items[1].shortcut, "⌥⌘Q");
        assert_eq!(menus[3].items[2].label, "Hide Sidebar");
        assert_eq!(menus[3].items[2].shortcut, "⌃⌘S");
        assert_eq!(menus[3].items.len(), 4);
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
    fn calculator_exports_quit_and_keep_windows_when_registered() {
        let actions = spec_actions(CALCULATOR_MENUS);
        let menus = definition(rmac_apps::identity::CALCULATOR, &actions).unwrap();
        let application = menus
            .iter()
            .find(|menu| menu.label == APPLICATION_MENU)
            .expect("Application menu");
        assert_eq!(application.items[0].label, "Quit and Keep Windows");
        assert_eq!(
            application.items[0].action,
            "calculator::QuitAndKeepWindows"
        );
        assert_eq!(application.items[0].shortcut, "⌥⌘Q");
        assert!(validate_menus(&menus).is_ok());
    }

    #[test]
    fn calendar_menu_actions_are_distinct_and_week_is_exposed() {
        let actions = spec_actions(CALENDAR_MENUS);
        let menus = definition(rmac_apps::identity::CALENDAR, &actions).unwrap();
        assert!(validate_menus(&menus).is_ok());
        let view = menus.iter().find(|menu| menu.label == "View").unwrap();
        assert_eq!(view.items[1].action, "calendar::ShowWeek");
        assert_eq!(view.items[1].shortcut, "⌘2");
        assert!(view
            .items
            .iter()
            .any(|item| item.action == "calendar::GoToday"));
    }

    #[test]
    fn mail_menu_actions_are_distinct_and_conversation_toggle_is_exposed() {
        let actions = spec_actions(MAIL_MENUS);
        let menus = definition(rmac_apps::identity::MAIL, &actions).unwrap();
        assert!(validate_menus(&menus).is_ok());
        assert_eq!(
            bus_name(rmac_apps::identity::MAIL),
            Some("org.rmac.Mail.Menu")
        );
        assert!(menus.iter().any(|menu| menu
            .items
            .iter()
            .any(|item| { item.action == "mail::ToggleThreads" })));
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
    fn british_files_menu_uses_bin_for_every_trash_action() {
        let menus = definition_for_vocabulary(
            rmac_apps::identity::FILES,
            &[
                "finder::MoveToTrash",
                "finder::GoTrash",
                "finder::EmptyTrash",
                "finder::EmptyTrashImmediately",
            ],
            rmac_locale::FileVocabulary::for_locale("en_GB.UTF-8"),
        )
        .unwrap();
        let labels = menus
            .iter()
            .flat_map(|menu| &menu.items)
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        for label in ["Move to Bin", "Bin", "Empty Bin…", "Empty Bin"] {
            assert!(labels.contains(&label), "missing {label}: {labels:?}");
        }
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
        assert_eq!(terminal["terminal::WindowBasicDefault"], "⌘N");
        assert_eq!(terminal["terminal::ResetTerminal"], "⌥⌘R");
        assert_eq!(terminal["terminal::HardResetTerminal"], "⌃⌥⌘R");
        assert_eq!(terminal["terminal::ShowSettings"], "⌘,");
        let preview =
            hints(&definition(rmac_apps::identity::PREVIEW, &spec_actions(PREVIEW_MENUS)).unwrap());
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
            ("finder::CopyAsLink", "⌃⌥⌘C"),
            ("finder::DeselectAll", "⌥⌘A"),
            ("finder::OpenSelectionInNewTab", "⌃⌘O"),
            ("finder::OpenSelectionInNewWindowAndClose", "⌥⌘O"),
            ("finder::CloseAll", "⌥⌘W"),
            ("finder::QuickLook", "⌘Y"),
            ("finder::GoUpInNewWindow", "⌃⌘↑"),
            ("finder::FindByName", "⌃⇧⌘F"),
            ("finder::SortByName", "⌃⌥⌘1"),
            ("finder::SortByKind", "⌃⌥⌘2"),
            ("finder::SortByDate", "⌃⌥⌘5"),
            ("finder::SortBySize", "⌃⌥⌘6"),
            ("finder::TogglePreview", "⇧⌘P"),
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
        let recents = labels.iter().position(|label| *label == "Recents").unwrap();
        assert_eq!(
            &labels[recents..recents + 3],
            ["Recents", "Documents", "Desktop"],
            "Finder's Go order"
        );
        assert!(validate_menus(&menus).is_ok());
    }
}
