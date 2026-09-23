//! System Settings launcher provider and searchable pane catalog.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingEntry {
    pub pane_id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub keywords: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsProvider {
    entries: Vec<SettingEntry>,
}

impl SettingsProvider {
    pub fn new(entries: Vec<SettingEntry>) -> Self {
        Self { entries }
    }

    pub fn system_settings() -> Self {
        Self::new(system_settings_entries())
    }
}

pub fn system_settings_entries() -> Vec<SettingEntry> {
    [
        (
            "wifi",
            "Wi-Fi",
            "Wireless network connections",
            &["wireless", "wlan", "internet"][..],
        ),
        (
            "bluetooth",
            "Bluetooth",
            "Nearby devices and accessories",
            &["headphones", "keyboard", "pair device"],
        ),
        (
            "network",
            "Network",
            "Ethernet, DNS, and proxy settings",
            &["ethernet", "dns", "proxy", "connection"],
        ),
        (
            "vpn",
            "VPN",
            "Private network connections",
            &["tunnel", "work network"],
        ),
        (
            "battery",
            "Battery",
            "Energy use and power behavior",
            &["power", "energy", "charging"],
        ),
        (
            "general",
            "General",
            "System information, updates, and storage",
            &["about", "update", "storage", "system information", "backup"],
        ),
        (
            "date-time",
            "Date & Time",
            "Time zone and automatic clock settings",
            &["clock", "timezone", "ntp", "automatic time"],
        ),
        (
            "language-region",
            "Language & Region",
            "Language, formats, and keyboard layouts",
            &["locale", "formats", "region", "xkb", "input source"],
        ),
        (
            "login-items",
            "Login Items",
            "Applications and services that start at sign in",
            &["startup", "autostart", "systemd user"],
        ),
        (
            "sharing",
            "Sharing",
            "Remote login and file sharing",
            &["ssh", "samba", "remote access", "shared folders"],
        ),
        (
            "accessibility",
            "Accessibility",
            "Vision, hearing, motor, and speech support",
            &[
                "screen reader",
                "zoom",
                "contrast",
                "reduce motion",
                "assistive",
            ],
        ),
        (
            "appearance",
            "Appearance",
            "Light, dark, accent, and interface style",
            &["theme", "dark mode", "light mode", "accent", "color"],
        ),
        (
            "desktop-dock",
            "Desktop & Dock",
            "Dock, windows, workspaces, and desktop behavior",
            &["dock", "windows", "workspace", "autohide", "magnification"],
        ),
        (
            "displays",
            "Displays",
            "Resolution, scale, arrangement, and brightness",
            &["monitor", "screen", "resolution", "scaling", "brightness"],
        ),
        (
            "menu-bar",
            "Menu Bar",
            "Status items, battery percentage, and clock seconds",
            &[
                "menu bar",
                "status bar",
                "battery percentage",
                "clock",
                "seconds",
            ],
        ),
        (
            "spotlight",
            "Spotlight",
            "Search providers, privacy, exclusions, and shortcut",
            &[
                "search", "launcher", "indexing", "privacy", "exclude", "shortcut",
            ],
        ),
        (
            "wallpaper",
            "Wallpaper",
            "Desktop background for each display",
            &["background", "desktop picture", "image"],
        ),
        (
            "notifications",
            "Notifications",
            "Alerts, banners, and application policy",
            &["alerts", "banners", "notification center"],
        ),
        (
            "sound",
            "Sound",
            "Output, input, effects, and volume",
            &["volume", "speaker", "microphone", "audio", "mute"],
        ),
        (
            "keyboard",
            "Keyboard",
            "Key repeat, input, and shortcuts",
            &["keys", "repeat", "input source", "shortcut"],
        ),
        (
            "mouse",
            "Mouse",
            "Pointer, scrolling, acceleration, and buttons",
            &["pointer", "scroll", "click", "acceleration"],
        ),
        (
            "trackpad",
            "Trackpad",
            "Tracking, tapping, scrolling, and gestures",
            &["touchpad", "gesture", "tap", "scroll"],
        ),
        (
            "focus",
            "Focus",
            "Silence interruptions with Focus modes",
            &["do not disturb", "quiet", "notifications"],
        ),
        (
            "lock-screen",
            "Lock Screen",
            "Lock, login, and idle timeout behavior",
            &["lock", "login", "password", "timeout", "idle"],
        ),
        (
            "privacy-security",
            "Privacy & Security",
            "Permissions, firewall, and system security",
            &[
                "permissions",
                "firewall",
                "encryption",
                "security",
                "privacy",
            ],
        ),
    ]
    .into_iter()
    .map(|(pane_id, title, subtitle, keywords)| SettingEntry {
        pane_id: pane_id.into(),
        title: title.into(),
        subtitle: Some(subtitle.into()),
        keywords: keywords.iter().map(|keyword| (*keyword).into()).collect(),
    })
    .collect()
}

impl Provider for SettingsProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(SETTINGS_PROVIDER, Category::Settings, Privacy::default())
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        let mut seen = BTreeSet::new();
        let mut results = Vec::new();
        for entry in &self.entries {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            if entry.pane_id.trim().is_empty()
                || !seen.insert(entry.pane_id.clone())
                || !(rmac_launcher::query_matches(query, &entry.title, entry.subtitle.as_deref())
                    || entry
                        .keywords
                        .iter()
                        .any(|keyword| rmac_launcher::query_matches(query, keyword, None)))
            {
                continue;
            }
            results.push(SearchResult {
                id: ResultId {
                    provider: provider_id(SETTINGS_PROVIDER),
                    local: entry.pane_id.clone(),
                },
                category: Category::Settings,
                application_group: None,
                title: entry.title.clone(),
                subtitle: entry.subtitle.clone(),
                icon: None,
                primary: Action::OpenSetting {
                    pane_id: entry.pane_id.clone(),
                },
                alternate: None,
                recency_rank: 0,
            });
            if results.len() == PROVIDER_LIMIT {
                break;
            }
        }
        Ok(results)
    }
}
