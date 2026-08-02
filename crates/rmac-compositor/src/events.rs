use serde::{Deserialize, Serialize};

use crate::{
    Activation, FocusState, LayerSurface, Output, Window, WindowId, Workspace, WorkspaceId,
};

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
    #[serde(default)]
    pub overview_visible: bool,
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
    OverviewChanged {
        visible: bool,
    },
    ConnectionChanged {
        state: ConnectionState,
    },
    Unknown {
        source_kind: String,
        payload: serde_json::Value,
    },
}
