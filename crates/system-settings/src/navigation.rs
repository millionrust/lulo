//! Truthful Settings pane inventory, launcher routing, and subpage model.

use gpui::{Hsla, SharedString};

#[derive(Clone)]
pub(super) struct Category {
    pub(super) name: SharedString,
    pub(super) icon: &'static str,
    pub(super) color: Hsla,
    pub(super) desc: SharedString,
    /// Exact labels and concepts implemented by this pane. These make a
    /// setting discoverable without inventing a destination or deep link.
    pub(super) search_terms: &'static [&'static str],
}

/// A navigation subpage pushed onto the back stack from a row chevron.
#[derive(Clone)]
pub(super) enum SubPage {
    About,
    SoftwareUpdate,
    Storage,
    NotificationApp { app_id: String },
    FocusMode { mode_id: String },
    FocusSchedule { schedule_id: String },
}

pub(super) const GENERAL_DESTINATIONS: [&str; 3] = ["About", "Software Update", "Storage"];
pub(super) const PANE_ROUTES: [(&str, &str); 24] = [
    ("wifi", "Wi-Fi"),
    ("bluetooth", "Bluetooth"),
    ("network", "Network"),
    ("vpn", "VPN"),
    ("battery", "Battery"),
    ("general", "General"),
    ("date-time", "Date & Time"),
    ("language-region", "Language & Region"),
    ("login-items", "Login Items"),
    ("sharing", "Sharing"),
    ("accessibility", "Accessibility"),
    ("appearance", "Appearance"),
    ("desktop-dock", "Desktop & Dock"),
    ("displays", "Displays"),
    ("spotlight", "Spotlight"),
    ("wallpaper", "Wallpaper"),
    ("notifications", "Notifications"),
    ("sound", "Sound"),
    ("keyboard", "Keyboard"),
    ("mouse", "Mouse"),
    ("trackpad", "Trackpad"),
    ("focus", "Focus"),
    ("lock-screen", "Lock Screen"),
    ("privacy-security", "Privacy & Security"),
];

pub(super) fn categories() -> Vec<Vec<Category>> {
    let blue = color(0x0a84ff);
    let gray = color(0x8e8e93);
    let green = color(0x34c759);
    let red = color(0xff3b30);
    let pink = color(0xff2d55);
    let indigo = color(0x5e5ce6);
    let teal = color(0x30b0c7);

    let cat = |name: &str, icon: &'static str, color: Hsla, desc: &str| Category {
        name: name.to_string().into(),
        icon,
        color,
        desc: desc.to_string().into(),
        search_terms: crate::settings_search::terms_for_pane(name),
    };

    vec![
        vec![
            cat(
                "Wi-Fi",
                "icons/wifi.svg",
                blue,
                "Connect to Wi-Fi networks and manage known networks.",
            ),
            cat(
                "Bluetooth",
                "icons/bluetooth.svg",
                blue,
                "Pair and manage Bluetooth devices.",
            ),
            cat(
                "Network",
                "icons/globe.svg",
                blue,
                "Configure network services and connections.",
            ),
            cat(
                "VPN",
                "icons/key.svg",
                blue,
                "Set up and manage VPN configurations.",
            ),
            cat(
                "Battery",
                "icons/battery-charging.svg",
                green,
                "Monitor battery usage and energy settings.",
            ),
        ],
        vec![
            cat(
                "General",
                "icons/settings.svg",
                gray,
                "View system information, update status, and storage.",
            ),
            cat(
                "Date & Time",
                "icons/clock.svg",
                blue,
                "Adjust the time zone and network time synchronization.",
            ),
            cat(
                "Language & Region",
                "icons/languages.svg",
                blue,
                "Choose the system language and inspect regional formats.",
            ),
            cat(
                "Login Items",
                "icons/app-window.svg",
                blue,
                "Choose applications and services that start when you sign in.",
            ),
            cat(
                "Sharing",
                "icons/globe.svg",
                blue,
                "Control reviewed remote access and file-sharing services.",
            ),
            cat(
                "Accessibility",
                "icons/accessibility.svg",
                blue,
                "Customize the computer for the way you work.",
            ),
            cat(
                "Appearance",
                "icons/palette.svg",
                color(0x1d1d1f),
                "Change how windows, buttons, and menus look.",
            ),
            cat(
                "Desktop & Dock",
                "icons/app-window.svg",
                gray,
                "Choose authoritative rmac Dock behavior and display placement.",
            ),
            cat(
                "Displays",
                "icons/monitor.svg",
                blue,
                "Arrange displays and adjust resolution.",
            ),
            cat(
                "Spotlight",
                "icons/search.svg",
                gray,
                "Choose search results, file privacy, and excluded folders.",
            ),
            cat(
                "Wallpaper",
                "icons/image.svg",
                teal,
                "Choose original or local images for every niri display.",
            ),
        ],
        vec![
            cat(
                "Notifications",
                "icons/bell.svg",
                red,
                "Choose how you receive notifications.",
            ),
            cat(
                "Sound",
                "icons/volume-2.svg",
                pink,
                "Adjust sound effects and output.",
            ),
            cat(
                "Keyboard",
                "icons/keyboard.svg",
                gray,
                "Adjust key repeat behavior and keyboard startup options.",
            ),
            cat(
                "Mouse",
                "icons/mouse.svg",
                gray,
                "Adjust tracking, scrolling, acceleration, and buttons.",
            ),
            cat(
                "Trackpad",
                "icons/touchpad.svg",
                gray,
                "Adjust tracking, tapping, scrolling, and gestures.",
            ),
            cat(
                "Focus",
                "icons/moon.svg",
                indigo,
                "Stay focused by silencing notifications.",
            ),
        ],
        vec![
            cat(
                "Lock Screen",
                "icons/lock.svg",
                gray,
                "Adjust your lock screen and login.",
            ),
            cat(
                "Privacy & Security",
                "icons/shield.svg",
                blue,
                "Control what the system and applications can access.",
            ),
        ],
    ]
}

pub(super) fn category_name_for_pane_id(pane_id: &str) -> Option<&'static str> {
    PANE_ROUTES
        .iter()
        .find_map(|(id, name)| (*id == pane_id).then_some(*name))
}

pub(super) fn pane_id_for_category_name(name: &str) -> Option<&'static str> {
    PANE_ROUTES
        .iter()
        .find_map(|(id, category)| (*category == name).then_some(*id))
}

pub(super) fn category_position(sections: &[Vec<Category>], name: &str) -> Option<(usize, usize)> {
    sections
        .iter()
        .enumerate()
        .find_map(|(section, categories)| {
            categories
                .iter()
                .position(|category| category.name == name)
                .map(|row| (section, row))
        })
}

pub(super) fn category_has_dedicated_renderer(name: &str) -> bool {
    matches!(
        name,
        "Wi-Fi"
            | "Bluetooth"
            | "Network"
            | "VPN"
            | "Battery"
            | "General"
            | "Date & Time"
            | "Language & Region"
            | "Login Items"
            | "Sharing"
            | "Accessibility"
            | "Appearance"
            | "Desktop & Dock"
            | "Displays"
            | "Spotlight"
            | "Wallpaper"
            | "Notifications"
            | "Sound"
            | "Keyboard"
            | "Mouse"
            | "Trackpad"
            | "Focus"
            | "Lock Screen"
            | "Privacy & Security"
    )
}

fn color(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_navigation_contains_only_truthful_destinations() {
        assert_eq!(
            GENERAL_DESTINATIONS,
            ["About", "Software Update", "Storage"]
        );
    }

    #[test]
    fn optional_unowned_panes_and_generic_clickable_rows_are_absent() {
        let categories = categories().into_iter().flatten().collect::<Vec<_>>();
        let names = categories
            .iter()
            .map(|category| category.name.to_string())
            .collect::<Vec<_>>();
        assert!(!names.iter().any(|name| name == "Assistant & Intelligence"));
        assert!(!names.iter().any(|name| name == "Screen Time"));
        assert!(!names.iter().any(|name| name == "Handoff"));
        assert!(names.iter().any(|name| name == "Language & Region"));
        assert!(names.iter().any(|name| name == "Login Items"));
        assert!(names.iter().any(|name| name == "Sharing"));
        assert!(names
            .iter()
            .all(|name| category_has_dedicated_renderer(name)));
    }

    #[test]
    fn every_launcher_destination_routes_to_a_visible_category() {
        let sections = categories();
        for entry in rmac_launcher_providers::system_settings_entries() {
            let category = category_name_for_pane_id(&entry.pane_id)
                .unwrap_or_else(|| panic!("missing category route for {}", entry.pane_id));
            assert!(category_position(&sections, category).is_some());
        }
        assert!(category_name_for_pane_id("assistant").is_none());
        assert!(category_name_for_pane_id("screen-time").is_none());
    }

    #[test]
    fn every_visible_category_has_one_stable_roundtrip_route() {
        let categories = categories().into_iter().flatten().collect::<Vec<_>>();
        assert_eq!(categories.len(), PANE_ROUTES.len());
        for category in categories {
            let name = category.name.as_ref();
            let pane_id = pane_id_for_category_name(name)
                .unwrap_or_else(|| panic!("missing pane ID for {name}"));
            assert_eq!(category_name_for_pane_id(pane_id), Some(name));
        }
    }
}
