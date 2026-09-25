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
            "lock",
            "Lock the Lulo OS session",
            "LOGO+CTRL+q",
            "Mod+Ctrl+Q",
        ),
    ]
}

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
