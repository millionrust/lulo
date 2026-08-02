//! Niri wire-event translation and domain conversion.

use super::*;

#[derive(Debug)]
pub(super) struct DecodedEvent {
    pub(super) event: Option<NiriEvent>,
    pub(super) source_kind: String,
    pub(super) payload: Value,
    pub(super) initial_workspaces: bool,
    pub(super) initial_windows: bool,
    pub(super) initial_overview: bool,
}

pub(super) fn decode_event(line: &str) -> Result<DecodedEvent, Error> {
    let value: Value = serde_json::from_str(line)?;
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or_else(|| Error::Protocol("event must be a single-key object".into()))?;
    let (kind, payload) = object.iter().next().expect("one key checked");
    let source_kind = kind.clone();
    let payload = payload.clone();
    let known = is_known_event(&source_kind);
    let event = if known {
        Some(serde_json::from_value(value)?)
    } else {
        None
    };
    Ok(DecodedEvent {
        event,
        source_kind: source_kind.clone(),
        payload,
        initial_workspaces: source_kind == "WorkspacesChanged",
        initial_windows: source_kind == "WindowsChanged",
        initial_overview: source_kind == "OverviewOpenedOrClosed",
    })
}

pub(super) fn is_known_event(kind: &str) -> bool {
    matches!(
        kind,
        "WorkspacesChanged"
            | "WorkspaceUrgencyChanged"
            | "WorkspaceActivated"
            | "WorkspaceActiveWindowChanged"
            | "WindowsChanged"
            | "WindowOpenedOrChanged"
            | "WindowClosed"
            | "WindowFocusChanged"
            | "WindowFocusTimestampChanged"
            | "WindowUrgencyChanged"
            | "WindowLayoutsChanged"
            | "OverviewOpenedOrClosed"
    )
}

pub(super) fn translate(
    event: Option<NiriEvent>,
    source_kind: String,
    payload: Value,
    state: &domain::State,
) -> Vec<domain::Event> {
    let Some(event) = event else {
        return vec![domain::Event::Unknown {
            source_kind,
            payload,
        }];
    };

    match event {
        NiriEvent::WorkspacesChanged { workspaces } => {
            let workspaces: Vec<_> = workspaces.into_iter().map(convert_workspace).collect();
            let focus = focus_from_workspaces(&workspaces, state);
            vec![
                domain::Event::WorkspacesReplaced { workspaces },
                domain::Event::FocusChanged { focus },
            ]
        }
        NiriEvent::WorkspaceUrgencyChanged { id, urgent } => {
            vec![domain::Event::UrgencyChanged {
                target: domain::UrgencyTarget::Workspace(domain::WorkspaceId(id)),
                urgent,
            }]
        }
        NiriEvent::WorkspaceActivated { id, focused } => {
            let id = domain::WorkspaceId(id);
            let mut workspaces: Vec<_> = state.workspaces.values().cloned().collect();
            let output = workspaces
                .iter()
                .find(|workspace| workspace.id == id)
                .and_then(|workspace| workspace.output.clone());
            for workspace in &mut workspaces {
                if workspace.output == output {
                    workspace.active = workspace.id == id;
                }
                if focused {
                    workspace.focused = workspace.id == id;
                }
            }
            let mut events = vec![domain::Event::WorkspacesReplaced { workspaces }];
            if focused {
                events.push(domain::Event::FocusChanged {
                    focus: focus_for_workspace(id, output, state),
                });
            }
            events
        }
        NiriEvent::WorkspaceActiveWindowChanged {
            workspace_id,
            active_window_id,
        } => state
            .workspaces
            .get(&domain::WorkspaceId(workspace_id))
            .cloned()
            .map(|mut workspace| {
                workspace.active_window = active_window_id.map(domain::WindowId);
                domain::Event::WorkspaceUpserted { workspace }
            })
            .into_iter()
            .collect(),
        NiriEvent::WindowsChanged { windows } => {
            let windows: Vec<_> = windows.into_iter().map(convert_window).collect();
            let focus = focus_from_windows(&windows, state);
            vec![
                domain::Event::WindowsReplaced { windows },
                domain::Event::FocusChanged { focus },
            ]
        }
        NiriEvent::WindowOpenedOrChanged { window } => {
            let window = convert_window(window);
            let focused = window.focused;
            let id = window.id;
            let workspace = window.workspace;
            let mut events = vec![domain::Event::WindowUpserted { window }];
            if focused {
                events.push(domain::Event::FocusChanged {
                    focus: focus_for_new_window(id, workspace, state),
                });
            }
            events
        }
        NiriEvent::WindowClosed { id } => vec![domain::Event::WindowRemoved {
            id: domain::WindowId(id),
        }],
        NiriEvent::WindowFocusChanged { id } => vec![domain::Event::FocusChanged {
            focus: focus_for_window(id.map(domain::WindowId), state),
        }],
        NiriEvent::WindowFocusTimestampChanged {
            id,
            focus_timestamp,
        } => state
            .windows
            .get(&domain::WindowId(id))
            .cloned()
            .map(|mut window| {
                window.focus_timestamp = focus_timestamp.map(convert_timestamp);
                domain::Event::WindowUpserted { window }
            })
            .into_iter()
            .collect(),
        NiriEvent::WindowUrgencyChanged { id, urgent } => {
            vec![domain::Event::UrgencyChanged {
                target: domain::UrgencyTarget::Window(domain::WindowId(id)),
                urgent,
            }]
        }
        NiriEvent::WindowLayoutsChanged { changes } => changes
            .into_iter()
            .filter_map(|(id, layout)| {
                state
                    .windows
                    .get(&domain::WindowId(id))
                    .cloned()
                    .map(|mut window| {
                        window.layout = convert_window_layout(layout);
                        domain::Event::WindowUpserted { window }
                    })
            })
            .collect(),
        NiriEvent::OverviewOpenedOrClosed { is_open } => {
            vec![domain::Event::OverviewChanged { visible: is_open }]
        }
    }
}

pub(super) fn focus_for_new_window(
    id: domain::WindowId,
    workspace: Option<domain::WorkspaceId>,
    state: &domain::State,
) -> domain::FocusState {
    let output = workspace.and_then(|id| state.workspaces.get(&id)?.output.clone());
    domain::FocusState {
        target: Some(domain::FocusTarget::Window(id)),
        output,
        workspace,
        window: Some(id),
    }
}

pub(super) fn focus_from_workspaces(
    workspaces: &[domain::Workspace],
    state: &domain::State,
) -> domain::FocusState {
    let Some(workspace) = workspaces.iter().find(|workspace| workspace.focused) else {
        return domain::FocusState {
            window: state.focus.window,
            ..domain::FocusState::default()
        };
    };
    domain::FocusState {
        target: Some(domain::FocusTarget::Workspace(workspace.id)),
        output: workspace.output.clone(),
        workspace: Some(workspace.id),
        window: state.focus.window,
    }
}

pub(super) fn focus_from_windows(
    windows: &[domain::Window],
    state: &domain::State,
) -> domain::FocusState {
    let id = windows
        .iter()
        .find(|window| window.focused)
        .map(|window| window.id);
    focus_for_window_with(id, windows, &state.workspaces)
}

pub(super) fn focus_for_window(
    id: Option<domain::WindowId>,
    state: &domain::State,
) -> domain::FocusState {
    let windows: Vec<_> = state.windows.values().cloned().collect();
    focus_for_window_with(id, &windows, &state.workspaces)
}

pub(super) fn focus_for_window_with(
    id: Option<domain::WindowId>,
    windows: &[domain::Window],
    workspaces: &BTreeMap<domain::WorkspaceId, domain::Workspace>,
) -> domain::FocusState {
    let workspace_id = id.and_then(|id| {
        windows
            .iter()
            .find(|window| window.id == id)
            .and_then(|window| window.workspace)
    });
    let output = workspace_id.and_then(|id| workspaces.get(&id)?.output.clone());
    domain::FocusState {
        target: id.map(domain::FocusTarget::Window),
        output,
        workspace: workspace_id,
        window: id,
    }
}

pub(super) fn focus_for_workspace(
    id: domain::WorkspaceId,
    output: Option<domain::OutputId>,
    state: &domain::State,
) -> domain::FocusState {
    let window = state
        .workspaces
        .get(&id)
        .and_then(|workspace| workspace.active_window);
    domain::FocusState {
        target: Some(domain::FocusTarget::Workspace(id)),
        output,
        workspace: Some(id),
        window,
    }
}

pub(super) fn convert_outputs(outputs: HashMap<String, wire::Output>) -> Vec<domain::Output> {
    let mut outputs: Vec<_> = outputs.into_values().map(convert_output).collect();
    outputs.sort_by(|left, right| left.id.cmp(&right.id));
    outputs
}

pub(super) fn convert_output(output: wire::Output) -> domain::Output {
    domain::Output {
        id: domain::OutputId(output.name),
        make: output.make,
        model: output.model,
        serial: output.serial,
        physical_size_mm: output
            .physical_size
            .map(|(width, height)| domain::PhysicalSize { width, height }),
        modes: output
            .modes
            .into_iter()
            .map(|mode| domain::OutputMode {
                physical_size: domain::PhysicalSize {
                    width: u32::from(mode.width),
                    height: u32::from(mode.height),
                },
                refresh_millihz: mode.refresh_rate,
                preferred: mode.is_preferred,
            })
            .collect(),
        current_mode: output.current_mode,
        custom_mode: output.is_custom_mode,
        vrr_supported: output.vrr_supported,
        vrr_enabled: output.vrr_enabled,
        logical: output.logical.map(|logical| domain::LogicalOutput {
            position: domain::LogicalPoint {
                x: f64::from(logical.x),
                y: f64::from(logical.y),
            },
            size: domain::LogicalSize {
                width: f64::from(logical.width),
                height: f64::from(logical.height),
            },
            scale: logical.scale,
            transform: logical.transform,
        }),
    }
}

pub(super) fn convert_workspace(workspace: wire::Workspace) -> domain::Workspace {
    domain::Workspace {
        id: domain::WorkspaceId(workspace.id),
        index: workspace.idx,
        name: workspace.name,
        output: workspace.output.map(domain::OutputId),
        urgent: workspace.is_urgent,
        active: workspace.is_active,
        focused: workspace.is_focused,
        active_window: workspace.active_window_id.map(domain::WindowId),
    }
}

pub(super) fn convert_window(window: wire::Window) -> domain::Window {
    domain::Window {
        id: domain::WindowId(window.id),
        title: window.title,
        app_id: window.app_id,
        pid: window.pid,
        workspace: window.workspace_id.map(domain::WorkspaceId),
        focused: window.is_focused,
        floating: window.is_floating,
        urgent: window.is_urgent,
        focus_timestamp: window.focus_timestamp.map(convert_timestamp),
        layout: convert_window_layout(window.layout),
    }
}

pub(super) fn convert_timestamp(timestamp: wire::Timestamp) -> domain::Timestamp {
    domain::Timestamp {
        seconds: timestamp.secs,
        nanoseconds: timestamp.nanos,
    }
}

pub(super) fn convert_window_layout(layout: wire::WindowLayout) -> domain::WindowLayout {
    domain::WindowLayout {
        scrolling_position: layout.pos_in_scrolling_layout,
        tile_size: domain::LogicalSize {
            width: layout.tile_size.0,
            height: layout.tile_size.1,
        },
        tile_position_in_view: layout
            .tile_pos_in_workspace_view
            .map(|(x, y)| domain::LogicalPoint { x, y }),
        window_size: domain::PhysicalSize {
            width: layout.window_size.0.max(0) as u32,
            height: layout.window_size.1.max(0) as u32,
        },
        window_offset_in_tile: domain::LogicalPoint {
            x: layout.window_offset_in_tile.0,
            y: layout.window_offset_in_tile.1,
        },
    }
}

pub(super) fn convert_layers(layers: Vec<wire::LayerSurface>) -> Vec<domain::LayerSurface> {
    let mut rows: Vec<_> = layers
        .into_iter()
        .map(|surface| {
            let layer = surface.layer;
            let keyboard = surface.keyboard_interactivity;
            (surface.output, surface.namespace, layer, keyboard)
        })
        .collect();
    rows.sort();
    let mut occurrences: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    rows.into_iter()
        .map(|(output, namespace, layer, keyboard)| {
            let key = (output.clone(), namespace.clone(), layer.clone());
            let occurrence = occurrences.entry(key).or_default();
            let id = domain::LayerSurfaceId(format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}",
                output, namespace, layer, *occurrence
            ));
            *occurrence += 1;
            domain::LayerSurface {
                id,
                namespace,
                output: domain::OutputId(output),
                layer: match layer.as_str() {
                    "Background" => domain::Layer::Background,
                    "Bottom" => domain::Layer::Bottom,
                    "Top" => domain::Layer::Top,
                    "Overlay" => domain::Layer::Overlay,
                    _ => domain::Layer::Other(layer),
                },
                keyboard_interactivity: match keyboard.as_str() {
                    "None" => domain::KeyboardInteractivity::None,
                    "Exclusive" => domain::KeyboardInteractivity::Exclusive,
                    "OnDemand" => domain::KeyboardInteractivity::OnDemand,
                    _ => domain::KeyboardInteractivity::Other(keyboard),
                },
            }
        })
        .collect()
}
