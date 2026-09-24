//! Appearance accessibility vocabulary and bounded public model.

use std::fmt;

pub const APPEARANCE_TITLE: &str = "Appearance";
pub const REFRESH_ID: &str = "theme-refresh";
pub const REFRESH_LABEL: &str = "Try Again";
pub const LOADING_LABEL: &str = "Loading appearance preferences…";
pub const APPLYING_LABEL: &str = "Applying appearance preferences…";
pub const UNAVAILABLE_LABEL: &str = "The Lulo OS theme preference service is unavailable.";
pub const HOST_UNAVAILABLE_LABEL: &str = "The Linux Settings portal is unavailable. Automatic values use safe Lulo OS defaults; explicit choices remain writable.";
pub const MAX_TEXT_VALUE_BYTES: usize = 16 * 1024;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneState {
    Loading,
    Unavailable,
    Ready,
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LivePoliteness {
    Polite,
    Assertive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccentChoice {
    Automatic,
    Blue,
    Purple,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Graphite,
}

impl AccentChoice {
    pub const ALL: [Self; 9] = [
        Self::Automatic,
        Self::Blue,
        Self::Purple,
        Self::Pink,
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Graphite,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Pink => "Pink",
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Graphite => "Graphite",
        }
    }

    pub const fn hex(self) -> Option<u32> {
        match self {
            Self::Automatic => None,
            Self::Blue => Some(0x1372f9),
            Self::Purple => Some(0xaf52de),
            Self::Pink => Some(0xff2d55),
            Self::Red => Some(0xff3b30),
            Self::Orange => Some(0xff9500),
            Self::Yellow => Some(0xffcc00),
            Self::Green => Some(0x34c759),
            Self::Graphite => Some(0x8e8e93),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppearanceAction {
    Refresh,
    SetScheme(rmac_theme::SchemePreference),
    SetAccent(AccentChoice),
    SetContrast(rmac_theme::ContrastPreference),
    SetMotion(rmac_theme::MotionPreferenceSetting),
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleAction {
    pub id: String,
    pub name: &'static str,
    pub kind: AppearanceAction,
    pub enabled: bool,
    pub selected: bool,
}

impl fmt::Debug for AccessibleAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleAction")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("enabled", &self.enabled)
            .field("selected", &self.selected)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChoiceGroupKind {
    Scheme,
    Accent,
    Contrast,
    Motion,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AccessibleChoiceGroup {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: ChoiceGroupKind,
    pub value_text: &'static str,
    pub busy: bool,
    pub choices: Vec<AccessibleAction>,
}

impl fmt::Debug for AccessibleChoiceGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleChoiceGroup")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("value_text", &self.value_text)
            .field("busy", &self.busy)
            .field("choice_count", &self.choices.len())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibleAuthority {
    pub host_preference: &'static str,
    pub host_preference_exposed: bool,
    pub effective_appearance: &'static str,
}

#[derive(Clone, Eq, PartialEq)]
pub struct LiveAnnouncement {
    pub id: &'static str,
    pub text: String,
    pub politeness: LivePoliteness,
}

impl fmt::Debug for LiveAnnouncement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveAnnouncement")
            .field("id", &self.id)
            .field("text", &"<redacted>")
            .field("politeness", &self.politeness)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AppearanceAccessibilitySnapshot {
    pub title: &'static str,
    pub state: PaneState,
    pub refresh_action: AccessibleAction,
    pub groups: Vec<AccessibleChoiceGroup>,
    pub authority: Option<AccessibleAuthority>,
    pub detail: Option<String>,
    pub keyboard_order: Vec<String>,
    pub initial_focus: Option<String>,
    pub announcements: Vec<LiveAnnouncement>,
}

impl fmt::Debug for AppearanceAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppearanceAccessibilitySnapshot")
            .field("state", &self.state)
            .field("refresh_action", &self.refresh_action)
            .field("group_count", &self.groups.len())
            .field("has_authority", &self.authority.is_some())
            .field("has_detail", &self.detail.is_some())
            .field("keyboard_action_count", &self.keyboard_order.len())
            .field("initial_focus", &self.initial_focus)
            .field("announcement_count", &self.announcements.len())
            .finish()
    }
}

#[derive(Clone, Copy)]
pub struct AppearanceInput<'a> {
    pub theme: Option<&'a rmac_theme::Snapshot>,
    pub host: &'a rmac_appearance::Snapshot,
    pub loading: bool,
    pub busy: bool,
    pub refreshing: bool,
    pub error: Option<&'a str>,
}

impl fmt::Debug for AppearanceInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppearanceInput")
            .field("has_theme", &self.theme.is_some())
            .field("host_available", &self.host.available)
            .field("loading", &self.loading)
            .field("busy", &self.busy)
            .field("refreshing", &self.refreshing)
            .field("has_error", &self.error.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    InvalidAccent,
    DuplicateAction,
    InvalidSelection,
    InvalidText,
    TextValueLimit,
    TextLimit,
}
