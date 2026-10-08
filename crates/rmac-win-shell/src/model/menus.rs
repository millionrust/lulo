//! The menu bar's own menus: the Lulo menu at the left, the app and
//! Window menus it builds for a Windows app that has no Lulo menus, and,
//! with the desktop in front, the desktop's own menus and context menus
//! (Lulo mode, where the Mac shows the Finder's).

use rmac_app_menu::{CheckState, Item, Menu};

pub const ABOUT: &str = "lulo::AboutThisPc";
pub const SYSTEM_SETTINGS: &str = "lulo::SystemSettings";
pub const FORCE_QUIT: &str = "lulo::ForceQuit";
pub const SLEEP: &str = "lulo::Sleep";
pub const RESTART: &str = "lulo::Restart";
pub const SHUT_DOWN: &str = "lulo::ShutDown";
pub const LOCK_SCREEN: &str = "lulo::LockScreen";
pub const LOG_OUT: &str = "lulo::LogOut";
pub const START_AT_SIGN_IN: &str = "lulo::StartAtSignIn";
pub const TURN_OFF: &str = "lulo::TurnOffLulo";
pub const FILES_FOR_FOLDERS: &str = "lulo::FilesForFolders";

pub const HIDE_APP: &str = "lulo::HideApp";
pub const QUIT_APP: &str = "lulo::QuitApp";
pub const MINIMIZE: &str = "lulo::Minimize";
pub const ZOOM: &str = "lulo::Zoom";
pub const CLOSE_WINDOW: &str = "lulo::CloseWindow";

pub const OPEN_RECYCLE_BIN: &str = "lulo::OpenRecycleBin";
pub const EMPTY_RECYCLE_BIN: &str = "lulo::EmptyRecycleBin";

pub const DESKTOP_NEW_WINDOW: &str = "desktop::NewFinderWindow";
pub const DESKTOP_NEW_FOLDER: &str = "desktop::NewFolder";
pub const DESKTOP_OPEN: &str = "desktop::Open";
pub const DESKTOP_RECYCLE: &str = "desktop::MoveToRecycleBin";
pub const DESKTOP_INFO: &str = "desktop::GetInfo";
pub const DESKTOP_RENAME: &str = "desktop::Rename";
pub const DESKTOP_DUPLICATE: &str = "desktop::Duplicate";
pub const DESKTOP_SHORTCUT: &str = "desktop::CreateShortcut";
pub const DESKTOP_SELECT_ALL: &str = "desktop::SelectAll";
pub const DESKTOP_CLEAN_UP: &str = "desktop::CleanUp";
pub const DESKTOP_SORT_NONE: &str = "desktop::SortNone";
pub const DESKTOP_SORT_NAME: &str = "desktop::SortName";
pub const DESKTOP_SORT_KIND: &str = "desktop::SortKind";
pub const DESKTOP_SORT_DATE: &str = "desktop::SortDateModified";
pub const DESKTOP_SORT_SIZE: &str = "desktop::SortSize";
pub const DESKTOP_CHANGE_WALLPAPER: &str = "desktop::ChangeWallpaper";
pub const GO_DESKTOP: &str = "desktop::GoDesktop";
pub const GO_DOCUMENTS: &str = "desktop::GoDocuments";
pub const GO_DOWNLOADS: &str = "desktop::GoDownloads";
pub const GO_HOME: &str = "desktop::GoHome";
pub const GO_RECENTS: &str = "desktop::GoRecents";

/// The desktop's Sort By choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopSort {
    None,
    Name,
    Kind,
    DateModified,
    Size,
}

/// Whether `action` is one of the desktop's commands.
pub fn is_desktop_command(action: &str) -> bool {
    action.starts_with("desktop::")
}

fn sort_items(current: DesktopSort) -> Vec<Item> {
    let row = |label: &str, action: &str, sort: DesktopSort| {
        Item::new(label, action, "").checked(if current == sort {
            CheckState::On
        } else {
            CheckState::Off
        })
    };
    vec![
        row("None", DESKTOP_SORT_NONE, DesktopSort::None),
        row("Name", DESKTOP_SORT_NAME, DesktopSort::Name).separated(),
        row("Kind", DESKTOP_SORT_KIND, DesktopSort::Kind),
        row(
            "Date Modified",
            DESKTOP_SORT_DATE,
            DesktopSort::DateModified,
        ),
        row("Size", DESKTOP_SORT_SIZE, DesktopSort::Size),
    ]
}

/// Right-click on the desktop's wallpaper, as the Finder's desktop menu.
pub fn desktop_background_menu(sort: DesktopSort) -> Vec<Item> {
    vec![
        Item::new("New Folder", DESKTOP_NEW_FOLDER, ""),
        Item::new("Change Wallpaper…", DESKTOP_CHANGE_WALLPAPER, "").separated(),
        Item::submenu("Sort By", "desktop::SortBy", sort_items(sort)).separated(),
        Item::new("Clean Up", DESKTOP_CLEAN_UP, ""),
    ]
}

/// Right-click on desktop icons: `selected` items, of which none or some
/// are folders (Duplicate copies files only) or on the shared desktop
/// (which Lulo does not rename).
pub fn desktop_item_menu(selected: usize, only_files: bool, any_shared: bool) -> Vec<Item> {
    vec![
        Item::new("Open", DESKTOP_OPEN, ""),
        Item::new("Move to Recycle Bin", DESKTOP_RECYCLE, "").separated(),
        Item::new("Get Info", DESKTOP_INFO, "").separated(),
        Item::new("Rename", DESKTOP_RENAME, "").enabled(selected == 1 && !any_shared),
        Item::new("Duplicate", DESKTOP_DUPLICATE, "").enabled(only_files),
        Item::new("Create Shortcut", DESKTOP_SHORTCUT, ""),
    ]
}

/// The Recycle Bin tile's menu, as the Mac's Trash tile has Open and
/// Empty Trash. Windows asks before it empties the bin.
pub fn recycle_bin_menu(full: bool) -> Vec<Item> {
    let mut empty = Item::new("Empty Recycle Bin", EMPTY_RECYCLE_BIN, "").separated();
    empty.enabled = full;
    vec![Item::new("Open", OPEN_RECYCLE_BIN, ""), empty]
}

/// The Lulo menu, the Apple menu's place: the same rows as the Mac's, each
/// doing the Windows equivalent, plus the Lulo layer's own switches.
pub fn lulo_menu(user: &str, starts_at_sign_in: bool, files_for_folders: bool) -> Menu {
    let log_out = if user.is_empty() {
        "Log Out…".to_owned()
    } else {
        format!("Log Out {user}…")
    };
    Menu {
        label: "Lulo".to_owned(),
        items: vec![
            Item::new("About This PC", ABOUT, ""),
            Item::new("System Settings…", SYSTEM_SETTINGS, "").separated(),
            Item::new("Force Quit…", FORCE_QUIT, "").separated(),
            Item::new("Sleep", SLEEP, "").separated(),
            Item::new("Restart…", RESTART, ""),
            Item::new("Shut Down…", SHUT_DOWN, ""),
            Item::new("Lock Screen", LOCK_SCREEN, "").separated(),
            Item::new(log_out, LOG_OUT, ""),
            Item::new("Start Lulo at Sign-In", START_AT_SIGN_IN, "")
                .separated()
                .checked(if starts_at_sign_in {
                    CheckState::On
                } else {
                    CheckState::Off
                }),
            Item::new("Use Files for Folders", FILES_FOR_FOLDERS, "").checked(
                if files_for_folders {
                    CheckState::On
                } else {
                    CheckState::Off
                },
            ),
            Item::new("Turn Off Lulo", TURN_OFF, ""),
        ],
    }
}

/// The menus for a Windows app, whose own menus stay in its window: its
/// name with Hide and Quit, and the standard Window menu.
pub fn windows_app_menus(app_name: &str) -> Vec<Menu> {
    vec![
        Menu {
            label: app_name.to_owned(),
            items: vec![
                Item::new(format!("Hide {app_name}"), HIDE_APP, ""),
                Item::new(format!("Quit {app_name}"), QUIT_APP, "").separated(),
            ],
        },
        Menu {
            label: "Window".to_owned(),
            items: vec![
                Item::new("Minimize", MINIMIZE, ""),
                Item::new("Zoom", ZOOM, ""),
                Item::new("Close Window", CLOSE_WINDOW, "").separated(),
            ],
        },
    ]
}

/// The menus for the desktop itself (nothing in front), where the Mac
/// shows the Finder's: Lulo mode's desktop is Files' desktop, so the bar
/// shows Files with the desktop's commands. `selected` desktop items are
/// selected; `only_files` when none of them is a folder.
pub fn desktop_menus(selected: usize, only_files: bool, sort: DesktopSort) -> Vec<Menu> {
    let with_selection = |item: Item| item.enabled(selected > 0);
    vec![
        Menu {
            label: "Files".to_owned(),
            items: vec![
                Item::new("New Finder Window", DESKTOP_NEW_WINDOW, ""),
                Item::new("Empty Recycle Bin…", EMPTY_RECYCLE_BIN, "").separated(),
            ],
        },
        Menu {
            label: "File".to_owned(),
            items: vec![
                Item::new("New Folder", DESKTOP_NEW_FOLDER, ""),
                with_selection(Item::new("Open", DESKTOP_OPEN, "")).separated(),
                with_selection(Item::new("Move to Recycle Bin", DESKTOP_RECYCLE, "")).separated(),
                with_selection(Item::new("Get Info", DESKTOP_INFO, "")).separated(),
                Item::new("Rename", DESKTOP_RENAME, "").enabled(selected == 1),
                Item::new("Duplicate", DESKTOP_DUPLICATE, "").enabled(selected > 0 && only_files),
                with_selection(Item::new("Create Shortcut", DESKTOP_SHORTCUT, "")),
            ],
        },
        Menu {
            label: "Edit".to_owned(),
            items: vec![Item::new("Select All", DESKTOP_SELECT_ALL, "")],
        },
        Menu {
            label: "View".to_owned(),
            items: vec![
                Item::submenu("Sort By", "desktop::SortBy", sort_items(sort)),
                Item::new("Clean Up", DESKTOP_CLEAN_UP, "").separated(),
            ],
        },
        Menu {
            label: "Go".to_owned(),
            items: vec![
                Item::new("Recents", GO_RECENTS, ""),
                Item::new("Desktop", GO_DESKTOP, "").separated(),
                Item::new("Documents", GO_DOCUMENTS, ""),
                Item::new("Downloads", GO_DOWNLOADS, ""),
                Item::new("Home", GO_HOME, ""),
            ],
        },
    ]
}

/// A command that asks first, as the Mac does: (message, detail, button).
pub fn confirmation(action: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match action {
        RESTART => Some((
            "Are you sure you want to restart your computer now?",
            "Apps that are open will be asked to close.",
            "Restart",
        )),
        SHUT_DOWN => Some((
            "Are you sure you want to shut down your computer now?",
            "Apps that are open will be asked to close.",
            "Shut Down",
        )),
        LOG_OUT => Some((
            "Are you sure you want to quit all applications and log out now?",
            "Apps that are open will be asked to close.",
            "Log Out",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(menus: &[Menu]) -> Vec<String> {
        menus
            .iter()
            .flat_map(Menu::leaves)
            .map(|(_, item)| item.action.clone())
            .collect()
    }

    #[test]
    fn the_lulo_menu_has_the_macs_rows_each_distinct() {
        let menu = lulo_menu("Ada", true, false);
        let labels = menu
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "About This PC",
                "System Settings…",
                "Force Quit…",
                "Sleep",
                "Restart…",
                "Shut Down…",
                "Lock Screen",
                "Log Out Ada…",
                "Start Lulo at Sign-In",
                "Use Files for Folders",
                "Turn Off Lulo"
            ]
        );
        assert_eq!(menu.items[8].checked, CheckState::On);
        assert_eq!(menu.items[9].checked, CheckState::Off);
        assert_eq!(lulo_menu("", false, true).items[9].checked, CheckState::On);
        assert_eq!(lulo_menu("", false, false).items[7].label, "Log Out…");
        let mut all = actions(&[menu]);
        let count = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), count);
        rmac_app_menu::validate_menus(&[lulo_menu("Ada", false, false)]).unwrap();
    }

    #[test]
    fn the_desktop_shows_files_menus_with_distinct_commands() {
        let menus = desktop_menus(0, true, DesktopSort::Name);
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.label.as_str())
                .collect::<Vec<_>>(),
            ["Files", "File", "Edit", "View", "Go"]
        );
        rmac_app_menu::validate_menus(&menus).unwrap();
        let mut all = actions(&menus);
        let count = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), count);
        // Nothing selected: the commands on a selection are off.
        let open = &menus[1].items[1];
        assert_eq!(open.label, "Open");
        assert!(!open.enabled);
        assert!(desktop_menus(1, true, DesktopSort::None)[1].items[1].enabled);
        assert!(is_desktop_command(DESKTOP_RENAME));
        assert!(!is_desktop_command(TURN_OFF));
    }

    #[test]
    fn desktop_context_menus_tick_the_sort_and_guard_rename() {
        let background = desktop_background_menu(DesktopSort::Kind);
        assert_eq!(background[0].label, "New Folder");
        let sort = &background[2].children;
        assert_eq!(sort[2].label, "Kind");
        assert_eq!(sort[2].checked, CheckState::On);
        assert_eq!(sort[1].checked, CheckState::Off);
        let items = desktop_item_menu(2, true, false);
        assert!(!items[3].enabled, "Rename needs one item");
        assert!(desktop_item_menu(1, false, false)[3].enabled);
        assert!(
            !desktop_item_menu(1, false, false)[4].enabled,
            "folders are not duplicated"
        );
        assert!(
            !desktop_item_menu(1, true, true)[3].enabled,
            "the shared desktop is not renamed"
        );
        for menu in [background, items] {
            let mut actions = menu
                .iter()
                .map(|item| item.action.clone())
                .collect::<Vec<_>>();
            let count = actions.len();
            actions.sort();
            actions.dedup();
            assert_eq!(actions.len(), count);
        }
    }

    #[test]
    fn windows_apps_get_their_name_and_a_window_menu() {
        let menus = windows_app_menus("Notepad");
        assert_eq!(menus[0].label, "Notepad");
        assert_eq!(menus[0].items[1].label, "Quit Notepad");
        assert_eq!(menus[1].label, "Window");
        rmac_app_menu::validate_menus(&menus).unwrap();
    }

    #[test]
    fn the_recycle_bin_menu_empties_only_a_full_bin() {
        let full = recycle_bin_menu(true);
        assert_eq!(full[0].label, "Open");
        assert_eq!(full[1].label, "Empty Recycle Bin");
        assert!(full[1].enabled);
        assert!(!recycle_bin_menu(false)[1].enabled);
        assert_ne!(full[0].action, full[1].action);
    }

    #[test]
    fn power_commands_ask_first() {
        for action in [RESTART, SHUT_DOWN, LOG_OUT] {
            assert!(confirmation(action).is_some());
        }
        assert!(confirmation(SLEEP).is_none());
        assert!(confirmation(LOCK_SCREEN).is_none());
    }
}
