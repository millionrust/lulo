//! Compositor-independent desktop state for the rmac session.
//!
//! This crate deliberately contains no GPUI, socket, niri, or Wayland types.
//! Adapters translate their wire format into these stable identities, snapshots,
//! and incremental events. Temporary dangling references are valid because a
//! compositor event stream may report workspace and window replacements in
//! separate messages.

use std::collections::BTreeMap;

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UrgencyTarget {
    Workspace(WorkspaceId),
    Window(WindowId),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Snapshot {
    pub outputs: Vec<Output>,
    pub workspaces: Vec<Workspace>,
    pub windows: Vec<Window>,
    pub layer_surfaces: Vec<LayerSurface>,
    pub focus: FocusState,
    pub activation: Option<Activation>,
}

/// Adapter-neutral events. `Unknown` is intentionally data-bearing so adapters
/// can preserve and diagnose future compositor events without crashing.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    Snapshot {
        snapshot: Snapshot,
    },
    OutputsReplaced {
        outputs: Vec<Output>,
    },
    WorkspacesReplaced {
        workspaces: Vec<Workspace>,
    },
    WorkspaceUpserted {
        workspace: Workspace,
    },
    WorkspaceRemoved {
        id: WorkspaceId,
    },
    WindowsReplaced {
        windows: Vec<Window>,
    },
    WindowUpserted {
        window: Window,
    },
    WindowRemoved {
        id: WindowId,
    },
    LayerSurfacesReplaced {
        layer_surfaces: Vec<LayerSurface>,
    },
    FocusChanged {
        focus: FocusState,
    },
    UrgencyChanged {
        target: UrgencyTarget,
        urgent: bool,
    },
    ActivationChanged {
        activation: Option<Activation>,
    },
    ConnectionChanged {
        state: ConnectionState,
    },
    Unknown {
        source_kind: String,
        payload: serde_json::Value,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Change {
    pub visible: bool,
    pub topology: bool,
    pub focus: bool,
    pub urgency: bool,
}

impl Change {
    fn topology() -> Self {
        Self {
            visible: true,
            topology: true,
            ..Self::default()
        }
    }

    fn focus() -> Self {
        Self {
            visible: true,
            focus: true,
            ..Self::default()
        }
    }

    fn urgency() -> Self {
        Self {
            visible: true,
            urgency: true,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub outputs: BTreeMap<OutputId, Output>,
    pub workspaces: BTreeMap<WorkspaceId, Workspace>,
    pub windows: BTreeMap<WindowId, Window>,
    pub layer_surfaces: BTreeMap<LayerSurfaceId, LayerSurface>,
    pub focus: FocusState,
    pub activation: Option<Activation>,
    pub connection: ConnectionState,
    pub unknown_events_seen: u64,
}

impl State {
    pub fn apply(&mut self, event: Event) -> Change {
        match event {
            Event::Snapshot { snapshot } => {
                self.outputs = keyed(snapshot.outputs, |output| output.id.clone());
                self.workspaces = keyed(snapshot.workspaces, |workspace| workspace.id);
                self.windows = keyed(snapshot.windows, |window| window.id);
                self.layer_surfaces = keyed(snapshot.layer_surfaces, |surface| surface.id.clone());
                self.focus = snapshot.focus;
                self.activation = snapshot.activation;
                self.apply_focus_flags();
                Change {
                    visible: true,
                    topology: true,
                    focus: true,
                    urgency: true,
                }
            }
            Event::OutputsReplaced { outputs } => {
                self.outputs = keyed(outputs, |output| output.id.clone());
                Change::topology()
            }
            Event::WorkspacesReplaced { workspaces } => {
                self.workspaces = keyed(workspaces, |workspace| workspace.id);
                self.apply_focus_flags();
                Change::topology()
            }
            Event::WorkspaceUpserted { workspace } => {
                let changed = self.workspaces.get(&workspace.id) != Some(&workspace);
                self.workspaces.insert(workspace.id, workspace);
                self.apply_focus_flags();
                Change {
                    visible: changed,
                    topology: changed,
                    ..Change::default()
                }
            }
            Event::WorkspaceRemoved { id } => {
                let changed = self.workspaces.remove(&id).is_some();
                if self.focus.workspace == Some(id) {
                    self.focus.workspace = None;
                    if matches!(self.focus.target, Some(FocusTarget::Workspace(target)) if target == id)
                    {
                        self.focus.target = None;
                    }
                }
                Change {
                    visible: changed,
                    topology: changed,
                    focus: changed,
                    ..Change::default()
                }
            }
            Event::WindowsReplaced { windows } => {
                self.windows = keyed(windows, |window| window.id);
                self.apply_focus_flags();
                Change::topology()
            }
            Event::WindowUpserted { window } => {
                let changed = self.windows.get(&window.id) != Some(&window);
                self.windows.insert(window.id, window);
                self.apply_focus_flags();
                Change {
                    visible: changed,
                    topology: changed,
                    ..Change::default()
                }
            }
            Event::WindowRemoved { id } => {
                let changed = self.windows.remove(&id).is_some();
                if self.focus.window == Some(id) {
                    self.focus.window = None;
                    if matches!(self.focus.target, Some(FocusTarget::Window(target)) if target == id)
                    {
                        self.focus.target = None;
                    }
                }
                for workspace in self.workspaces.values_mut() {
                    if workspace.active_window == Some(id) {
                        workspace.active_window = None;
                    }
                }
                Change {
                    visible: changed,
                    topology: changed,
                    focus: changed,
                    ..Change::default()
                }
            }
            Event::LayerSurfacesReplaced { layer_surfaces } => {
                self.layer_surfaces = keyed(layer_surfaces, |surface| surface.id.clone());
                Change::topology()
            }
            Event::FocusChanged { focus } => {
                let changed = self.focus != focus;
                self.focus = focus;
                self.apply_focus_flags();
                if changed {
                    Change::focus()
                } else {
                    Change::default()
                }
            }
            Event::UrgencyChanged { target, urgent } => {
                let changed = match target {
                    UrgencyTarget::Workspace(id) => self
                        .workspaces
                        .get_mut(&id)
                        .map(|workspace| replace_bool(&mut workspace.urgent, urgent))
                        .unwrap_or(false),
                    UrgencyTarget::Window(id) => self
                        .windows
                        .get_mut(&id)
                        .map(|window| replace_bool(&mut window.urgent, urgent))
                        .unwrap_or(false),
                };
                if changed {
                    Change::urgency()
                } else {
                    Change::default()
                }
            }
            Event::ActivationChanged { activation } => {
                let changed = self.activation != activation;
                self.activation = activation;
                Change {
                    visible: changed,
                    ..Change::default()
                }
            }
            Event::ConnectionChanged { state } => {
                let changed = self.connection != state;
                self.connection = state;
                Change {
                    visible: changed,
                    ..Change::default()
                }
            }
            Event::Unknown { .. } => {
                self.unknown_events_seen = self.unknown_events_seen.saturating_add(1);
                Change::default()
            }
        }
    }

    /// Cross-object references may be temporarily dangling, so validation only
    /// reports true local invariants and never rejects event ordering allowed by
    /// the compositor.
    pub fn validate(&self) -> Vec<InvariantViolation> {
        let mut violations = Vec::new();
        if self
            .windows
            .values()
            .filter(|window| window.focused)
            .count()
            > 1
        {
            violations.push(InvariantViolation::MultipleFocusedWindows);
        }
        if self
            .workspaces
            .values()
            .filter(|workspace| workspace.focused)
            .count()
            > 1
        {
            violations.push(InvariantViolation::MultipleFocusedWorkspaces);
        }
        for output in self.outputs.values() {
            if let Some(logical) = output.logical.as_ref() {
                if !logical.scale.is_finite() || logical.scale <= 0.0 {
                    violations.push(InvariantViolation::InvalidOutputScale(output.id.clone()));
                }
                if !logical.size.is_valid() {
                    violations.push(InvariantViolation::InvalidLogicalSize(output.id.clone()));
                }
            }
            if output
                .current_mode
                .is_some_and(|index| index >= output.modes.len())
            {
                violations.push(InvariantViolation::InvalidCurrentMode(output.id.clone()));
            }
        }
        violations
    }

    fn apply_focus_flags(&mut self) {
        for window in self.windows.values_mut() {
            window.focused = self.focus.window == Some(window.id);
        }
        for workspace in self.workspaces.values_mut() {
            workspace.focused = self.focus.workspace == Some(workspace.id);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvariantViolation {
    MultipleFocusedWindows,
    MultipleFocusedWorkspaces,
    InvalidOutputScale(OutputId),
    InvalidLogicalSize(OutputId),
    InvalidCurrentMode(OutputId),
}

fn keyed<K: Ord, V>(values: Vec<V>, key: impl Fn(&V) -> K) -> BTreeMap<K, V> {
    values
        .into_iter()
        .map(|value| (key(&value), value))
        .collect()
}

fn replace_bool(slot: &mut bool, value: bool) -> bool {
    let changed = *slot != value;
    *slot = value;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: u64, output: Option<&str>) -> Workspace {
        Workspace {
            id: WorkspaceId(id),
            index: id as u8,
            name: None,
            output: output.map(OutputId::from),
            urgent: false,
            active: true,
            focused: false,
            active_window: None,
        }
    }

    fn window(id: u64, workspace: Option<u64>) -> Window {
        Window {
            id: WindowId(id),
            title: Some(format!("Window {id}")),
            app_id: Some("org.rmac.Test".into()),
            pid: Some(42),
            workspace: workspace.map(WorkspaceId),
            focused: false,
            floating: false,
            urgent: false,
            focus_timestamp: None,
            layout: WindowLayout::default(),
        }
    }

    #[test]
    fn focus_event_canonicalizes_window_and_workspace_flags() {
        let mut state = State::default();
        state.apply(Event::WorkspacesReplaced {
            workspaces: vec![workspace(1, Some("eDP-1")), workspace(2, Some("eDP-1"))],
        });
        state.apply(Event::WindowsReplaced {
            windows: vec![window(10, Some(1)), window(20, Some(2))],
        });
        let change = state.apply(Event::FocusChanged {
            focus: FocusState {
                target: Some(FocusTarget::Window(WindowId(20))),
                output: Some(OutputId::from("eDP-1")),
                workspace: Some(WorkspaceId(2)),
                window: Some(WindowId(20)),
            },
        });

        assert!(change.focus);
        assert!(!state.windows[&WindowId(10)].focused);
        assert!(state.windows[&WindowId(20)].focused);
        assert!(state.workspaces[&WorkspaceId(2)].focused);
        assert!(state.validate().is_empty());
    }

    #[test]
    fn workspace_replacement_tolerates_temporarily_dangling_windows() {
        let mut state = State::default();
        state.apply(Event::WindowsReplaced {
            windows: vec![window(10, Some(1))],
        });
        state.apply(Event::WorkspacesReplaced { workspaces: vec![] });

        assert_eq!(state.windows[&WindowId(10)].workspace, Some(WorkspaceId(1)));
        assert!(state.validate().is_empty());
    }

    #[test]
    fn removing_window_clears_focus_and_active_window_references() {
        let mut ws = workspace(1, Some("eDP-1"));
        ws.active_window = Some(WindowId(10));
        let mut state = State::default();
        state.apply(Event::WorkspacesReplaced {
            workspaces: vec![ws],
        });
        state.apply(Event::WindowsReplaced {
            windows: vec![window(10, Some(1))],
        });
        state.apply(Event::FocusChanged {
            focus: FocusState {
                target: Some(FocusTarget::Window(WindowId(10))),
                output: Some(OutputId::from("eDP-1")),
                workspace: Some(WorkspaceId(1)),
                window: Some(WindowId(10)),
            },
        });
        let change = state.apply(Event::WindowRemoved { id: WindowId(10) });

        assert!(change.topology && change.focus);
        assert_eq!(state.focus.window, None);
        assert_eq!(state.workspaces[&WorkspaceId(1)].active_window, None);
    }

    #[test]
    fn urgency_updates_are_idempotent() {
        let mut state = State::default();
        state.apply(Event::WindowsReplaced {
            windows: vec![window(10, None)],
        });
        let first = state.apply(Event::UrgencyChanged {
            target: UrgencyTarget::Window(WindowId(10)),
            urgent: true,
        });
        let second = state.apply(Event::UrgencyChanged {
            target: UrgencyTarget::Window(WindowId(10)),
            urgent: true,
        });

        assert!(first.urgency);
        assert_eq!(second, Change::default());
    }

    #[test]
    fn unknown_events_are_counted_without_visible_change() {
        let mut state = State::default();
        let change = state.apply(Event::Unknown {
            source_kind: "FutureNiriEvent".into(),
            payload: serde_json::json!({"new": true}),
        });
        assert_eq!(change, Change::default());
        assert_eq!(state.unknown_events_seen, 1);
    }

    #[test]
    fn invalid_output_values_are_reported_without_panicking() {
        let output = Output {
            id: OutputId::from("DP-1"),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: vec![],
            current_mode: Some(4),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(LogicalOutput {
                position: LogicalPoint::default(),
                size: LogicalSize {
                    width: 100.0,
                    height: 100.0,
                },
                scale: 0.0,
                transform: "normal".into(),
            }),
        };
        let mut state = State::default();
        state.apply(Event::OutputsReplaced {
            outputs: vec![output],
        });

        assert_eq!(state.validate().len(), 2);
    }
}
