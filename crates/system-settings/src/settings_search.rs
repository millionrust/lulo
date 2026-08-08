//! Truthful search vocabulary for settings implemented by each visible pane.

use crate::navigation::Category;

pub(crate) fn matches(category: &Category, query: &str) -> bool {
    rmac_system_settings::accessibility::category_matches(
        query,
        category.name.as_ref(),
        category.desc.as_ref(),
        category.search_terms,
    )
}

pub(crate) fn match_hint(category: &Category, query: &str) -> Option<&'static str> {
    rmac_system_settings::accessibility::category_match_hint(query, category.search_terms)
}

pub(crate) fn terms_for_pane(name: &str) -> &'static [&'static str] {
    match name {
        "Wi-Fi" => &[
            "wifi",
            "Wi-Fi power",
            "available networks",
            "known networks",
            "join a network",
            "enterprise Wi-Fi",
            "forget a network",
        ],
        "Bluetooth" => &[
            "Bluetooth power",
            "nearby devices",
            "known devices",
            "pair a device",
            "forget a device",
            "discoverable",
        ],
        "Network" => &[
            "network interfaces",
            "IP addresses",
            "IPv4",
            "IPv6",
            "DNS servers",
            "router",
            "proxy configuration",
        ],
        "VPN" => &[
            "VPN configurations",
            "connect VPN",
            "import configuration",
            "saved authentication",
            "connection timeout",
            "keep connection",
        ],
        "Battery" => &[
            "battery level",
            "battery health",
            "energy mode",
            "power profile",
            "optimized charging",
            "charge threshold",
            "battery history",
        ],
        "General" => &[
            "About this computer",
            "device name",
            "hostname",
            "software update",
            "operating system",
            "kernel",
            "graphics",
            "storage volumes",
        ],
        "Date & Time" => &[
            "current time",
            "set time automatically",
            "network time",
            "time zone",
            "set date and time",
            "hardware clock",
        ],
        "Language & Region" => &[
            "system language",
            "region",
            "locale",
            "date formats",
            "number formats",
            "currency",
            "measurement",
            "input sources",
            "keyboard layouts",
        ],
        "Login Items" => &[
            "open at login",
            "login applications",
            "startup applications",
            "allow in background",
            "background services",
            "XDG autostart",
            "systemd user services",
        ],
        "Sharing" => &[
            "remote login",
            "SSH",
            "file sharing",
            "SMB",
            "Samba",
            "shared folders",
            "firewall",
        ],
        "Accessibility" => &[
            "text size",
            "screen reader",
            "Orca",
            "display contrast",
            "reduced motion",
            "key repeat",
            "mouse precision",
            "middle-button emulation",
        ],
        "Appearance" => &[
            "light appearance",
            "dark appearance",
            "automatic appearance",
            "accent color",
            "increased contrast",
            "reduced motion",
        ],
        "Desktop & Dock" => &[
            "Dock position",
            "Dock displays",
            "Dock size",
            "Dock magnification",
            "automatically hide Dock",
            "reserve screen space",
            "application clicks",
        ],
        "Displays" => &[
            "display arrangement",
            "main display",
            "resolution",
            "scale",
            "rotation",
            "display position",
            "graphics",
        ],
        "Spotlight" => &[
            "search applications",
            "search System Settings",
            "search files",
            "calculator results",
            "recent documents",
            "private file results",
            "removable mounts",
            "excluded folders",
            "global shortcuts",
            "clear history",
        ],
        "Wallpaper" => &[
            "desktop wallpaper",
            "wallpaper image",
            "wallpaper per display",
            "fill image",
            "fit image",
            "stretch image",
            "center image",
            "tile image",
        ],
        "Notifications" => &[
            "allow notifications",
            "notification banners",
            "notification sounds",
            "badge indicator",
            "notification history",
            "urgent notifications",
            "application notifications",
        ],
        "Sound" => &[
            "output device",
            "output volume",
            "mute output",
            "input device",
            "input volume",
            "mute microphone",
            "audio ports",
            "device profiles",
        ],
        "Keyboard" => &[
            "connected keyboards",
            "key repeat rate",
            "delay until repeat",
            "Num Lock on startup",
            "keyboard layout",
        ],
        "Mouse" => &[
            "connected mice",
            "tracking speed",
            "natural scrolling",
            "pointer acceleration",
            "primary button",
            "middle-click emulation",
        ],
        "Trackpad" => &[
            "connected trackpads",
            "tracking speed",
            "tap to click",
            "natural scrolling",
            "pointer acceleration",
            "primary click",
            "drag lock",
            "ignore while typing",
        ],
        "Focus" => &[
            "Do Not Disturb",
            "Focus mode",
            "Focus schedules",
            "allowed applications",
            "urgent notifications",
            "turn on for one hour",
        ],
        "Lock Screen" => &[
            "lock now",
            "lock after inactivity",
            "screen lock timeout",
            "password required",
            "automatic suspend",
            "suspend timeout",
            "notification previews",
            "test lock screen",
        ],
        "Privacy & Security" => &[
            "portal permissions",
            "camera permission",
            "microphone permission",
            "reset permission",
            "security updates",
            "automatic security updates",
            "Ubuntu security coverage",
            "application sources",
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_panes_cannot_invent_search_results() {
        assert!(terms_for_pane("Assistant & Intelligence").is_empty());
    }

    #[test]
    fn implemented_setting_labels_route_to_their_real_panes() {
        let categories = crate::navigation::categories()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        for (query, expected) in [
            ("DNS servers", "Network"),
            ("set time automatically", "Date & Time"),
            ("open at login", "Login Items"),
            ("Dock magnification", "Desktop & Dock"),
            ("mute microphone", "Sound"),
            ("tap to click", "Trackpad"),
            ("automatic suspend", "Lock Screen"),
            ("camera permission", "Privacy & Security"),
        ] {
            let panes = categories
                .iter()
                .filter(|category| matches(category, query))
                .map(|category| category.name.as_ref())
                .collect::<Vec<_>>();
            assert_eq!(panes, [expected], "unexpected route for {query}");
        }
        assert!(categories
            .iter()
            .all(|category| !category.search_terms.is_empty()));

        let lock_panes = categories
            .iter()
            .filter(|category| matches(category, "lock"))
            .map(|category| category.name.as_ref())
            .collect::<Vec<_>>();
        assert!(lock_panes.contains(&"Lock Screen"));
        assert!(!lock_panes.contains(&"Date & Time"));
    }
}
