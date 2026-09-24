//! Pure menu geometry and status-menu models, measured on macOS 26
//! (design-lab/menus.html has the numbers and where each came from). Kept
//! free of GPUI so it is unit tested on every host.

use std::time::Duration;

use rmac_app_menu::Item;
use rmac_network::{NetworkDevice, WifiNetwork, WifiNetworkId, WifiSecurity, WifiSnapshot};

// ---- App menus (rmac menu, app menu, exported menus) ----

/// A dropdown hangs one point below the bar.
pub const MENU_TOP_GAP: f32 = 1.0;
/// Title menus open 4 pt left of the title's item frame, which sits 1 pt
/// inside rmac's highlight slot.
pub const TITLE_MENU_OFFSET: f32 = -3.0;
/// The rmac (Apple) menu opens 4 pt left of the logo slot.
pub const LOGO_MENU_OFFSET: f32 = -4.0;
pub const APP_MENU_RADIUS: f32 = 12.0;
pub const APP_MENU_PADDING: f32 = 5.0;
pub const APP_ROW_HEIGHT: f32 = 24.0;
/// 5 pt · 1 pt line · 5 pt.
pub const APP_SEPARATOR_HEIGHT: f32 = 11.0;
pub const APP_SEPARATOR_INSET: f32 = 16.0;
/// Highlight inset from the panel edge.
pub const ROW_INSET: f32 = 5.0;
/// Text column without icons.
pub const APP_TEXT_INSET: f32 = 16.5;
/// Icon glyphs centre here; text follows at [`APP_ICON_TEXT`].
pub const APP_ICON_CENTRE: f32 = 24.5;
pub const APP_ICON_TEXT: f32 = 39.0;
/// The Apple menu's laptop glyph is 14.5 wide, which pushes its text column.
pub const APP_WIDE_ICON_CENTRE: f32 = 25.0;
pub const APP_WIDE_ICON_TEXT: f32 = 41.5;
/// Icons are drawn in a 16 pt box.
pub const MENU_ICON_BOX: f32 = 16.0;
/// Modifier glyphs sit centred in 13.75 pt cells; the key itself is left
/// aligned 26 pt from the right edge (a 12 pt cell ending 14 pt in).
pub const KEY_CELL: f32 = 13.75;
pub const KEY_LETTER_GAP: f32 = 3.1;
pub const KEY_LETTER_WIDTH: f32 = 12.0;
pub const KEY_RIGHT: f32 = 14.0;
/// Submenu chevron: 5 × 9 glyph ending 17 pt from the right edge.
pub const CHEVRON_RIGHT: f32 = 17.0;
pub const CHEVRON_WIDTH: f32 = 5.0;
/// Minimum space between a title and its shortcut or chevron.
pub const SHORTCUT_GAP: f32 = 24.0;
/// Exported and synthesized menus mark a submenu with this shortcut.
pub const SUBMENU_MARK: &str = "›";
const MODIFIERS: [char; 5] = ['⌃', '⌥', '⇧', '⌘', '🌐'];

pub fn app_menu_height(items: &[Item]) -> f32 {
    let separators = items.iter().filter(|item| item.separator_before).count() as f32;
    2.0 * APP_MENU_PADDING + APP_ROW_HEIGHT * items.len() as f32 + APP_SEPARATOR_HEIGHT * separators
}

/// Top of item `index` measured from the panel's top edge.
pub fn app_menu_item_top(items: &[Item], index: usize) -> f32 {
    let separators = items
        .iter()
        .take(index + 1)
        .filter(|item| item.separator_before)
        .count() as f32;
    APP_MENU_PADDING + APP_ROW_HEIGHT * index as f32 + APP_SEPARATOR_HEIGHT * separators
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub modifiers: Vec<char>,
    pub key: String,
}

/// Splits "⇧⌘N" into its modifier glyphs and the key they apply to.
pub fn split_shortcut(shortcut: &str) -> Shortcut {
    let modifiers = shortcut
        .chars()
        .take_while(|glyph| MODIFIERS.contains(glyph))
        .collect::<Vec<_>>();
    let key = shortcut.chars().skip(modifiers.len()).collect::<String>();
    Shortcut { modifiers, key }
}

/// Width of the shortcut column for `shortcut`, 0 when there is none.
pub fn shortcut_width(shortcut: &str) -> f32 {
    if shortcut.is_empty() || shortcut == SUBMENU_MARK {
        return 0.0;
    }
    let parts = split_shortcut(shortcut);
    parts.modifiers.len() as f32 * KEY_CELL
        + if parts.key.is_empty() {
            0.0
        } else {
            KEY_LETTER_GAP + KEY_LETTER_WIDTH
        }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconColumn {
    None,
    Standard,
    Wide,
}

impl IconColumn {
    pub fn for_icons<'a>(icons: impl IntoIterator<Item = Option<&'a str>>) -> Self {
        let mut column = Self::None;
        for icon in icons.into_iter().flatten() {
            if icon == "laptop" {
                return Self::Wide;
            }
            column = Self::Standard;
        }
        column
    }

    pub fn text_x(self) -> f32 {
        match self {
            Self::None => APP_TEXT_INSET,
            Self::Standard => APP_ICON_TEXT,
            Self::Wide => APP_WIDE_ICON_TEXT,
        }
    }

    /// Left edge of the 16 pt icon box.
    pub fn icon_x(self) -> f32 {
        match self {
            Self::None | Self::Standard => APP_ICON_CENTRE - MENU_ICON_BOX / 2.0,
            Self::Wide => APP_WIDE_ICON_CENTRE - MENU_ICON_BOX / 2.0,
        }
    }
}

/// A content-sized menu: the widest title plus its shortcut or chevron.
pub fn app_menu_width(
    items: &[Item],
    column: IconColumn,
    min_width: f32,
    label_width: impl Fn(&str) -> f32,
) -> f32 {
    let content = items
        .iter()
        .map(|item| {
            let trailing = if item.shortcut == SUBMENU_MARK {
                SHORTCUT_GAP + CHEVRON_WIDTH + CHEVRON_RIGHT
            } else if item.shortcut.is_empty() {
                APP_TEXT_INSET
            } else {
                SHORTCUT_GAP + shortcut_width(&item.shortcut) + KEY_RIGHT
            };
            column.text_x() + label_width(&item.label) + trailing
        })
        .fold(0.0, f32::max);
    content.ceil().max(min_width)
}

/// macOS 26 decorates standard menu items with a symbol. The rmac menu and
/// the synthesized app menu are matched by action; exported menus by title.
pub fn menu_item_icon(action: &str, label: &str) -> Option<&'static str> {
    let by_action = match action {
        "system::about" => Some("laptop"),
        "system::settings" => Some("gear"),
        "system::software-center" => Some("store"),
        "system::recents" => Some("clock"),
        "system::force-quit" => Some("force-quit"),
        "system::sleep" => Some("sleep"),
        "system::restart" => Some("restart"),
        "system::shutdown" => Some("power"),
        "system::lock" => Some("lock"),
        "system::logout" => Some("person"),
        "app::about" => Some("info"),
        "app::services" => Some("services"),
        "app::hide" => Some("hide"),
        "app::hide-others" => Some("hide-others"),
        "app::show-all" => Some("show-all"),
        _ => None,
    };
    if by_action.is_some() || action.starts_with("system::") || action.starts_with("app::") {
        return by_action;
    }
    let title = label
        .split(" “")
        .next()
        .unwrap_or(label)
        .trim_end_matches('…')
        .trim_end_matches("...");
    Some(match title {
        "New Window" | "New Finder Window" => "new-window",
        "New Folder" => "new-folder",
        "New Tab" => "new-tab",
        "Open" => "open",
        "Close" | "Close Window" | "Close Tab" => "close",
        "Get Info" => "info",
        "Rename" => "rename",
        "Duplicate" => "duplicate",
        "Quick Look" => "eye",
        "Print" => "print",
        "Share" => "share",
        "Add to Sidebar" => "star",
        "Move to Trash" | "Move to Bin" => "trash",
        "Eject" => "eject",
        "Find" => "search",
        "Undo" => "undo",
        "Redo" => "redo",
        "Cut" => "cut",
        "Copy" => "copy",
        "Paste" => "paste",
        "Select All" => "select-all",
        "Show Clipboard" => "clipboard",
        "Minimize" | "Minimise" => "minimize",
        "Zoom" => "zoom",
        "Show Sidebar" | "Hide Sidebar" => "sidebar",
        "Enter Full Screen" | "Exit Full Screen" => "full-screen",
        "Settings" | "Preferences" => "settings",
        _ => return None,
    })
}

// ---- Status menus (Wi-Fi, Battery) ----

pub const STATUS_MENU_WIDTH: f32 = 308.0;
pub const STATUS_MENU_RADIUS: f32 = 15.0;
pub const STATUS_PADDING_TOP: f32 = 5.0;
pub const STATUS_PADDING_BOTTOM: f32 = 5.5;
pub const STATUS_TEXT_INSET: f32 = 14.5;
pub const STATUS_SEPARATOR_INSET: f32 = 14.0;
pub const STATUS_SWITCH_RIGHT: f32 = 14.0;
pub const STATUS_BADGE: f32 = 26.0;
pub const STATUS_BADGE_TEXT: f32 = 48.5;
pub const STATUS_DETAIL_SIZE: f32 = 11.0;
pub const SWITCH_WIDTH: f32 = 54.0;
pub const SWITCH_HEIGHT: f32 = 24.0;
pub const SWITCH_KNOB_WIDTH: f32 = 32.0;
pub const SWITCH_KNOB_HEIGHT: f32 = 20.0;
pub const MAX_LISTED_NETWORKS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusMenuKind {
    Wifi,
    Battery,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusAction {
    ToggleWifi,
    Join(WifiNetworkId),
    ToggleOtherNetworks,
    OpenSettings(&'static str),
    ToggleLowPower,
    /// Dismiss a failed Wi-Fi mutation's banner without changing anything.
    DismissWifiError,
    /// Dismiss a failed energy-mode mutation's banner without changing
    /// anything.
    DismissBatteryError,
}

impl StatusAction {
    /// Whether choosing it dismisses the menu (switches, the disclosure and
    /// error dismissals act in place, as on macOS).
    pub fn closes_menu(&self) -> bool {
        matches!(self, Self::Join(_) | Self::OpenSettings(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeGlyph {
    Wifi(u8),
    LowPower,
}

impl BadgeGlyph {
    pub fn icon(self) -> &'static str {
        match self {
            Self::Wifi(1) => "wifi-1",
            Self::Wifi(2) => "wifi-2",
            Self::Wifi(_) => "wifi-3",
            Self::LowPower => "battery-low",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusRow {
    /// Bold title with an optional trailing value or switch.
    Title {
        label: String,
        value: Option<String>,
        switch: Option<bool>,
        action: Option<StatusAction>,
    },
    /// Secondary text line such as "Power Source: Battery".
    Info(String),
    Item {
        label: String,
        warning: bool,
        action: StatusAction,
    },
    Separator,
    Header(String),
    Disclosure {
        label: String,
        expanded: bool,
    },
    /// A row led by a 26 pt circular badge (networks, Low Power).
    Badge {
        label: String,
        glyph: BadgeGlyph,
        on: bool,
        locked: bool,
        action: Option<StatusAction>,
    },
    /// Small secondary line under a network (Option-click details).
    Detail(String),
    /// The extra point that closes a badge group before its separator.
    GroupEnd,
}

impl StatusRow {
    pub fn height(&self) -> f32 {
        match self {
            Self::Title { .. } => 31.0,
            Self::Info(_) => 20.0,
            Self::Item { .. } | Self::Disclosure { .. } => 24.0,
            Self::Separator => 9.0,
            Self::Header(_) => 23.0,
            Self::Badge { .. } => 32.0,
            Self::Detail(_) => 16.0,
            Self::GroupEnd => 1.0,
        }
    }

    pub fn action(&self) -> Option<StatusAction> {
        match self {
            Self::Title { action, .. } | Self::Badge { action, .. } => action.clone(),
            Self::Item { action, .. } => Some(action.clone()),
            Self::Disclosure { .. } => Some(StatusAction::ToggleOtherNetworks),
            _ => None,
        }
    }

    /// Rows the arrow keys stop on: every actionable row except the title,
    /// whose switch is reached with the pointer.
    pub fn selectable(&self) -> bool {
        !matches!(self, Self::Title { .. }) && self.action().is_some()
    }
}

pub fn status_menu_height(rows: &[StatusRow]) -> f32 {
    STATUS_PADDING_TOP + rows.iter().map(StatusRow::height).sum::<f32>() + STATUS_PADDING_BOTTOM
}

/// Top of row `index` measured from the panel's top edge.
#[cfg_attr(not(test), allow(dead_code))]
pub fn status_row_top(rows: &[StatusRow], index: usize) -> f32 {
    STATUS_PADDING_TOP + rows.iter().take(index).map(StatusRow::height).sum::<f32>()
}

/// Status menus start at the item's highlight and flip to end at its right
/// edge when they would run off the screen (the Wi-Fi menu does).
pub fn status_menu_left(slot_left: f32, slot_right: f32, width: f32, screen_width: f32) -> f32 {
    if slot_left + width <= screen_width {
        slot_left
    } else {
        (slot_right - width).max(0.0)
    }
}

/// The next selectable row from `current` (none selected when `None`).
pub fn next_status_selection(
    rows: &[StatusRow],
    current: Option<usize>,
    forward: bool,
) -> Option<usize> {
    let selectable = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.selectable())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let position = current.and_then(|current| selectable.iter().position(|&i| i == current));
    let next = match (position, forward) {
        (None, true) => 0,
        (None, false) => selectable.len().checked_sub(1)?,
        (Some(position), true) => (position + 1) % selectable.len(),
        (Some(position), false) => position.checked_sub(1).unwrap_or(selectable.len() - 1),
    };
    selectable.get(next).copied()
}

/// Evidence capture: which status menu to open on the first frame.
pub fn parse_capture_status(value: &str) -> Option<(StatusMenuKind, bool)> {
    match value {
        "wifi" => Some((StatusMenuKind::Wifi, false)),
        "wifi-option" => Some((StatusMenuKind::Wifi, true)),
        "battery" => Some((StatusMenuKind::Battery, false)),
        _ => None,
    }
}

pub fn wifi_bars(strength: u8) -> u8 {
    match strength.min(100) {
        0..=32 => 1,
        33..=65 => 2,
        _ => 3,
    }
}

pub fn security_label(security: WifiSecurity) -> &'static str {
    match security {
        WifiSecurity::Open => "None",
        WifiSecurity::EnhancedOpen => "Enhanced Open",
        WifiSecurity::Personal(rmac_network::WifiPersonalMode::Psk) => "WPA/WPA2 Personal",
        WifiSecurity::Personal(rmac_network::WifiPersonalMode::Sae) => "WPA3 Personal",
        WifiSecurity::Personal(rmac_network::WifiPersonalMode::Transition) => "WPA2/WPA3 Personal",
        WifiSecurity::Enterprise => "Enterprise",
        WifiSecurity::Legacy => "WEP",
        WifiSecurity::Protected => "Protected",
    }
}

pub struct WifiMenuInput<'a> {
    pub wifi: Option<&'a WifiSnapshot>,
    /// The Wi-Fi device, for Option-click details.
    pub device: Option<&'a NetworkDevice>,
    pub option: bool,
    pub others_expanded: bool,
    /// The network a join is currently in flight for, if any.
    pub joining: Option<&'a WifiNetworkId>,
    /// The last join or radio-toggle failure, shown until dismissed.
    pub error: Option<&'a str>,
}

/// One entry per network name: the connected access point, else the
/// strongest, sorted by name as the Mac lists them.
fn unique_networks<'a>(networks: impl Iterator<Item = &'a WifiNetwork>) -> Vec<&'a WifiNetwork> {
    let mut unique: Vec<&WifiNetwork> = Vec::new();
    for network in networks.filter(|network| !network.ssid.is_empty()) {
        match unique.iter_mut().find(|seen| seen.ssid == network.ssid) {
            Some(seen) => {
                if (network.connected, network.strength) > (seen.connected, seen.strength) {
                    *seen = network;
                }
            }
            None => unique.push(network),
        }
    }
    unique.sort_by(|left, right| {
        left.ssid
            .to_lowercase()
            .cmp(&right.ssid.to_lowercase())
            .then_with(|| left.ssid.cmp(&right.ssid))
    });
    unique.truncate(MAX_LISTED_NETWORKS);
    unique
}

fn network_row(network: &WifiNetwork, joining: bool) -> StatusRow {
    // Saved and open networks join directly; a new protected network needs
    // credentials, which Wi-Fi Settings asks for. A join already in flight
    // is not reactivatable until it resolves.
    let action = if joining || network.connected {
        None
    } else if network.known
        || matches!(
            network.security,
            WifiSecurity::Open | WifiSecurity::EnhancedOpen
        )
    {
        Some(StatusAction::Join(network.id.clone()))
    } else {
        Some(StatusAction::OpenSettings("wifi"))
    };
    StatusRow::Badge {
        label: network.ssid.clone(),
        glyph: BadgeGlyph::Wifi(wifi_bars(network.strength)),
        on: network.connected,
        locked: network.security.is_secure(),
        action,
    }
}

fn first_ipv4(addresses: &[String]) -> Option<&str> {
    addresses
        .iter()
        .map(|address| address.split('/').next().unwrap_or(address))
        .find(|address| address.contains('.'))
}

fn connected_details(network: &WifiNetwork, device: Option<&NetworkDevice>) -> Vec<StatusRow> {
    let mut rows = Vec::new();
    if let Some(address) = device.and_then(|device| first_ipv4(&device.addresses)) {
        rows.push(StatusRow::Detail(format!("IP Address: {address}")));
    }
    if let Some(router) = device.and_then(|device| device.gateway.as_deref()) {
        rows.push(StatusRow::Detail(format!("Router: {router}")));
    }
    rows.push(StatusRow::Detail(format!(
        "Security: {}",
        security_label(network.security)
    )));
    rows
}

pub fn wifi_menu_rows(input: WifiMenuInput<'_>) -> Vec<StatusRow> {
    let settings = StatusRow::Item {
        label: "Wi-Fi Settings…".into(),
        warning: false,
        action: StatusAction::OpenSettings("wifi"),
    };
    let error_row = input.error.map(|error| StatusRow::Item {
        label: error.to_string(),
        warning: true,
        action: StatusAction::DismissWifiError,
    });
    let Some(wifi) = input.wifi else {
        let mut rows = vec![StatusRow::Title {
            label: "Wi-Fi".into(),
            value: None,
            switch: None,
            action: None,
        }];
        rows.extend(error_row);
        rows.push(StatusRow::Separator);
        rows.push(settings);
        return rows;
    };
    let mut rows = vec![StatusRow::Title {
        label: "Wi-Fi".into(),
        value: None,
        switch: wifi.available.then_some(wifi.enabled),
        action: wifi.available.then_some(StatusAction::ToggleWifi),
    }];
    rows.extend(error_row);
    if !wifi.available {
        rows.push(StatusRow::Info("Wi-Fi Unavailable".into()));
    }
    if input.option {
        if let Some(interface) = &wifi.interface {
            rows.push(StatusRow::Info(format!("Interface Name: {interface}")));
        }
        if let Some(address) = input
            .device
            .and_then(|device| device.hardware_address.as_deref())
        {
            rows.push(StatusRow::Info(format!("Address: {address}")));
        }
    }
    if wifi.available && wifi.enabled {
        let connected = wifi.networks.iter().find(|network| network.connected);
        if connected.is_some_and(|network| {
            matches!(network.security, WifiSecurity::Open | WifiSecurity::Legacy)
        }) {
            rows.push(StatusRow::Item {
                label: "Weak Security…".into(),
                warning: true,
                action: StatusAction::OpenSettings("wifi"),
            });
        }
        let known = unique_networks(wifi.networks.iter().filter(|network| network.known));
        if !known.is_empty() {
            rows.push(StatusRow::Separator);
            rows.push(StatusRow::Header("Known Networks".into()));
            for network in known.iter().copied() {
                let joining = input.joining == Some(&network.id);
                rows.push(network_row(network, joining));
                if joining {
                    rows.push(StatusRow::Detail("Connecting…".into()));
                } else if input.option && network.connected {
                    rows.extend(connected_details(network, input.device));
                }
            }
            rows.push(StatusRow::GroupEnd);
        }
        let known_names = known
            .iter()
            .map(|network| network.ssid.as_str())
            .collect::<Vec<_>>();
        let others = unique_networks(
            wifi.networks
                .iter()
                .filter(|network| !network.known && !known_names.contains(&network.ssid.as_str())),
        );
        rows.push(StatusRow::Separator);
        rows.push(StatusRow::Disclosure {
            label: "Other Networks".into(),
            expanded: input.others_expanded,
        });
        if input.others_expanded {
            if others.is_empty() {
                rows.push(StatusRow::Info("No Other Networks".into()));
            } else {
                for network in others {
                    let joining = input.joining == Some(&network.id);
                    rows.push(network_row(network, joining));
                    if joining {
                        rows.push(StatusRow::Detail("Connecting…".into()));
                    } else if input.option && network.connected {
                        rows.extend(connected_details(network, input.device));
                    }
                }
                rows.push(StatusRow::GroupEnd);
            }
        }
    }
    rows.push(StatusRow::Separator);
    rows.push(settings);
    rows
}

pub fn battery_menu_rows(
    snapshot: Option<&rmac_power::Snapshot>,
    error: Option<&str>,
) -> Vec<StatusRow> {
    let battery = snapshot.and_then(|snapshot| snapshot.battery.as_ref());
    let mut rows = vec![StatusRow::Title {
        label: "Battery".into(),
        value: battery.map(|battery| format!("{}%", battery.percentage.min(100))),
        switch: None,
        action: None,
    }];
    if let Some(error) = error {
        rows.push(StatusRow::Item {
            label: error.to_string(),
            warning: true,
            action: StatusAction::DismissBatteryError,
        });
    }
    if let Some(battery) = battery {
        rows.push(StatusRow::Info(
            if battery.on_battery {
                "Power Source: Battery"
            } else {
                "Power Source: Power Adapter"
            }
            .into(),
        ));
        // `battery.seconds_remaining` (UPower TimeToEmpty/TimeToFull) is read
        // by the backend and already shown in System Settings' Battery pane
        // (`system-settings/src/power.rs::format_duration`). It is
        // deliberately not repeated here: the reference capture of the real
        // Tahoe menu-bar Battery dropdown at 38%/discharging
        // (target/evidence/mac-2026-09-23/ax/status-Battery.png) shows only
        // the percentage and power source, no time estimate — inventing a
        // row macOS doesn't draw here would violate "measure, never invent
        // numbers." Re-check a charging capture before adding one.
    }
    if let Some(profiles) = snapshot
        .map(|snapshot| &snapshot.profiles)
        .filter(|profiles| {
            profiles.available
                && profiles
                    .supported
                    .contains(&rmac_power::PowerProfile::PowerSaver)
        })
    {
        rows.push(StatusRow::Separator);
        rows.push(StatusRow::Header("Energy Mode".into()));
        rows.push(StatusRow::Badge {
            label: "Low Power".into(),
            glyph: BadgeGlyph::LowPower,
            on: profiles.active == Some(rmac_power::PowerProfile::PowerSaver),
            locked: false,
            action: Some(StatusAction::ToggleLowPower),
        });
        rows.push(StatusRow::GroupEnd);
    }
    rows.push(StatusRow::Separator);
    rows.push(StatusRow::Item {
        label: "Battery Settings…".into(),
        warning: false,
        action: StatusAction::OpenSettings("battery"),
    });
    rows
}

// ---- Log Out, Restart and Shut Down ----

/// How long a confirmed Log Out, Restart or Shut Down waits for every
/// application to close its windows. As on macOS, the request first asks
/// each app to quit, so an edited document gets its Save / Don't Save /
/// Cancel alert; an app still open when this runs out cancels the whole
/// request instead of losing work. Not measured on the Mac.
pub const QUIT_ALL_GRACE: Duration = Duration::from_secs(30);
/// How often the window list is re-read while a confirmed request waits.
/// This runs only during that request, never while idle.
pub const QUIT_ALL_CHECK: Duration = Duration::from_millis(250);

/// Where a confirmed Log Out, Restart or Shut Down stands after it asked
/// every window to close.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuitAllProgress {
    /// Every window closed: end the session or power off now.
    Proceed,
    /// Windows remain and the grace period has not run out.
    Wait,
    /// These applications still had windows open when the grace period ran
    /// out (usually an unsaved-changes alert). The request is cancelled.
    Interrupted(Vec<String>),
}

/// `remaining` names the application of each window still open, already
/// resolved to a display name (`None` when the window has no app ID).
pub fn quit_all_progress(remaining: &[Option<String>], elapsed: Duration) -> QuitAllProgress {
    if remaining.is_empty() {
        return QuitAllProgress::Proceed;
    }
    if elapsed < QUIT_ALL_GRACE {
        return QuitAllProgress::Wait;
    }
    let mut names = Vec::<String>::new();
    for name in remaining {
        let name = name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or("An application");
        if !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    QuitAllProgress::Interrupted(names)
}

/// Title and body of the notice shown when an application stops a Log Out,
/// Restart or Shut Down. Wording is rmac's; the Mac's alert was not captured.
pub fn quit_all_interrupted_copy(action: &str, apps: &[String]) -> (String, String) {
    let (title, retry) = match action {
        "system::restart" => ("Restart Cancelled", "restart"),
        "system::shutdown" => ("Shut Down Cancelled", "shut down"),
        _ => ("Log Out Cancelled", "log out"),
    };
    let subject = match apps {
        [] => "An application".to_owned(),
        [only] => only.clone(),
        [first, second] => format!("{first} and {second}"),
        [first, second, rest @ ..] => format!("{first}, {second} and {} more", rest.len()),
    };
    let pronoun = if apps.len() > 1 { "their" } else { "its" };
    (
        title.to_owned(),
        format!("{subject} didn't quit. Save or close {pronoun} windows, then {retry} again."),
    )
}

// ---- Low battery ----

/// Battery levels, in percent, that post a warning while running on
/// battery: one at 10 % and a stronger one at 5 %. Not measured on the Mac.
pub const LOW_BATTERY_WARNINGS: [u8; 2] = [10, 5];

/// Remembers which low-battery warning has been shown for the current
/// discharge, so each level is announced once. Connecting power resets it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LowBatteryWatch {
    warned_at: Option<u8>,
}

impl LowBatteryWatch {
    /// Feed each battery reading. Returns the warning level to announce now,
    /// if the reading reached one that has not been announced yet.
    pub fn observe(&mut self, percentage: u8, on_battery: bool) -> Option<u8> {
        if !on_battery {
            self.warned_at = None;
            return None;
        }
        let level = LOW_BATTERY_WARNINGS
            .iter()
            .copied()
            .filter(|threshold| percentage <= *threshold)
            .min()?;
        if self.warned_at.is_some_and(|warned| warned <= level) {
            return None;
        }
        self.warned_at = Some(level);
        Some(level)
    }
}

/// Title and body of a low-battery warning. Wording is rmac's.
pub fn low_battery_copy(level: u8, percentage: u8) -> (String, String) {
    if level <= LOW_BATTERY_WARNINGS[1] {
        (
            "Battery Very Low".to_owned(),
            format!(
                "{percentage}% of battery remains. Connect to power now to avoid losing unsaved work."
            ),
        )
    } else {
        (
            "Low Battery".to_owned(),
            format!("{percentage}% of battery remains. Connect to power soon."),
        )
    }
}

// ---- System Settings… and Force Quit… ----

/// What choosing System Settings… or Force Quit… does. Like the Mac, a
/// second choice brings the open window forward instead of opening another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenOrFocus {
    /// No window yet: start the app.
    Launch,
    /// Raise this visible window, the app's most recently focused one.
    Focus(rmac_compositor::WindowId),
    /// Every window is hidden: bring them back.
    Restore(Vec<rmac_compositor::WindowId>),
}

pub fn open_or_focus(snapshot: &rmac_compositor::Snapshot, app_id: &str) -> OpenOrFocus {
    let windows = snapshot
        .windows
        .iter()
        .filter(|window| window.app_id.as_deref() == Some(app_id))
        .collect::<Vec<_>>();
    let recency = |window: &&rmac_compositor::Window| {
        window
            .focus_timestamp
            .map(|stamp| (stamp.seconds, stamp.nanoseconds))
    };
    let visible = windows
        .iter()
        .copied()
        .filter(|window| !rmac_compositor::window_is_parked(snapshot, window))
        .max_by_key(recency);
    match visible {
        Some(window) => OpenOrFocus::Focus(window.id),
        None if windows.is_empty() => OpenOrFocus::Launch,
        None => OpenOrFocus::Restore(windows.iter().map(|window| window.id).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, shortcut: &str, separator_before: bool) -> Item {
        Item {
            label: label.into(),
            action: format!("test::{label}"),
            shortcut: shortcut.into(),
            enabled: true,
            separator_before,
        }
    }

    /// The Apple menu's shape: 10 rows, 5 separators — 295 × 305 on the Mac.
    fn apple_shape() -> Vec<Item> {
        [
            false, true, false, true, true, true, false, false, true, false,
        ]
        .iter()
        .enumerate()
        .map(|(index, &separator)| item(&format!("Row {index}"), "", separator))
        .collect()
    }

    #[test]
    fn app_menu_geometry_matches_the_mac_apple_menu() {
        let items = apple_shape();
        assert_eq!(app_menu_height(&items), 305.0);
        // AX: About 39, System Settings 74, Recent Items 133, Sleep 203,
        // Lock Screen 286 — minus the panel top at 34.
        assert_eq!(app_menu_item_top(&items, 0), 5.0);
        assert_eq!(app_menu_item_top(&items, 1), 40.0);
        assert_eq!(app_menu_item_top(&items, 3), 99.0);
        assert_eq!(app_menu_item_top(&items, 5), 169.0);
        assert_eq!(app_menu_item_top(&items, 8), 252.0);
    }

    #[test]
    fn shortcuts_split_into_modifier_cells_and_a_key() {
        assert_eq!(
            split_shortcut("⇧⌘N"),
            Shortcut {
                modifiers: vec!['⇧', '⌘'],
                key: "N".into()
            }
        );
        assert_eq!(split_shortcut("⌘+").key, "+");
        assert_eq!(shortcut_width(""), 0.0);
        assert_eq!(shortcut_width(SUBMENU_MARK), 0.0);
        assert_eq!(
            shortcut_width("⌘Q"),
            KEY_CELL + KEY_LETTER_GAP + KEY_LETTER_WIDTH
        );
        assert_eq!(
            shortcut_width("⌥⌘H"),
            2.0 * KEY_CELL + KEY_LETTER_GAP + KEY_LETTER_WIDTH
        );
    }

    #[test]
    fn menu_width_is_the_widest_row_and_never_below_the_minimum() {
        let items = vec![
            item("Short", "", false),
            item("A much longer title", "⇧⌘N", false),
        ];
        let width = app_menu_width(&items, IconColumn::Standard, 100.0, |label| {
            label.len() as f32 * 7.0
        });
        let expected =
            APP_ICON_TEXT + 19.0 * 7.0 + SHORTCUT_GAP + shortcut_width("⇧⌘N") + KEY_RIGHT;
        assert_eq!(width, expected.ceil());
        assert_eq!(
            app_menu_width(&[item("x", "", false)], IconColumn::None, 180.0, |_| 7.0),
            180.0
        );
    }

    #[test]
    fn icon_column_follows_the_widest_glyph() {
        assert_eq!(IconColumn::for_icons([None, None]), IconColumn::None);
        assert_eq!(
            IconColumn::for_icons([None, Some("copy")]),
            IconColumn::Standard
        );
        assert_eq!(
            IconColumn::for_icons([Some("gear"), Some("laptop")]),
            IconColumn::Wide
        );
        assert_eq!(IconColumn::Standard.text_x(), 39.0);
        assert_eq!(IconColumn::Wide.text_x(), 41.5);
    }

    #[test]
    fn standard_items_get_their_macos_symbols() {
        assert_eq!(
            menu_item_icon("system::about", "About This Lulo OS"),
            Some("laptop")
        );
        assert_eq!(menu_item_icon("app::quit", "Quit Files"), None);
        assert_eq!(menu_item_icon("terminal::Copy", "Copy"), Some("copy"));
        assert_eq!(menu_item_icon("x", "Copy “notes”"), Some("copy"));
        assert_eq!(
            menu_item_icon("text_editor::OpenFile", "Open…"),
            Some("open")
        );
        assert_eq!(menu_item_icon("terminal::Clear", "Clear"), None);
    }

    fn network(
        name: &str,
        strength: u8,
        known: bool,
        connected: bool,
        security: WifiSecurity,
    ) -> WifiNetwork {
        WifiNetwork {
            id: WifiNetworkId::from_bytes(name.as_bytes().to_vec(), security).unwrap(),
            ssid: name.into(),
            strength,
            security,
            known,
            connected,
        }
    }

    fn psk() -> WifiSecurity {
        WifiSecurity::Personal(rmac_network::WifiPersonalMode::Psk)
    }

    fn snapshot(networks: Vec<WifiNetwork>) -> WifiSnapshot {
        WifiSnapshot {
            available: true,
            enabled: true,
            interface: Some("wlan0".into()),
            current_ssid: None,
            networks,
            saved_networks: Vec::new(),
        }
    }

    fn labels(rows: &[StatusRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                StatusRow::Title { label, .. } => format!("title:{label}"),
                StatusRow::Info(label) => format!("info:{label}"),
                StatusRow::Item { label, .. } => format!("item:{label}"),
                StatusRow::Separator => "---".into(),
                StatusRow::Header(label) => format!("head:{label}"),
                StatusRow::Disclosure { label, .. } => format!("more:{label}"),
                StatusRow::Badge { label, .. } => format!("badge:{label}"),
                StatusRow::Detail(label) => format!("detail:{label}"),
                StatusRow::GroupEnd => "end".into(),
            })
            .collect()
    }

    #[test]
    fn wifi_menu_lists_known_networks_by_name_and_folds_the_rest() {
        let wifi = snapshot(vec![
            network("Home Wi-Fi", 90, true, true, psk()),
            network("cafe", 40, true, false, psk()),
            network("Home Wi-Fi", 20, true, false, psk()),
            network("Neighbour", 70, false, false, psk()),
        ]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Wi-Fi",
                "---",
                "head:Known Networks",
                "badge:cafe",
                "badge:Home Wi-Fi",
                "end",
                "---",
                "more:Other Networks",
                "---",
                "item:Wi-Fi Settings…",
            ]
        );
        // The connected network is highlighted and has nothing to do.
        let StatusRow::Badge {
            on, action, glyph, ..
        } = &rows[4]
        else {
            panic!("expected the connected network");
        };
        assert!(*on);
        assert_eq!(*action, None);
        assert_eq!(*glyph, BadgeGlyph::Wifi(3));
        assert!(matches!(
            rows[3],
            StatusRow::Badge {
                action: Some(StatusAction::Join(_)),
                glyph: BadgeGlyph::Wifi(2),
                ..
            }
        ));
    }

    #[test]
    fn other_networks_expand_and_new_protected_ones_open_settings() {
        let wifi = snapshot(vec![
            network("Neighbour", 70, false, false, psk()),
            network("Guest", 10, false, false, WifiSecurity::Open),
        ]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: true,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows)[1..6],
            [
                "---",
                "more:Other Networks",
                "badge:Guest",
                "badge:Neighbour",
                "end"
            ]
        );
        assert!(matches!(
            &rows[3],
            StatusRow::Badge {
                action: Some(StatusAction::Join(_)),
                locked: false,
                ..
            }
        ));
        assert!(matches!(
            &rows[4],
            StatusRow::Badge {
                action: Some(StatusAction::OpenSettings("wifi")),
                locked: true,
                ..
            }
        ));
    }

    #[test]
    fn option_click_adds_interface_and_connection_details() {
        let wifi = snapshot(vec![network("Home Wi-Fi", 90, true, true, psk())]);
        let device = NetworkDevice {
            interface: "wlan0".into(),
            kind: rmac_network::DeviceKind::WiFi,
            state: rmac_network::DeviceState::Connected,
            connection: Some("Home Wi-Fi".into()),
            primary: true,
            addresses: vec!["fe80::1/64".into(), "192.168.1.20/24".into()],
            gateway: Some("192.168.1.1".into()),
            dns: Vec::new(),
            hardware_address: Some("00:11:22:33:44:55".into()),
            configuration: None,
            configuration_error: None,
        };
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: Some(&device),
            option: true,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows)[..9],
            [
                "title:Wi-Fi",
                "info:Interface Name: wlan0",
                "info:Address: 00:11:22:33:44:55",
                "---",
                "head:Known Networks",
                "badge:Home Wi-Fi",
                "detail:IP Address: 192.168.1.20",
                "detail:Router: 192.168.1.1",
                "detail:Security: WPA/WPA2 Personal",
            ]
        );
    }

    #[test]
    fn a_weak_connection_is_flagged_and_wifi_off_hides_the_lists() {
        let wifi = snapshot(vec![network("Cafe", 90, true, true, WifiSecurity::Open)]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(labels(&rows)[1], "item:Weak Security…");

        let mut off = snapshot(Vec::new());
        off.enabled = false;
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&off),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        assert_eq!(
            labels(&rows),
            ["title:Wi-Fi", "---", "item:Wi-Fi Settings…"]
        );
        assert!(matches!(
            rows[0],
            StatusRow::Title {
                switch: Some(false),
                action: Some(StatusAction::ToggleWifi),
                ..
            }
        ));
    }

    #[test]
    fn a_network_being_joined_shows_connecting_and_cannot_be_reactivated() {
        let wifi = snapshot(vec![
            network("Home Wi-Fi", 90, true, true, psk()),
            network("cafe", 40, true, false, psk()),
        ]);
        let joining = wifi.networks[1].id.clone();
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: Some(&joining),
            error: None,
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Wi-Fi",
                "---",
                "head:Known Networks",
                "badge:cafe",
                "detail:Connecting…",
                "badge:Home Wi-Fi",
                "end",
                "---",
                "more:Other Networks",
                "---",
                "item:Wi-Fi Settings…",
            ]
        );
        // The row being joined cannot be clicked again until it resolves.
        assert!(matches!(rows[3], StatusRow::Badge { action: None, .. }));
    }

    #[test]
    fn a_join_or_radio_failure_shows_a_dismissible_banner() {
        let wifi = snapshot(vec![network("cafe", 40, true, false, psk())]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: Some("Couldn't join \u{201c}cafe\u{201d}: wrong password"),
        });
        assert_eq!(
            labels(&rows)[..2],
            [
                "title:Wi-Fi",
                "item:Couldn't join \u{201c}cafe\u{201d}: wrong password"
            ]
        );
        assert!(matches!(
            rows[1],
            StatusRow::Item {
                warning: true,
                action: StatusAction::DismissWifiError,
                ..
            }
        ));

        // Wi-Fi unavailable still shows the banner ahead of the title-only
        // fallback.
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: None,
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: Some("Couldn't reach the Wi-Fi service"),
        });
        assert_eq!(
            labels(&rows),
            [
                "title:Wi-Fi",
                "item:Couldn't reach the Wi-Fi service",
                "---",
                "item:Wi-Fi Settings…",
            ]
        );
    }

    #[test]
    fn a_battery_mutation_failure_shows_a_dismissible_banner() {
        let battery = rmac_power::Battery {
            percentage: 61,
            state: rmac_power::BatteryState::Discharging,
            on_battery: true,
            seconds_remaining: None,
            capacity: None,
            charge_cycles: None,
            energy_rate_watts: None,
            model: None,
            charge_threshold: Default::default(),
            history: Default::default(),
        };
        let snapshot = rmac_power::Snapshot {
            battery: Some(battery),
            profiles: rmac_power::Profiles::default(),
        };
        let rows = battery_menu_rows(
            Some(&snapshot),
            Some("Couldn't change the energy mode: not authorized"),
        );
        assert_eq!(
            labels(&rows)[..2],
            [
                "title:Battery",
                "item:Couldn't change the energy mode: not authorized",
            ]
        );
        assert!(matches!(
            rows[1],
            StatusRow::Item {
                warning: true,
                action: StatusAction::DismissBatteryError,
                ..
            }
        ));
    }

    #[test]
    fn status_geometry_matches_the_mac_wifi_menu() {
        // f021: title, Weak Security, hotspot section, three known
        // networks, Other Networks, Wi-Fi Settings — 326 tall.
        let badge = || StatusRow::Badge {
            label: "n".into(),
            glyph: BadgeGlyph::Wifi(3),
            on: false,
            locked: true,
            action: None,
        };
        let rows = vec![
            StatusRow::Title {
                label: "Wi-Fi".into(),
                value: None,
                switch: Some(true),
                action: None,
            },
            StatusRow::Item {
                label: "Weak Security…".into(),
                warning: true,
                action: StatusAction::OpenSettings("wifi"),
            },
            StatusRow::Separator,
            StatusRow::Header("Personal Hotspot".into()),
            badge(),
            StatusRow::GroupEnd,
            StatusRow::Separator,
            StatusRow::Header("Known Networks".into()),
            badge(),
            badge(),
            badge(),
            StatusRow::GroupEnd,
            StatusRow::Separator,
            StatusRow::Disclosure {
                label: "Other Networks".into(),
                expanded: false,
            },
            StatusRow::Separator,
            StatusRow::Item {
                label: "Wi-Fi Settings…".into(),
                warning: false,
                action: StatusAction::OpenSettings("wifi"),
            },
        ];
        assert_eq!(status_menu_height(&rows), 325.5);
        // Row centres measured on the Mac: Weak Security 48, hotspot 107.75,
        // known networks 172.75, Wi-Fi Settings 307.75.
        assert_eq!(status_row_top(&rows, 1) + 12.0, 48.0);
        assert_eq!(status_row_top(&rows, 4) + 16.0, 108.0);
        assert_eq!(status_row_top(&rows, 8) + 16.0, 173.0);
        assert_eq!(status_row_top(&rows, 15) + 12.0, 308.0);
    }

    #[test]
    fn capture_hook_names_each_status_menu() {
        assert_eq!(
            parse_capture_status("wifi-option"),
            Some((StatusMenuKind::Wifi, true))
        );
        assert_eq!(
            parse_capture_status("battery"),
            Some((StatusMenuKind::Battery, false))
        );
        assert_eq!(parse_capture_status("clock"), None);
    }

    #[test]
    fn status_menus_flip_at_the_screen_edge() {
        // Battery fits and starts at its highlight; Wi-Fi would overflow a
        // 1470 pt screen so it ends at its highlight's right edge.
        assert_eq!(status_menu_left(1150.5, 1197.0, 308.0, 1470.0), 1150.5);
        assert_eq!(status_menu_left(1192.5, 1234.5, 308.0, 1470.0), 926.5);
    }

    #[test]
    fn arrow_keys_skip_titles_headers_and_separators() {
        let wifi = snapshot(vec![
            network("A", 90, true, true, psk()),
            network("B", 90, true, false, psk()),
        ]);
        let rows = wifi_menu_rows(WifiMenuInput {
            wifi: Some(&wifi),
            device: None,
            option: false,
            others_expanded: false,
            joining: None,
            error: None,
        });
        // A is connected (no action): first stop is B, then the disclosure,
        // then Wi-Fi Settings, then back to B.
        let first = next_status_selection(&rows, None, true);
        assert_eq!(first, Some(4));
        let second = next_status_selection(&rows, first, true);
        assert!(matches!(
            rows[second.unwrap()],
            StatusRow::Disclosure { .. }
        ));
        let third = next_status_selection(&rows, second, true);
        assert!(matches!(rows[third.unwrap()], StatusRow::Item { .. }));
        assert_eq!(next_status_selection(&rows, third, true), first);
        assert_eq!(next_status_selection(&rows, first, false), third);
        assert_eq!(next_status_selection(&[], None, true), None);
    }

    #[test]
    fn battery_menu_reports_source_and_low_power_only_when_backed() {
        let battery = rmac_power::Battery {
            percentage: 38,
            state: rmac_power::BatteryState::Discharging,
            on_battery: true,
            seconds_remaining: None,
            capacity: None,
            charge_cycles: None,
            energy_rate_watts: None,
            model: None,
            charge_threshold: Default::default(),
            history: Default::default(),
        };
        let mut snapshot = rmac_power::Snapshot {
            battery: Some(battery),
            profiles: rmac_power::Profiles::default(),
        };
        assert_eq!(
            labels(&battery_menu_rows(Some(&snapshot), None)),
            [
                "title:Battery",
                "info:Power Source: Battery",
                "---",
                "item:Battery Settings…"
            ]
        );
        snapshot.profiles = rmac_power::Profiles {
            available: true,
            active: Some(rmac_power::PowerProfile::PowerSaver),
            supported: vec![
                rmac_power::PowerProfile::PowerSaver,
                rmac_power::PowerProfile::Balanced,
            ],
            performance_degraded: None,
        };
        let rows = battery_menu_rows(Some(&snapshot), None);
        assert_eq!(
            labels(&rows),
            [
                "title:Battery",
                "info:Power Source: Battery",
                "---",
                "head:Energy Mode",
                "badge:Low Power",
                "end",
                "---",
                "item:Battery Settings…",
            ]
        );
        assert!(
            matches!(rows[0], StatusRow::Title { value: Some(ref value), .. } if value == "38%")
        );
        assert!(matches!(rows[4], StatusRow::Badge { on: true, .. }));
        // Measured Battery menu without the per-app energy section.
        assert_eq!(status_menu_height(&rows), 159.5);
    }

    #[test]
    fn quit_all_proceeds_once_every_window_has_closed() {
        assert_eq!(
            quit_all_progress(&[], Duration::ZERO),
            QuitAllProgress::Proceed
        );
        assert_eq!(
            quit_all_progress(&[], QUIT_ALL_GRACE * 2),
            QuitAllProgress::Proceed
        );
    }

    #[test]
    fn quit_all_waits_for_open_windows_then_is_interrupted_never_forced() {
        let remaining = vec![
            Some("Text Editor".to_owned()),
            Some("Text Editor".to_owned()),
            None,
            Some("  ".to_owned()),
            Some("Firefox".to_owned()),
        ];
        assert_eq!(
            quit_all_progress(&remaining, QUIT_ALL_GRACE - QUIT_ALL_CHECK),
            QuitAllProgress::Wait
        );
        assert_eq!(
            quit_all_progress(&remaining, QUIT_ALL_GRACE),
            QuitAllProgress::Interrupted(vec![
                "Text Editor".to_owned(),
                "An application".to_owned(),
                "Firefox".to_owned(),
            ])
        );
    }

    #[test]
    fn interrupted_notice_names_the_apps_and_the_request() {
        let one = ["Text Editor".to_owned()];
        assert_eq!(
            quit_all_interrupted_copy("system::logout", &one),
            (
                "Log Out Cancelled".to_owned(),
                "Text Editor didn't quit. Save or close its windows, then log out again."
                    .to_owned()
            )
        );
        let two = ["Text Editor".to_owned(), "Firefox".to_owned()];
        assert_eq!(
            quit_all_interrupted_copy("system::restart", &two).1,
            "Text Editor and Firefox didn't quit. Save or close their windows, then restart again."
        );
        let four = [
            "A".to_owned(),
            "B".to_owned(),
            "C".to_owned(),
            "D".to_owned(),
        ];
        let (title, body) = quit_all_interrupted_copy("system::shutdown", &four);
        assert_eq!(title, "Shut Down Cancelled");
        assert!(body.starts_with("A, B and 2 more didn't quit."));
        assert!(body.ends_with("then shut down again."));
    }

    #[test]
    fn low_battery_warns_once_per_level_and_resets_on_power() {
        let mut watch = LowBatteryWatch::default();
        assert_eq!(watch.observe(40, true), None);
        assert_eq!(watch.observe(11, true), None);
        assert_eq!(watch.observe(10, true), Some(10));
        assert_eq!(watch.observe(9, true), None);
        assert_eq!(watch.observe(5, true), Some(5));
        assert_eq!(watch.observe(3, true), None);
        // Plugged in: the next discharge warns again.
        assert_eq!(watch.observe(3, false), None);
        assert_eq!(watch.observe(4, true), Some(5));
        // Starting below both levels announces only the stronger warning.
        let mut late = LowBatteryWatch::default();
        assert_eq!(late.observe(2, true), Some(5));
        assert_eq!(late.observe(1, true), None);
        // Charging while low never warns.
        assert_eq!(LowBatteryWatch::default().observe(2, false), None);
    }

    #[test]
    fn low_battery_copy_names_the_percentage() {
        assert_eq!(
            low_battery_copy(10, 9),
            (
                "Low Battery".to_owned(),
                "9% of battery remains. Connect to power soon.".to_owned()
            )
        );
        assert_eq!(low_battery_copy(5, 4).0, "Battery Very Low");
        assert!(low_battery_copy(5, 4)
            .1
            .starts_with("4% of battery remains."));
    }

    #[test]
    fn system_apps_come_forward_instead_of_opening_twice() {
        use rmac_compositor::{Snapshot, Timestamp, Window, WindowId, Workspace, WorkspaceId};

        let workspace = |id: u64, name: &str| Workspace {
            id: WorkspaceId(id),
            index: id as u8,
            name: Some(name.into()),
            output: None,
            urgent: false,
            active: id == 1,
            focused: id == 1,
            active_window: None,
        };
        let window = |id: u64, app: &str, workspace: u64, seconds: u64| Window {
            id: WindowId(id),
            title: None,
            app_id: Some(app.into()),
            pid: None,
            workspace: Some(WorkspaceId(workspace)),
            focused: false,
            floating: true,
            urgent: false,
            focus_timestamp: Some(Timestamp {
                seconds,
                nanoseconds: 0,
            }),
            layout: Default::default(),
        };
        let settings = rmac_apps::identity::SYSTEM_SETTINGS;
        let mut snapshot = Snapshot {
            workspaces: vec![
                workspace(1, "Desktop"),
                workspace(2, rmac_compositor::PARKING_WORKSPACE),
            ],
            windows: vec![window(1, "firefox", 1, 50)],
            ..Snapshot::default()
        };
        assert_eq!(open_or_focus(&snapshot, settings), OpenOrFocus::Launch);

        snapshot.windows.push(window(2, settings, 2, 10));
        snapshot.windows.push(window(3, settings, 2, 20));
        assert_eq!(
            open_or_focus(&snapshot, settings),
            OpenOrFocus::Restore(vec![WindowId(2), WindowId(3)])
        );

        snapshot.windows.push(window(4, settings, 1, 5));
        snapshot.windows.push(window(5, settings, 1, 30));
        assert_eq!(
            open_or_focus(&snapshot, settings),
            OpenOrFocus::Focus(WindowId(5))
        );
    }
}
