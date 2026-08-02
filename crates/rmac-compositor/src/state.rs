use std::collections::BTreeMap;

use crate::{
    Activation, ConnectionState, Event, FocusState, FocusTarget, LayerSurface, LayerSurfaceId,
    Output, OutputId, Snapshot, UrgencyTarget, Window, WindowId, Workspace, WorkspaceId,
};

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
    pub overview_visible: bool,
    pub connection: ConnectionState,
    pub unknown_events_seen: u64,
}

impl State {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            outputs: self.outputs.values().cloned().collect(),
            workspaces: self.workspaces.values().cloned().collect(),
            windows: self.windows.values().cloned().collect(),
            layer_surfaces: self.layer_surfaces.values().cloned().collect(),
            focus: self.focus.clone(),
            activation: self.activation.clone(),
            overview_visible: self.overview_visible,
        }
    }

    pub fn apply(&mut self, event: Event) -> Change {
        match event {
            Event::Snapshot { snapshot } => {
                self.outputs = keyed(snapshot.outputs, |output| output.id.clone());
                self.workspaces = keyed(snapshot.workspaces, |workspace| workspace.id);
                self.windows = keyed(snapshot.windows, |window| window.id);
                self.layer_surfaces = keyed(snapshot.layer_surfaces, |surface| surface.id.clone());
                self.focus = snapshot.focus;
                self.activation = snapshot.activation;
                self.overview_visible = snapshot.overview_visible;
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
            Event::OverviewChanged { visible } => {
                let changed = replace_bool(&mut self.overview_visible, visible);
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
