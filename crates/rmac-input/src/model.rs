//! Stable input service model.

use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccelProfile {
    #[default]
    Adaptive,
    Flat,
}

impl AccelProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Adaptive => "Adaptive",
            Self::Flat => "Flat",
        }
    }

    #[cfg(not(windows))]
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Flat => "flat",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardSettings {
    pub repeat_delay_ms: u32,
    pub repeat_rate: u32,
    pub numlock: bool,
    pub xkb_override: Option<XkbOverride>,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        Self {
            repeat_delay_ms: 600,
            repeat_rate: 25,
            numlock: false,
            xkb_override: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct XkbOverride {
    pub rules: String,
    pub layout: String,
    pub model: String,
    pub variant: String,
    pub options: Option<String>,
    pub file: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardLayoutAuthority {
    SystemLocaled,
    NiriConfig,
    IncludedConfig,
    #[default]
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceKind {
    Keyboard,
    Mouse,
    Touchpad,
    Trackpoint,
    Trackball,
    Tablet,
    Touchscreen,
    Other,
}

impl DeviceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Keyboard => "Keyboard",
            Self::Mouse => "Mouse",
            Self::Touchpad => "Trackpad",
            Self::Trackpoint => "Pointing Stick",
            Self::Trackball => "Trackball",
            Self::Tablet => "Tablet",
            Self::Touchscreen => "Touchscreen",
            Self::Other => "Input Device",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDevice {
    /// Ephemeral kernel event identity. Never persisted as a hardware identity.
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DeviceOverrideCapability {
    #[default]
    Unsupported,
}

impl DeviceOverrideCapability {
    pub fn detail(&self) -> &'static str {
        match self {
            Self::Unsupported => {
                "This niri version applies settings by device type and does not support individual-device overrides."
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerSettings {
    pub enabled: bool,
    pub natural_scroll: bool,
    pub accel_speed: f64,
    pub accel_profile: AccelProfile,
    pub left_handed: bool,
    pub middle_emulation: bool,
}

impl Default for PointerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            natural_scroll: false,
            accel_speed: 0.0,
            accel_profile: AccelProfile::Adaptive,
            left_handed: false,
            middle_emulation: false,
        }
    }
}

/// libinput's touchpad `click-method`: how a secondary (right) click is
/// produced from a device with no dedicated secondary button. The Mac also
/// offers a bottom-left corner, which libinput's `button-areas` method does
/// not support choosing independently of `left-handed`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SecondaryClick {
    /// `clickfinger`: click or tap with two fingers anywhere on the pad.
    TwoFingerClickOrTap,
    /// `button-areas`: click or tap in a screen-bottom corner.
    #[default]
    CornerClick,
}

impl SecondaryClick {
    pub fn label(self) -> &'static str {
        match self {
            Self::TwoFingerClickOrTap => "Click or Tap with Two Fingers",
            Self::CornerClick => "Click or Tap in Corner",
        }
    }

    #[cfg(not(windows))]
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::TwoFingerClickOrTap => "clickfinger",
            Self::CornerClick => "button-areas",
        }
    }

    #[cfg(not(windows))]
    pub(super) fn from_id(value: &str) -> Option<Self> {
        match value {
            "clickfinger" => Some(Self::TwoFingerClickOrTap),
            "button-areas" => Some(Self::CornerClick),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TouchpadSettings {
    pub pointer: PointerSettings,
    pub tap_to_click: bool,
    pub disable_while_typing: bool,
    pub drag_lock: bool,
    pub secondary_click: SecondaryClick,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TouchSettings {
    pub enabled: bool,
    pub map_to_output: Option<String>,
}

impl TouchSettings {
    pub fn default_enabled() -> Self {
        Self {
            enabled: true,
            map_to_output: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputSettings {
    pub keyboard: KeyboardSettings,
    pub mouse: PointerSettings,
    pub mouse_scroll_with_middle_button: bool,
    pub touchpad: TouchpadSettings,
    pub trackpoint: PointerSettings,
    pub trackpoint_scroll_with_middle_button: bool,
    pub touch: TouchSettings,
    /// Maximum milliseconds between the two clicks of a double-click, for
    /// every pointing device. `None` where the platform has no such setting
    /// (niri); see [`Supported::double_click_time`].
    pub double_click_ms: Option<u32>,
    /// Lines one mouse-wheel notch scrolls; [`WHEEL_SCROLL_PAGE`] scrolls a
    /// whole screen. `None` where the platform has no such setting (niri);
    /// see [`Supported::wheel_scroll_lines`].
    pub wheel_scroll_lines: Option<u32>,
}

/// [`InputSettings::wheel_scroll_lines`] value meaning "one screen at a time".
pub const WHEEL_SCROLL_PAGE: u32 = u32::MAX;

impl Default for InputSettings {
    fn default() -> Self {
        Self {
            keyboard: KeyboardSettings::default(),
            mouse: PointerSettings::default(),
            mouse_scroll_with_middle_button: false,
            touchpad: TouchpadSettings::default(),
            trackpoint: PointerSettings::default(),
            trackpoint_scroll_with_middle_button: false,
            touch: TouchSettings::default_enabled(),
            double_click_ms: None,
            wheel_scroll_lines: None,
        }
    }
}

/// Which [`InputSettings`] fields this platform can change. Settings hides a
/// control whose flag is false, and `save` rejects a change to such a field.
///
/// The default is niri's set: every field except the two Windows-only
/// pointer settings, so Linux behaviour does not depend on these flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Supported {
    /// `keyboard.repeat_delay_ms` and `keyboard.repeat_rate`.
    pub keyboard_repeat: bool,
    pub keyboard_numlock: bool,
    pub mouse_enabled: bool,
    pub mouse_natural_scroll: bool,
    pub mouse_accel_speed: bool,
    pub mouse_accel_profile: bool,
    pub mouse_left_handed: bool,
    pub mouse_middle_emulation: bool,
    pub mouse_scroll_with_middle_button: bool,
    /// The touchpad section applies at all: false on Windows when no
    /// Precision Touchpad is present. Each touchpad field also has its own
    /// flag below.
    pub touchpad: bool,
    pub touchpad_enabled: bool,
    pub touchpad_natural_scroll: bool,
    pub touchpad_accel_speed: bool,
    pub touchpad_accel_profile: bool,
    pub touchpad_left_handed: bool,
    pub touchpad_middle_emulation: bool,
    pub touchpad_tap_to_click: bool,
    pub touchpad_disable_while_typing: bool,
    pub touchpad_drag_lock: bool,
    pub touchpad_secondary_click: bool,
    /// Every `trackpoint` field and `trackpoint_scroll_with_middle_button`.
    pub trackpoint: bool,
    /// Every `touch` field.
    pub touch: bool,
    /// [`InputSettings::double_click_ms`].
    pub double_click_time: bool,
    /// [`InputSettings::wheel_scroll_lines`].
    pub wheel_scroll_lines: bool,
}

impl Default for Supported {
    fn default() -> Self {
        Self {
            keyboard_repeat: true,
            keyboard_numlock: true,
            mouse_enabled: true,
            mouse_natural_scroll: true,
            mouse_accel_speed: true,
            mouse_accel_profile: true,
            mouse_left_handed: true,
            mouse_middle_emulation: true,
            mouse_scroll_with_middle_button: true,
            touchpad: true,
            touchpad_enabled: true,
            touchpad_natural_scroll: true,
            touchpad_accel_speed: true,
            touchpad_accel_profile: true,
            touchpad_left_handed: true,
            touchpad_middle_emulation: true,
            touchpad_tap_to_click: true,
            touchpad_disable_while_typing: true,
            touchpad_drag_lock: true,
            touchpad_secondary_click: true,
            trackpoint: true,
            touch: true,
            double_click_time: false,
            wheel_scroll_lines: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub can_configure: bool,
    pub config_path: Option<PathBuf>,
    pub detail: Option<String>,
    pub keyboard_layout_authority: KeyboardLayoutAuthority,
    pub settings: InputSettings,
    pub included_files: usize,
    pub devices: Vec<InputDevice>,
    pub device_detail: Option<String>,
    pub device_overrides: DeviceOverrideCapability,
    pub supported: Supported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    WatchError(String),
}

#[derive(Debug)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    pub(super) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}
