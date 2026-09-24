//! Minimal typed niri IPC wire protocol.

use super::*;
pub type Reply = Result<Response, String>;

#[derive(Debug, Serialize)]
pub enum Request {
    Outputs,
    Layers,
    Action(Action),
    EventStream,
}

#[derive(Debug, Serialize)]
pub enum Action {
    Spawn {
        command: Vec<String>,
    },
    CloseWindow {
        id: Option<u64>,
    },
    FocusWindow {
        id: u64,
    },
    FocusWorkspace {
        reference: WorkspaceReference,
    },
    FocusMonitor {
        output: String,
    },
    MoveWindowToWorkspace {
        window_id: Option<u64>,
        reference: WorkspaceReference,
        focus: bool,
    },
    MoveWindowToMonitor {
        id: Option<u64>,
        output: String,
    },
    OpenOverview {},
    CloseOverview {},
    FullscreenWindow {},
    CenterWindow {},
    SetWindowWidth {
        id: Option<u64>,
        change: SizeChange,
    },
    SetWindowHeight {
        id: Option<u64>,
        change: SizeChange,
    },
    MoveFloatingWindow {
        id: Option<u64>,
        x: PositionChange,
        y: PositionChange,
    },
    SetWorkspaceName {
        name: String,
        workspace: Option<WorkspaceReference>,
    },
    UnsetWorkspaceName {
        reference: Option<WorkspaceReference>,
    },
    DoScreenTransition {
        delay_ms: Option<u16>,
    },
}

/// niri-ipc `SizeChange`; proportions are percentages of the working area
/// ("50%" on the command line is `SetProportion(50.0)`).
#[derive(Debug, Serialize)]
pub enum SizeChange {
    SetProportion(f64),
}

/// niri-ipc `PositionChange` for floating windows: a percentage of the
/// working area, or a relative move in logical pixels.
#[derive(Debug, Serialize)]
pub enum PositionChange {
    SetProportion(f64),
    AdjustFixed(f64),
}

#[derive(Debug, Serialize)]
pub enum WorkspaceReference {
    Id(u64),
    Name(String),
}

#[derive(Debug, Deserialize)]
pub enum Response {
    Handled,
    Outputs(HashMap<String, Output>),
    Layers(Vec<LayerSurface>),
}

#[derive(Debug, Deserialize)]
pub enum Event {
    WorkspacesChanged {
        workspaces: Vec<Workspace>,
    },
    WorkspaceUrgencyChanged {
        id: u64,
        urgent: bool,
    },
    WorkspaceActivated {
        id: u64,
        focused: bool,
    },
    WorkspaceActiveWindowChanged {
        workspace_id: u64,
        active_window_id: Option<u64>,
    },
    WindowsChanged {
        windows: Vec<Window>,
    },
    WindowOpenedOrChanged {
        window: Window,
    },
    WindowClosed {
        id: u64,
    },
    WindowFocusChanged {
        id: Option<u64>,
    },
    WindowFocusTimestampChanged {
        id: u64,
        focus_timestamp: Option<Timestamp>,
    },
    WindowUrgencyChanged {
        id: u64,
        urgent: bool,
    },
    WindowLayoutsChanged {
        changes: Vec<(u64, WindowLayout)>,
    },
    OverviewOpenedOrClosed {
        is_open: bool,
    },
}

#[derive(Debug, Deserialize)]
pub struct Output {
    pub name: String,
    pub make: String,
    pub model: String,
    pub serial: Option<String>,
    pub physical_size: Option<(u32, u32)>,
    pub modes: Vec<Mode>,
    pub current_mode: Option<usize>,
    pub is_custom_mode: bool,
    pub vrr_supported: bool,
    pub vrr_enabled: bool,
    pub logical: Option<LogicalOutput>,
}

#[derive(Debug, Deserialize)]
pub struct Mode {
    pub width: u16,
    pub height: u16,
    pub refresh_rate: u32,
    pub is_preferred: bool,
}

#[derive(Debug, Deserialize)]
pub struct LogicalOutput {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub transform: String,
}

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub id: u64,
    pub idx: u8,
    pub name: Option<String>,
    pub output: Option<String>,
    pub is_urgent: bool,
    pub is_active: bool,
    pub is_focused: bool,
    pub active_window_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct Window {
    pub id: u64,
    pub title: Option<String>,
    pub app_id: Option<String>,
    pub pid: Option<i32>,
    pub workspace_id: Option<u64>,
    pub is_focused: bool,
    pub is_floating: bool,
    pub is_urgent: bool,
    pub layout: WindowLayout,
    pub focus_timestamp: Option<Timestamp>,
}

#[derive(Debug, Deserialize)]
pub struct WindowLayout {
    pub pos_in_scrolling_layout: Option<(usize, usize)>,
    pub tile_size: (f64, f64),
    pub window_size: (i32, i32),
    pub tile_pos_in_workspace_view: Option<(f64, f64)>,
    pub window_offset_in_tile: (f64, f64),
}

#[derive(Debug, Deserialize)]
pub struct Timestamp {
    pub secs: u64,
    pub nanos: u32,
}

#[derive(Debug, Deserialize)]
pub struct LayerSurface {
    pub namespace: String,
    pub output: String,
    pub layer: String,
    pub keyboard_interactivity: String,
}
