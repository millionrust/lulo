//! Stable keyboard preference model.

use std::fmt;

use rmac_locale::X11Keyboard;

/// What the Caps Lock key does. The choices and their order are macOS's
/// Modifier Keys sheet minus Globe, which PC keyboards do not have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CapsLockAction {
    #[default]
    CapsLock,
    Control,
    Option,
    Command,
    Escape,
    NoAction,
}

impl CapsLockAction {
    pub const ALL: [Self; 6] = [
        Self::CapsLock,
        Self::Control,
        Self::Option,
        Self::Command,
        Self::Escape,
        Self::NoAction,
    ];

    /// The label macOS shows in the Modifier Keys pop-up.
    pub fn label(self) -> &'static str {
        match self {
            Self::CapsLock => "⇪ Caps Lock",
            Self::Control => "⌃ Control",
            Self::Option => "⌥ Option",
            Self::Command => "⌘ Command",
            Self::Escape => "⎋ Escape",
            Self::NoAction => "No Action",
        }
    }

    /// Stable identifier used on the helper command line and in the keyd
    /// configuration header.
    pub fn id(self) -> &'static str {
        match self {
            Self::CapsLock => "caps-lock",
            Self::Control => "control",
            Self::Option => "option",
            Self::Command => "command",
            Self::Escape => "escape",
            Self::NoAction => "none",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }
}

/// Where the Mac modifiers sit on a PC keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PhysicalLayout {
    /// Make the key beside the space bar (PC Alt) ⌘ Command and the Windows
    /// key ⌥ Option, which puts both where a Mac keyboard has them.
    pub swap_command_option: bool,
    pub caps_lock: CapsLockAction,
    /// ⌥ types the characters printed on a Mac keyboard (⌥E then E = é,
    /// ⌥2 = €…) through the layout's XKB `mac` variant.
    pub option_characters: bool,
}

/// The complete system-wide Mac keyboard state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MacKeyboard {
    /// keyd translates ⌘ shortcuts for apps made for PC keyboards.
    pub shortcuts_in_all_apps: bool,
    pub layout: PhysicalLayout,
}

/// How the focused surface expects shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Profile {
    /// rmac apps, rmac shell surfaces and niri itself: ⌘ stays Super.
    Native,
    /// Apps made for PC keyboards: ⌘C becomes Control-C.
    PcApp,
    /// PC terminal emulators: ⌘C becomes Control-Shift-C so Control-C keeps
    /// interrupting, as in the Mac Terminal.
    Terminal,
}

/// Everything System Settings needs to draw the Mac keyboard controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: MacKeyboard,
    pub keyboard: X11Keyboard,
    /// keyd is installed, so "Use Mac shortcuts in all apps" can be offered.
    pub keyd_installed: bool,
    /// The rmac privileged helper is installed (packaged sessions only).
    pub helper_installed: bool,
    /// This login session may use keyd's socket. False right after the
    /// feature is turned on, until the user logs in again.
    pub session_can_bind: bool,
    /// Other keyd configurations rmac will not override.
    pub foreign_keyd_configs: Vec<String>,
    /// The primary layout has a Mac variant for ⌥ characters.
    pub option_characters_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    detail: String,
}

impl Error {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for Error {}
