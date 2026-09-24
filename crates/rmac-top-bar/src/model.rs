use std::fmt;
use std::time::Duration;

pub const BAR_HEIGHT: f64 = 32.0;
pub const SYSTEM_MARK_ACCESSIBLE_NAME: &str = "desktop";
pub(crate) const MAX_ACTIVE_APP_CHARACTERS: usize = 48;
pub(crate) const MAX_WORKSPACE_CHARACTERS: usize = 32;
pub(crate) const MAX_MODE_CHARACTERS: usize = 48;
pub(crate) const MAX_VPN_NAMES: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocaleHourCycle {
    TwelveHour,
    TwentyFourHour,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockLabel {
    pub visible: String,
    pub accessible: String,
    pub activation: PanelTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelTarget {
    QuickSettings,
    NotificationCenter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorKind {
    Focus,
    Vpn,
    Network,
    Bluetooth,
    Sound,
    Battery,
    Notifications,
}

impl IndicatorKind {
    pub const fn panel_target(self) -> PanelTarget {
        match self {
            Self::Notifications => PanelTarget::NotificationCenter,
            Self::Focus
            | Self::Vpn
            | Self::Network
            | Self::Bluetooth
            | Self::Sound
            | Self::Battery => PanelTarget::QuickSettings,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinIcon {
    System,
    Focus,
    Vpn,
    Network,
    Bluetooth,
    Sound,
    Battery,
    Notifications,
}

impl BuiltinIcon {
    pub const ALL: [Self; 8] = [
        Self::System,
        Self::Focus,
        Self::Vpn,
        Self::Network,
        Self::Bluetooth,
        Self::Sound,
        Self::Battery,
        Self::Notifications,
    ];

    /// A self-contained original monochrome vector asset. The renderer tints
    /// `currentColor` from the effective top-bar theme.
    pub fn svg(self) -> &'static str {
        match self {
            Self::System => include_str!("../assets/icons/system.svg"),
            Self::Focus => include_str!("../assets/icons/focus.svg"),
            Self::Vpn => include_str!("../assets/icons/vpn.svg"),
            Self::Network => include_str!("../assets/icons/network.svg"),
            Self::Bluetooth => include_str!("../assets/icons/bluetooth.svg"),
            Self::Sound => include_str!("../assets/icons/sound.svg"),
            Self::Battery => include_str!("../assets/icons/battery.svg"),
            Self::Notifications => include_str!("../assets/icons/notifications.svg"),
        }
    }
}

impl From<IndicatorKind> for BuiltinIcon {
    fn from(kind: IndicatorKind) -> Self {
        match kind {
            IndicatorKind::Focus => Self::Focus,
            IndicatorKind::Vpn => Self::Vpn,
            IndicatorKind::Network => Self::Network,
            IndicatorKind::Bluetooth => Self::Bluetooth,
            IndicatorKind::Sound => Self::Sound,
            IndicatorKind::Battery => Self::Battery,
            IndicatorKind::Notifications => Self::Notifications,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemMark {
    pub icon: BuiltinIcon,
    pub accessible: &'static str,
}

impl Default for SystemMark {
    fn default() -> Self {
        Self {
            icon: BuiltinIcon::System,
            accessible: SYSTEM_MARK_ACCESSIBLE_NAME,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct IndicatorLabel {
    pub kind: IndicatorKind,
    pub icon: BuiltinIcon,
    pub activation: PanelTarget,
    pub visible: String,
    pub accessible: String,
    pub urgent: bool,
}

impl fmt::Debug for IndicatorLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndicatorLabel")
            .field("kind", &self.kind)
            .field("icon", &self.icon)
            .field("activation", &self.activation)
            .field("visible", &"<redacted>")
            .field("accessible", &"<redacted>")
            .field("urgent", &self.urgent)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub output: rmac_compositor::OutputId,
    pub logical_width: f64,
    pub logical_height: f64,
    pub scale: f64,
    pub exclusive_zone: f64,
    pub keyboard_interactive: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Content {
    pub system_mark: SystemMark,
    pub active_app: String,
    pub workspace: Option<String>,
    pub clock: ClockLabel,
    pub indicators: Vec<IndicatorLabel>,
}

impl fmt::Debug for Content {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Content")
            .field("system_mark", &self.system_mark)
            .field("active_app", &"<redacted>")
            .field("workspace", &self.workspace.as_ref().map(|_| "<redacted>"))
            .field("clock", &"<redacted>")
            .field("indicator_count", &self.indicators.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    pub surfaces: Vec<Surface>,
    pub content: Content,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub projection: Projection,
    /// True only when a renderer-visible value or output surface changed.
    pub redraw: bool,
    /// One event-driven clock deadline; never a frame-loop interval.
    pub next_clock_update: Duration,
}
