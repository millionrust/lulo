//! Stable Dock item, action, and pin-command model.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowItem {
    pub id: rmac_compositor::WindowId,
    pub title: Option<String>,
    /// Positive compositor-reported process identity when available.
    pub pid: Option<u32>,
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
    pub(super) source: Option<PathBuf>,
    pub(super) actions: Vec<rmac_apps::DesktopAction>,
    /// MIME types the desktop entry says it opens; decides file drops.
    pub(super) mime_types: Vec<String>,
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

/// A window the shell parked on the hidden workspace, shown as a tile in the
/// Dock's right group until it is restored (§4.11).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MinimizedItem {
    pub window: rmac_compositor::WindowId,
    pub app_id: Option<String>,
    pub title: Option<String>,
    /// Application icon drawn as the tile's badge.
    pub icon: Option<PathBuf>,
    /// Snapshot captured just before the window was parked, when available.
    pub thumbnail: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Activation {
    Launch {
        app_id: String,
        spec: rmac_apps::LaunchSpec,
    },
    FocusWindow(rmac_compositor::WindowId),
    /// Bring every window of an application forward, as a macOS Dock click
    /// does: the windows are focused back to front so the most recently used
    /// one (the last entry) ends up focused on top.
    FocusApplication {
        app_id: String,
        windows: Vec<rmac_compositor::WindowId>,
    },
    /// Restore a parked window to the workspace it was minimized from.
    RestoreWindow {
        window: rmac_compositor::WindowId,
    },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationKind {
    Quit,
    ForceQuit,
}

#[derive(Clone, Eq, PartialEq)]
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
    RevealApplication {
        app_id: String,
        source: PathBuf,
    },
    TerminateApplication {
        app_id: String,
        pids: Vec<u32>,
        kind: TerminationKind,
    },
    UpdatePins(PinCommand),
    /// Hide: park these visible windows, recording where each came from.
    HideApplication {
        app_id: String,
        windows: Vec<rmac_compositor::WindowId>,
    },
    /// Option-Hide (Hide Others): park every other application's visible
    /// windows.
    HideOthers {
        app_id: String,
        windows: Vec<rmac_compositor::WindowId>,
    },
    /// Show All Windows: App Exposé for this application, starting from its
    /// most recent window.
    ShowAllWindows {
        app_id: String,
        window: rmac_compositor::WindowId,
    },
}

impl fmt::Debug for ContextAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LaunchNew { app_id, .. } => formatter
                .debug_struct("LaunchNew")
                .field("app_id", app_id)
                .field("spec", &"<private>")
                .finish(),
            Self::FocusWindow { app_id, window } => formatter
                .debug_struct("FocusWindow")
                .field("app_id", app_id)
                .field("window", window)
                .finish(),
            Self::CloseWindow { app_id, window } => formatter
                .debug_struct("CloseWindow")
                .field("app_id", app_id)
                .field("window", window)
                .finish(),
            Self::RevealApplication { app_id, .. } => formatter
                .debug_struct("RevealApplication")
                .field("app_id", app_id)
                .field("source", &"<private>")
                .finish(),
            Self::TerminateApplication { app_id, kind, .. } => formatter
                .debug_struct("TerminateApplication")
                .field("app_id", app_id)
                .field("kind", kind)
                .field("pids", &"<redacted>")
                .finish(),
            Self::UpdatePins(command) => {
                formatter.debug_tuple("UpdatePins").field(command).finish()
            }
            Self::HideApplication { app_id, windows } => formatter
                .debug_struct("HideApplication")
                .field("app_id", app_id)
                .field("windows", &windows.len())
                .finish(),
            Self::HideOthers { app_id, windows } => formatter
                .debug_struct("HideOthers")
                .field("app_id", app_id)
                .field("windows", &windows.len())
                .finish(),
            Self::ShowAllWindows { app_id, window } => formatter
                .debug_struct("ShowAllWindows")
                .field("app_id", app_id)
                .field("window", window)
                .finish(),
        }
    }
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
pub struct ApplicationCommand {
    pub id: String,
    pub name: String,
    pub action: ContextAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextMenu {
    pub app_id: String,
    pub application_name: String,
    pub open: Option<ContextAction>,
    pub application_commands: Vec<ApplicationCommand>,
    pub windows: Vec<WindowMenu>,
    pub show_in_finder: Option<ContextAction>,
    pub pin: PinCommand,
    pub quit: Option<ContextAction>,
    pub force_quit: Option<ContextAction>,
    /// Running applications only: Show All Windows, Hide, and Hide Others
    /// (Hide's Option alternative).
    pub show_all_windows: Option<ContextAction>,
    pub hide: Option<ContextAction>,
    pub hide_others: Option<ContextAction>,
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
    /// Non-application endpoints kept after a renderer-owned separator. The
    /// default projection contains only Trash; Files remains a configured app
    /// and optional folder stacks require persisted user configuration.
    pub special_items: Vec<SpecialItem>,
    /// Parked windows shown as minimized tiles ahead of the special items when
    /// "minimize into application icon" is off (§4.11). These windows are
    /// excluded from `items` so they never count as running applications.
    pub minimized: Vec<MinimizedItem>,
    pub(super) repeated_click: rmac_shell_settings::RepeatedClickBehavior,
}
