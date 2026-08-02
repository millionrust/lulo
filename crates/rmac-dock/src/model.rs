//! Stable Dock item, action, and pin-command model.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowItem {
    pub id: rmac_compositor::WindowId,
    pub title: Option<String>,
    pub focused: bool,
    pub urgent: bool,
    pub focus_timestamp: Option<rmac_compositor::Timestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    /// Catalog identity when known, otherwise the compositor-provided app ID.
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
    pub pinned: bool,
    pub running: bool,
    pub active: bool,
    pub urgent: bool,
    pub launchable: bool,
    pub windows: Vec<WindowItem>,
    pub(super) launch: Option<rmac_apps::LaunchSpec>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SpecialItemKind {
    Files,
    Downloads,
    Trash,
}

#[derive(Clone, Eq, PartialEq)]
pub enum SpecialActivation {
    OpenDirectory {
        kind: SpecialItemKind,
        path: PathBuf,
    },
    OpenTrash,
    Unavailable {
        kind: SpecialItemKind,
        detail: String,
    },
}

impl fmt::Debug for SpecialActivation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenDirectory { kind, .. } => formatter
                .debug_struct("OpenDirectory")
                .field("kind", kind)
                .field("path", &"<private>")
                .finish(),
            Self::OpenTrash => formatter.write_str("OpenTrash"),
            Self::Unavailable { kind, detail } => formatter
                .debug_struct("Unavailable")
                .field("kind", kind)
                .field("detail", detail)
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpecialItem {
    pub kind: SpecialItemKind,
    pub name: &'static str,
    pub available: bool,
    /// Present only for an authoritative Trash snapshot. Renderers may use it
    /// for an item-count badge but must not infer availability from the count.
    pub item_count: Option<usize>,
    pub(super) activation: SpecialActivation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpecialContextAction {
    EmptyTrash { expected_item_count: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpecialContextMenu {
    pub kind: SpecialItemKind,
    pub open: SpecialActivation,
    /// Present only for an available, authoritatively nonempty Trash.
    pub empty_trash: Option<SpecialContextAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Activation {
    Launch {
        app_id: String,
        spec: rmac_apps::LaunchSpec,
    },
    FocusWindow(rmac_compositor::WindowId),
    NoAction,
    Unavailable {
        app_id: String,
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MoveDirection {
    Left,
    Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PinCommand {
    Pin {
        app_id: String,
    },
    Unpin {
        app_id: String,
    },
    Move {
        app_id: String,
        direction: MoveDirection,
    },
    MoveTo {
        app_id: String,
        index: usize,
    },
}

impl PinCommand {
    pub fn app_id(&self) -> &str {
        match self {
            Self::Pin { app_id }
            | Self::Unpin { app_id }
            | Self::Move { app_id, .. }
            | Self::MoveTo { app_id, .. } => app_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextAction {
    LaunchNew {
        app_id: String,
        spec: rmac_apps::LaunchSpec,
    },
    FocusWindow {
        app_id: String,
        window: rmac_compositor::WindowId,
    },
    CloseWindow {
        app_id: String,
        window: rmac_compositor::WindowId,
    },
    UpdatePins(PinCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowMenu {
    pub id: rmac_compositor::WindowId,
    pub title: String,
    pub focused: bool,
    pub urgent: bool,
    pub focus: ContextAction,
    pub close: ContextAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextMenu {
    pub app_id: String,
    pub application_name: String,
    pub launch_new: Option<ContextAction>,
    pub windows: Vec<WindowMenu>,
    pub pin: PinCommand,
    pub move_left: Option<PinCommand>,
    pub move_right: Option<PinCommand>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PinError {
    InvalidIdentity,
    NotPinned { app_id: String },
}

impl fmt::Display for PinError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity => formatter.write_str("application identity is empty"),
            Self::NotPinned { app_id } => write!(formatter, "{app_id} is not pinned"),
        }
    }
}

impl std::error::Error for PinError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Model {
    pub items: Vec<Item>,
    /// Files, Downloads, and Trash are kept after a renderer-owned separator;
    /// they are not application identities and cannot enter pinned ordering.
    pub special_items: Vec<SpecialItem>,
    pub(super) repeated_click: rmac_shell_settings::RepeatedClickBehavior,
}
