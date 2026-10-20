//! Stable shortcut, event, backend-status, and error model.

use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ShortcutId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShortcutSpec {
    pub id: ShortcutId,
    pub description: String,
    pub preferred_trigger: String,
    pub niri_trigger: String,
}

pub fn default_shortcuts() -> Vec<ShortcutSpec> {
    vec![
        shortcut("launcher", "Open Spotlight", "LOGO+space", "Mod+Space"),
        shortcut(
            "notification-center",
            "Open Notification Center",
            "LOGO+CTRL+n",
            "Mod+Ctrl+N",
        ),
        shortcut(
            "lock",
            "Lock the Lulo OS session",
            "LOGO+CTRL+q",
            "Mod+Ctrl+Q",
        ),
        // Hold ⌥Space and speak (ADR 0024 §8). The press starts the talk-key
        // HUD; it sees the release itself, once it has the keyboard.
        shortcut("voice", "Talk to Lulo", "ALT+space", "Alt+Space"),
    ]
}

/// Closes Spotlight if it is open; nothing otherwise.
pub const LAUNCHER_DISMISS: &str = "launcher-dismiss";
/// Closes Apps if it is open; nothing otherwise.
pub const APP_DRAWER_DISMISS: &str = "app-drawer-dismiss";

pub fn known_action(id: &ShortcutId) -> bool {
    matches!(
        id.0.as_str(),
        "launcher"
            | "app-drawer"
            | "notification-center"
            | "quick-settings"
            | "lock"
            | "power-key"
            | "shutdown-dialog"
            | "restart-to-update"
            | "menu-bar-focus"
            // ⌃F8: keyboard focus to the status menus (CC-05).
            | "status-menus-focus"
            // ⇧⌘Q: the Log Out confirmation (KB-01).
            | "log-out-dialog"
            | "voice"
            // ⌘Q and ⌘W, answered by the menu bar for every app (WIN-17).
            | "app-quit"
            | "window-close"
            // Spotlight and Apps are one panel on the Mac: opening one
            // closes the other (SPOT-04). Sent with `notify`, never bound.
            | LAUNCHER_DISMISS
            | APP_DRAWER_DISMISS
    )
}

fn shortcut(id: &str, description: &str, preferred: &str, niri: &str) -> ShortcutSpec {
    ShortcutSpec {
        id: ShortcutId(id.into()),
        description: description.into(),
        preferred_trigger: preferred.into(),
        niri_trigger: niri.into(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BackendStatus {
    Portal { version: u32, can_configure: bool },
    FallbackRequired { reason: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    Backend { status: BackendStatus },
    Bound { shortcuts: Vec<BoundShortcut> },
    Activated { id: ShortcutId, timestamp_ms: u64 },
    Deactivated { id: ShortcutId, timestamp_ms: u64 },
    BindingsChanged { shortcuts: Vec<BoundShortcut> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundShortcut {
    pub id: ShortcutId,
    pub description: String,
    pub trigger_description: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Validate,
    ConnectPortal,
    BindPortal,
    WatchPortal,
    Dispatch,
    WriteFallback,
    ResolveStatus,
    ReadStatus,
    ParseStatus,
    BindDispatch,
    ReadDispatch,
    ResolveControl,
    BindControl,
    ReadControl,
    RequestConfigure,
    ConfigurePortal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub detail: String,
}

impl Error {
    pub(super) fn new(operation: Operation, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Could not {:?}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}
