//! Direct niri IPC adapter for [`rmac_compositor`].

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_channel::Sender;
use async_io::{Async, Timer};
use futures_util::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use rmac_compositor as domain;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wire::{Event as NiriEvent, Reply, Request, Response};

pub const SOCKET_PATH_ENV: &str = "NIRI_SOCKET";
const MAX_INITIAL_EVENTS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub maximum_delay: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            maximum_delay: Duration::from_secs(5),
        }
    }
}

impl ReconnectPolicy {
    fn next_delay(self, current: Duration) -> Duration {
        current.saturating_mul(2).min(self.maximum_delay)
    }
}

#[derive(Debug)]
pub enum Error {
    MissingSocketPath,
    Io(io::Error),
    Json(serde_json::Error),
    Protocol(String),
    UnexpectedResponse(&'static str),
    InitialStateIncomplete,
    ConsumerClosed,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSocketPath => write!(formatter, "{SOCKET_PATH_ENV} is not set"),
            Self::Io(error) => write!(formatter, "niri IPC I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "niri IPC JSON failed: {error}"),
            Self::Protocol(error) => write!(formatter, "niri IPC protocol failed: {error}"),
            Self::UnexpectedResponse(request) => {
                write!(formatter, "niri returned an unexpected {request} response")
            }
            Self::InitialStateIncomplete => {
                write!(
                    formatter,
                    "niri event stream omitted its initial workspace/window/overview state"
                )
            }
            Self::ConsumerClosed => write!(formatter, "compositor event consumer closed"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

/// Reconnecting event watcher. It returns only when the consumer closes.
pub async fn watch(sender: Sender<domain::Event>) -> Result<(), Error> {
    watch_with_policy(sender, ReconnectPolicy::default()).await
}

pub async fn watch_with_policy(
    sender: Sender<domain::Event>,
    policy: ReconnectPolicy,
) -> Result<(), Error> {
    let path = env::var_os(SOCKET_PATH_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or(Error::MissingSocketPath)?;
    let mut delay = policy.initial_delay;
    let mut first = true;

    loop {
        send(
            &sender,
            domain::Event::ConnectionChanged {
                state: if first {
                    domain::ConnectionState::Connecting
                } else {
                    domain::ConnectionState::Reconnecting
                },
            },
        )
        .await?;

        match stream_once(&path, &sender).await {
            Ok(()) | Err(Error::ConsumerClosed) => return Ok(()),
            Err(_) => {
                first = false;
                send(
                    &sender,
                    domain::Event::ConnectionChanged {
                        state: domain::ConnectionState::Disconnected,
                    },
                )
                .await?;
                Timer::after(delay).await;
                delay = policy.next_delay(delay);
            }
        }
    }
}

pub fn action_capabilities() -> domain::ActionCapabilities {
    domain::ActionCapabilities {
        supported: vec![
            domain::ActionKind::Spawn,
            domain::ActionKind::FocusWindow,
            domain::ActionKind::FocusWorkspace,
            domain::ActionKind::FocusOutput,
            domain::ActionKind::CloseWindow,
            domain::ActionKind::MoveWindowToWorkspace,
            domain::ActionKind::MoveWindowToOutput,
            domain::ActionKind::SetOverview,
        ],
    }
}

pub async fn execute(request: domain::ActionRequest) -> domain::ActionResult {
    let result = match env::var_os(SOCKET_PATH_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
    {
        Some(path) => execute_at(&path, &request.action).await,
        None => Err(domain::ActionError {
            kind: domain::ActionErrorKind::Unavailable,
            message: format!("{SOCKET_PATH_ENV} is not set"),
        }),
    };
    domain::ActionResult {
        id: request.id,
        action: request.action,
        result,
    }
}

/// Sends one action on its own socket so targets cannot race between requests.
pub async fn execute_at(path: &Path, action: &domain::Action) -> Result<(), domain::ActionError> {
    let request = Request::Action(convert_action(action));
    let reply = request_reply_once(path, &request)
        .await
        .map_err(action_error)?;
    match reply {
        Ok(Response::Handled) => Ok(()),
        Ok(_) => Err(domain::ActionError {
            kind: domain::ActionErrorKind::Protocol,
            message: "niri returned an unexpected action response".into(),
        }),
        Err(message) => Err(domain::ActionError {
            kind: domain::ActionErrorKind::Rejected,
            message,
        }),
    }
}

fn convert_action(action: &domain::Action) -> wire::Action {
    match action {
        domain::Action::Spawn { command } => wire::Action::Spawn {
            command: command.arguments().to_vec(),
        },
        domain::Action::FocusWindow { window } => wire::Action::FocusWindow { id: window.0 },
        domain::Action::FocusWorkspace { workspace } => wire::Action::FocusWorkspace {
            reference: wire::WorkspaceReference::Id(workspace.0),
        },
        domain::Action::FocusOutput { output } => wire::Action::FocusMonitor {
            output: output.0.clone(),
        },
        domain::Action::CloseWindow { window } => wire::Action::CloseWindow { id: Some(window.0) },
        domain::Action::MoveWindowToWorkspace {
            window,
            workspace,
            follow,
        } => wire::Action::MoveWindowToWorkspace {
            window_id: Some(window.0),
            reference: wire::WorkspaceReference::Id(workspace.0),
            focus: *follow,
        },
        domain::Action::MoveWindowToOutput { window, output } => {
            wire::Action::MoveWindowToMonitor {
                id: Some(window.0),
                output: output.0.clone(),
            }
        }
        domain::Action::SetOverview { visible: true } => wire::Action::OpenOverview {},
        domain::Action::SetOverview { visible: false } => wire::Action::CloseOverview {},
    }
}

fn action_error(error: Error) -> domain::ActionError {
    let kind = match &error {
        Error::MissingSocketPath => domain::ActionErrorKind::Unavailable,
        Error::Io(_) => domain::ActionErrorKind::Transport,
        Error::Json(_)
        | Error::Protocol(_)
        | Error::UnexpectedResponse(_)
        | Error::InitialStateIncomplete => domain::ActionErrorKind::Protocol,
        Error::ConsumerClosed => domain::ActionErrorKind::Unavailable,
    };
    domain::ActionError {
        kind,
        message: error.to_string(),
    }
}

/// One complete connection lifetime, exposed for deterministic socket tests.
pub async fn stream_once(path: &Path, sender: &Sender<domain::Event>) -> Result<(), Error> {
    let mut stream = connect(path)?;
    write_request(&mut stream, &Request::EventStream).await?;
    match read_reply(&mut stream).await? {
        Response::Handled => {}
        _ => return Err(Error::UnexpectedResponse("event-stream")),
    }

    // The stream is already live, so events accumulate in the socket buffer
    // while these non-streamed collections are queried on independent sockets.
    let outputs = match request_once(path, &Request::Outputs).await? {
        Response::Outputs(outputs) => convert_outputs(outputs),
        _ => return Err(Error::UnexpectedResponse("outputs")),
    };
    let layers = match request_once(path, &Request::Layers).await? {
        Response::Layers(layers) => convert_layers(layers),
        _ => return Err(Error::UnexpectedResponse("layers")),
    };

    let mut state = domain::State::default();
    state.apply(domain::Event::OutputsReplaced { outputs });
    state.apply(domain::Event::LayerSurfacesReplaced {
        layer_surfaces: layers,
    });
    let mut initial_workspaces = false;
    let mut initial_windows = false;
    let mut initial_overview = false;
    let mut ready = false;
    let mut pending_unknown = Vec::new();

    for _ in 0..MAX_INITIAL_EVENTS {
        let line = read_line(&mut stream).await?;
        let decoded = decode_event(&line)?;
        initial_workspaces |= decoded.initial_workspaces;
        initial_windows |= decoded.initial_windows;
        initial_overview |= decoded.initial_overview;
        for event in translate(decoded.event, decoded.source_kind, decoded.payload, &state) {
            state.apply(event.clone());
            if ready {
                send(sender, event).await?;
            } else if matches!(event, domain::Event::Unknown { .. }) {
                pending_unknown.push(event);
            }
        }

        if !ready && initial_workspaces && initial_windows && initial_overview {
            ready = true;
            send(
                sender,
                domain::Event::Snapshot {
                    snapshot: state.snapshot(),
                },
            )
            .await?;
            send(
                sender,
                domain::Event::ConnectionChanged {
                    state: domain::ConnectionState::Connected,
                },
            )
            .await?;
            for event in pending_unknown.drain(..) {
                send(sender, event).await?;
            }
            break;
        }
    }

    if !ready {
        return Err(Error::InitialStateIncomplete);
    }

    loop {
        let line = read_line(&mut stream).await?;
        let decoded = decode_event(&line)?;
        for event in translate(decoded.event, decoded.source_kind, decoded.payload, &state) {
            state.apply(event.clone());
            send(sender, event).await?;
        }
    }
}

async fn send(sender: &Sender<domain::Event>, event: domain::Event) -> Result<(), Error> {
    sender.send(event).await.map_err(|_| Error::ConsumerClosed)
}

type IpcStream = BufReader<Async<UnixStream>>;

fn connect(path: &Path) -> Result<IpcStream, Error> {
    let stream = UnixStream::connect(path)?;
    stream.set_nonblocking(true)?;
    Ok(BufReader::new(Async::new(stream)?))
}

async fn request_once(path: &Path, request: &Request) -> Result<Response, Error> {
    request_reply_once(path, request)
        .await?
        .map_err(Error::Protocol)
}

async fn request_reply_once(path: &Path, request: &Request) -> Result<Reply, Error> {
    let mut stream = connect(path)?;
    write_request(&mut stream, request).await?;
    let line = read_line(&mut stream).await?;
    Ok(serde_json::from_str(&line)?)
}

async fn write_request(stream: &mut IpcStream, request: &Request) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    stream.get_mut().write_all(&bytes).await?;
    stream.get_mut().flush().await?;
    Ok(())
}

async fn read_reply(stream: &mut IpcStream) -> Result<Response, Error> {
    let line = read_line(stream).await?;
    let reply: Reply = serde_json::from_str(&line)?;
    reply.map_err(Error::Protocol)
}

async fn read_line(stream: &mut IpcStream) -> Result<String, Error> {
    let mut line = String::new();
    let count = stream.read_line(&mut line).await?;
    if count == 0 {
        return Err(Error::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "niri IPC socket closed",
        )));
    }
    while matches!(line.as_bytes().last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Ok(line)
}

#[derive(Debug)]
struct DecodedEvent {
    event: Option<NiriEvent>,
    source_kind: String,
    payload: Value,
    initial_workspaces: bool,
    initial_windows: bool,
    initial_overview: bool,
}

fn decode_event(line: &str) -> Result<DecodedEvent, Error> {
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

fn is_known_event(kind: &str) -> bool {
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

fn translate(
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

fn focus_for_new_window(
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

fn focus_from_workspaces(
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

fn focus_from_windows(windows: &[domain::Window], state: &domain::State) -> domain::FocusState {
    let id = windows
        .iter()
        .find(|window| window.focused)
        .map(|window| window.id);
    focus_for_window_with(id, windows, &state.workspaces)
}

fn focus_for_window(id: Option<domain::WindowId>, state: &domain::State) -> domain::FocusState {
    let windows: Vec<_> = state.windows.values().cloned().collect();
    focus_for_window_with(id, &windows, &state.workspaces)
}

fn focus_for_window_with(
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

fn focus_for_workspace(
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

fn convert_outputs(outputs: HashMap<String, wire::Output>) -> Vec<domain::Output> {
    let mut outputs: Vec<_> = outputs.into_values().map(convert_output).collect();
    outputs.sort_by(|left, right| left.id.cmp(&right.id));
    outputs
}

fn convert_output(output: wire::Output) -> domain::Output {
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

fn convert_workspace(workspace: wire::Workspace) -> domain::Workspace {
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

fn convert_window(window: wire::Window) -> domain::Window {
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

fn convert_timestamp(timestamp: wire::Timestamp) -> domain::Timestamp {
    domain::Timestamp {
        seconds: timestamp.secs,
        nanoseconds: timestamp.nanos,
    }
}

fn convert_window_layout(layout: wire::WindowLayout) -> domain::WindowLayout {
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

fn convert_layers(layers: Vec<wire::LayerSurface>) -> Vec<domain::LayerSurface> {
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

mod wire {
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
    }

    #[derive(Debug, Serialize)]
    pub enum WorkspaceReference {
        Id(u64),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{BufRead, BufReader as SyncBufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    #[test]
    fn unknown_event_survives_without_typed_deserialization() {
        let decoded = decode_event(r#"{"FutureEvent":{"answer":42}}"#).unwrap();
        assert!(decoded.event.is_none());
        let events = translate(
            decoded.event,
            decoded.source_kind,
            decoded.payload,
            &domain::State::default(),
        );
        assert!(matches!(
            &events[0],
            domain::Event::Unknown { source_kind, payload }
                if source_kind == "FutureEvent" && payload["answer"] == 42
        ));
    }

    #[test]
    fn known_event_rejects_malformed_payload() {
        let error = decode_event(r#"{"WindowClosed":{"id":"wrong"}}"#).unwrap_err();
        assert!(matches!(error, Error::Json(_)));
    }

    #[test]
    fn overview_event_is_typed_and_marks_initial_authority_ready() {
        let decoded = decode_event(r#"{"OverviewOpenedOrClosed":{"is_open":true}}"#).unwrap();
        assert!(decoded.initial_overview);
        assert_eq!(
            translate(
                decoded.event,
                decoded.source_kind,
                decoded.payload,
                &domain::State::default(),
            ),
            [domain::Event::OverviewChanged { visible: true }]
        );
    }

    #[test]
    fn reconnect_delay_is_bounded() {
        let policy = ReconnectPolicy {
            initial_delay: Duration::from_millis(100),
            maximum_delay: Duration::from_millis(350),
        };
        assert_eq!(
            policy.next_delay(Duration::from_millis(100)),
            Duration::from_millis(200)
        );
        assert_eq!(
            policy.next_delay(Duration::from_millis(200)),
            Duration::from_millis(350)
        );
        assert_eq!(
            policy.next_delay(Duration::from_millis(350)),
            Duration::from_millis(350)
        );
    }

    #[test]
    fn action_wire_format_uses_stable_ids_and_explicit_targets() {
        let cases = [
            (
                domain::Action::Spawn {
                    command: domain::SpawnCommand::new(vec!["demo".into(), "--new-window".into()])
                        .unwrap(),
                },
                r#"{"Action":{"Spawn":{"command":["demo","--new-window"]}}}"#,
            ),
            (
                domain::Action::FocusWindow {
                    window: domain::WindowId(7),
                },
                r#"{"Action":{"FocusWindow":{"id":7}}}"#,
            ),
            (
                domain::Action::FocusWorkspace {
                    workspace: domain::WorkspaceId(3),
                },
                r#"{"Action":{"FocusWorkspace":{"reference":{"Id":3}}}}"#,
            ),
            (
                domain::Action::MoveWindowToOutput {
                    window: domain::WindowId(7),
                    output: domain::OutputId::from("DP-1"),
                },
                r#"{"Action":{"MoveWindowToMonitor":{"id":7,"output":"DP-1"}}}"#,
            ),
            (
                domain::Action::SetOverview { visible: true },
                r#"{"Action":{"OpenOverview":{}}}"#,
            ),
        ];

        for (action, expected) in cases {
            let request = Request::Action(convert_action(&action));
            assert_eq!(serde_json::to_string(&request).unwrap(), expected);
        }
    }

    #[test]
    fn action_results_distinguish_handled_rejected_and_transport_failures() {
        let socket = PathBuf::from(format!("/tmp/rmac-action-{}.sock", std::process::id()));
        let _ = fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut accepted, _) = listener.accept().unwrap();
            assert!(read_sync_line(&mut accepted).contains("CloseWindow"));
            accepted.write_all(b"{\"Ok\":\"Handled\"}\n").unwrap();

            let (mut rejected, _) = listener.accept().unwrap();
            assert!(read_sync_line(&mut rejected).contains("OpenOverview"));
            rejected
                .write_all(b"{\"Err\":\"disabled by policy\"}\n")
                .unwrap();
        });

        let accepted = async_io::block_on(execute_at(
            &socket,
            &domain::Action::CloseWindow {
                window: domain::WindowId(7),
            },
        ));
        assert_eq!(accepted, Ok(()));
        let rejected = async_io::block_on(execute_at(
            &socket,
            &domain::Action::SetOverview { visible: true },
        ));
        assert!(matches!(
            rejected,
            Err(domain::ActionError {
                kind: domain::ActionErrorKind::Rejected,
                message,
            }) if message == "disabled by policy"
        ));
        server.join().unwrap();
        let _ = fs::remove_file(&socket);

        let unavailable = async_io::block_on(execute_at(
            &socket,
            &domain::Action::FocusWindow {
                window: domain::WindowId(7),
            },
        ));
        assert!(matches!(
            unavailable,
            Err(domain::ActionError {
                kind: domain::ActionErrorKind::Transport,
                ..
            })
        ));
    }

    #[test]
    fn stream_emits_one_coherent_initial_snapshot() {
        // macOS limits Unix-domain socket paths to 103 bytes, and its
        // per-user temporary directory is already quite long.
        let socket = PathBuf::from(format!("/tmp/rmac-niri-{}.sock", std::process::id()));
        let _ = fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || serve_fixture(listener));
        let (sender, receiver) = async_channel::bounded(16);

        let result = async_io::block_on(stream_once(&socket, &sender));
        assert!(
            matches!(result, Err(Error::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof)
        );
        server.join().unwrap();
        let events: Vec<_> = std::iter::from_fn(|| receiver.try_recv().ok()).collect();
        assert!(matches!(
            events.first(),
            Some(domain::Event::Snapshot { snapshot }) if snapshot.overview_visible
        ));
        assert!(matches!(
            events.get(1),
            Some(domain::Event::ConnectionChanged {
                state: domain::ConnectionState::Connected
            })
        ));
        assert!(matches!(
            events.get(2),
            Some(domain::Event::Unknown { source_kind, payload })
                if source_kind == "FutureEvent" && payload["answer"] == 42
        ));
        assert!(matches!(
            events.get(3),
            Some(domain::Event::UrgencyChanged {
                target: domain::UrgencyTarget::Window(domain::WindowId(7)),
                urgent: true,
            })
        ));
        let _ = fs::remove_file(socket);
    }

    fn serve_fixture(listener: UnixListener) {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_sync_line(&mut stream);
        assert_eq!(request, "\"EventStream\"");
        stream.write_all(b"{\"Ok\":\"Handled\"}\n").unwrap();

        let (mut outputs, _) = listener.accept().unwrap();
        assert_eq!(read_sync_line(&mut outputs), "\"Outputs\"");
        outputs.write_all(b"{\"Ok\":{\"Outputs\":{}}}\n").unwrap();

        let (mut layers, _) = listener.accept().unwrap();
        assert_eq!(read_sync_line(&mut layers), "\"Layers\"");
        layers.write_all(b"{\"Ok\":{\"Layers\":[]}}\n").unwrap();

        stream
            .write_all(b"{\"FutureEvent\":{\"answer\":42}}\n")
            .unwrap();
        stream
            .write_all(b"{\"WorkspacesChanged\":{\"workspaces\":[]}}\n")
            .unwrap();
        stream
            .write_all(b"{\"WindowsChanged\":{\"windows\":[]}}\n")
            .unwrap();
        stream
            .write_all(b"{\"OverviewOpenedOrClosed\":{\"is_open\":true}}\n")
            .unwrap();
        stream
            .write_all(b"{\"WindowUrgencyChanged\":{\"id\":7,\"urgent\":true}}\n")
            .unwrap();
    }

    fn read_sync_line(stream: &mut UnixStream) -> String {
        let mut line = String::new();
        SyncBufReader::new(stream).read_line(&mut line).unwrap();
        line.trim().to_owned()
    }
}
