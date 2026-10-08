//! The menu bar's own menus: the Lulo menu at the left, and the app and
//! Window menus it builds for a Windows app that has no Lulo menus.

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

pub const HIDE_APP: &str = "lulo::HideApp";
pub const QUIT_APP: &str = "lulo::QuitApp";
pub const MINIMIZE: &str = "lulo::Minimize";
pub const ZOOM: &str = "lulo::Zoom";
pub const CLOSE_WINDOW: &str = "lulo::CloseWindow";

pub const OPEN_RECYCLE_BIN: &str = "lulo::OpenRecycleBin";
pub const EMPTY_RECYCLE_BIN: &str = "lulo::EmptyRecycleBin";

/// The Recycle Bin tile's menu, as the Mac's Trash tile has Open and
/// Empty Trash. Windows asks before it empties the bin.
pub fn recycle_bin_menu(full: bool) -> Vec<Item> {
    let mut empty = Item::new("Empty Recycle Bin", EMPTY_RECYCLE_BIN, "").separated();
    empty.enabled = full;
    vec![Item::new("Open", OPEN_RECYCLE_BIN, ""), empty]
}

/// The Lulo menu, the Apple menu's place: the same rows as the Mac's, each
/// doing the Windows equivalent, plus the Lulo layer's own switch.
pub fn lulo_menu(user: &str, starts_at_sign_in: bool) -> Menu {
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

/// The menus for the desktop itself (nothing in front): the Mac shows the
/// Finder's; here File Explorer's name with its Window menu.
pub fn desktop_menus() -> Vec<Menu> {
    windows_app_menus("File Explorer")
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
        let menu = lulo_menu("Ada", true);
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
                "Turn Off Lulo"
            ]
        );
        assert_eq!(menu.items[8].checked, CheckState::On);
        assert_eq!(lulo_menu("", false).items[7].label, "Log Out…");
        let mut all = actions(&[menu]);
        let count = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), count);
        rmac_app_menu::validate_menus(&[lulo_menu("Ada", false)]).unwrap();
    }

    #[test]
    fn windows_apps_get_their_name_and_a_window_menu() {
        let menus = windows_app_menus("Notepad");
        assert_eq!(menus[0].label, "Notepad");
        assert_eq!(menus[0].items[1].label, "Quit Notepad");
        assert_eq!(menus[1].label, "Window");
        rmac_app_menu::validate_menus(&menus).unwrap();
        assert_eq!(desktop_menus()[0].label, "File Explorer");
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
