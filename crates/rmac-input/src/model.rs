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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TouchpadSettings {
    pub pointer: PointerSettings,
    pub tap_to_click: bool,
    pub disable_while_typing: bool,
    pub drag_lock: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputSettings {
    pub keyboard: KeyboardSettings,
    pub mouse: PointerSettings,
    pub touchpad: TouchpadSettings,
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
