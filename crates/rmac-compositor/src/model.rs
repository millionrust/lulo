use serde::{Deserialize, Serialize};

macro_rules! numeric_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

numeric_id!(WorkspaceId);
numeric_id!(WindowId);
numeric_id!(ActivationId);

/// Stable output identity. For niri this is the compositor output name, not a
/// connector index or presentation order.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct OutputId(pub String);

impl From<&str> for OutputId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Adapter-assigned identity for a layer surface, whose source protocol may
/// expose no stable numeric id.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct LayerSurfaceId(pub String);

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LogicalPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LogicalSize {
    pub width: f64,
    pub height: f64,
}

impl LogicalSize {
    pub fn is_valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width >= 0.0 && self.height >= 0.0
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OutputMode {
    pub physical_size: PhysicalSize,
    /// Refresh rate in millihertz.
    pub refresh_millihz: u32,
    pub preferred: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LogicalOutput {
    pub position: LogicalPoint,
    pub size: LogicalSize,
    pub scale: f64,
    /// Adapter-provided transform name. Unknown future transforms survive.
    pub transform: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Output {
    pub id: OutputId,
    pub make: String,
    pub model: String,
    /// Serial is retained in memory for identity diagnostics but must be
    /// redacted from default logs and evidence bundles.
    pub serial: Option<String>,
    pub physical_size_mm: Option<PhysicalSize>,
    pub modes: Vec<OutputMode>,
    pub current_mode: Option<usize>,
    pub custom_mode: bool,
    pub vrr_supported: bool,
    pub vrr_enabled: bool,
    pub logical: Option<LogicalOutput>,
}

impl Output {
    pub fn enabled(&self) -> bool {
        self.current_mode.is_some() && self.logical.is_some()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    /// Current one-based position on its output; not a stable identity.
    pub index: u8,
    pub name: Option<String>,
    pub output: Option<OutputId>,
    pub urgent: bool,
    pub active: bool,
    pub focused: bool,
    pub active_window: Option<WindowId>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Timestamp {
    pub seconds: u64,
    pub nanoseconds: u32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WindowLayout {
    /// One-based (column, tile-in-column) position for tiled windows.
    pub scrolling_position: Option<(usize, usize)>,
    pub tile_size: LogicalSize,
    pub tile_position_in_view: Option<LogicalPoint>,
    pub window_size: PhysicalSize,
    pub window_offset_in_tile: LogicalPoint,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Window {
    pub id: WindowId,
    pub title: Option<String>,
    pub app_id: Option<String>,
    /// Kept signed because the compositor wire contract is signed.
    pub pid: Option<i32>,
    pub workspace: Option<WorkspaceId>,
    pub focused: bool,
    pub floating: bool,
    pub urgent: bool,
    pub focus_timestamp: Option<Timestamp>,
    pub layout: WindowLayout,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layer {
    Background,
    Bottom,
    Top,
    Overlay,
    Other(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyboardInteractivity {
    None,
    Exclusive,
    OnDemand,
    Other(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LayerSurface {
    pub id: LayerSurfaceId,
    pub namespace: String,
    pub output: OutputId,
    pub layer: Layer,
    pub keyboard_interactivity: KeyboardInteractivity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FocusTarget {
    Output(OutputId),
    Workspace(WorkspaceId),
    Window(WindowId),
    LayerSurface(LayerSurfaceId),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FocusState {
    pub target: Option<FocusTarget>,
    pub output: Option<OutputId>,
    pub workspace: Option<WorkspaceId>,
    pub window: Option<WindowId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationTarget {
    Output(OutputId),
    Workspace(WorkspaceId),
    Window(WindowId),
    Overview,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationStatus {
    Pending,
    Confirmed,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Activation {
    pub id: ActivationId,
    pub target: ActivationTarget,
    pub status: ActivationStatus,
    pub error: Option<String>,
}
