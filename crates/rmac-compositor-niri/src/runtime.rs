//! Reconnecting niri event and action runtime.

use super::*;

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
            domain::ActionKind::FullscreenWindow,
            domain::ActionKind::FillWindow,
            domain::ActionKind::CenterWindow,
            domain::ActionKind::TileWindow,
            domain::ActionKind::MinimizeWindow,
            domain::ActionKind::RestoreWindow,
            domain::ActionKind::NameWorkspace,
            domain::ActionKind::UnnameWorkspace,
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
    for wire_action in convert_action_sequence(action)? {
        let request = Request::Action(wire_action);
        let reply = request_reply_once(path, &request)
            .await
            .map_err(action_error)?;
        match reply {
            Ok(Response::Handled) => {}
            Ok(_) => {
                return Err(domain::ActionError {
                    kind: domain::ActionErrorKind::Protocol,
                    message: "niri returned an unexpected action response".into(),
                })
            }
            Err(message) => {
                return Err(domain::ActionError {
                    kind: domain::ActionErrorKind::Rejected,
                    message,
                })
            }
        }
    }
    Ok(())
}

pub(super) fn convert_action(action: &domain::Action) -> wire::Action {
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
        domain::Action::FullscreenWindow { .. } => wire::Action::FullscreenWindow {},
        // Fill and tiling resize as well as move; `convert_action_sequence`
        // sends the whole frame. Alone, each maps to its final placement.
        domain::Action::FillWindow { window } => move_floating(*window, 0.0, 0.0),
        domain::Action::CenterWindow { .. } => wire::Action::CenterWindow {},
        domain::Action::TileWindow { window, region } => {
            let (x, y, _, _) = region.frame_percent();
            move_floating(*window, x, y)
        }
        domain::Action::MinimizeWindow { window } => wire::Action::MoveWindowToWorkspace {
            window_id: Some(window.0),
            reference: wire::WorkspaceReference::Name(domain::PARKING_WORKSPACE.into()),
            focus: false,
        },
        domain::Action::RestoreWindow { window, workspace } => {
            wire::Action::MoveWindowToWorkspace {
                window_id: Some(window.0),
                reference: wire::WorkspaceReference::Id(workspace.0),
                focus: true,
            }
        }
        domain::Action::NameWorkspace { workspace, name } => wire::Action::SetWorkspaceName {
            name: name.clone(),
            workspace: Some(wire::WorkspaceReference::Id(workspace.0)),
        },
        domain::Action::UnnameWorkspace { workspace } => wire::Action::UnsetWorkspaceName {
            reference: Some(wire::WorkspaceReference::Id(workspace.0)),
        },
    }
}

fn move_floating(window: domain::WindowId, x: f64, y: f64) -> wire::Action {
    wire::Action::MoveFloatingWindow {
        id: Some(window.0),
        x: wire::PositionChange::SetProportion(x),
        y: wire::PositionChange::SetProportion(y),
    }
}

/// Resize then move a floating window to a frame given in working-area
/// percentages, the way the green button's Move & Resize and Fill do.
fn floating_frame(
    window: domain::WindowId,
    (x, y, width, height): (f64, f64, f64, f64),
) -> Vec<wire::Action> {
    vec![
        wire::Action::SetWindowWidth {
            id: Some(window.0),
            change: wire::SizeChange::SetProportion(width),
        },
        wire::Action::SetWindowHeight {
            id: Some(window.0),
            change: wire::SizeChange::SetProportion(height),
        },
        move_floating(window, x, y),
    ]
}

/// niri's full-screen and centre actions apply to the focused window, so
/// those are a focus step followed by the action on its own socket. Fill and
/// tiling address the window by id and set its whole frame.
pub(super) fn convert_action_sequence(
    action: &domain::Action,
) -> Result<Vec<wire::Action>, domain::ActionError> {
    match action {
        domain::Action::FillWindow { window } => {
            Ok(floating_frame(*window, (0.0, 0.0, 100.0, 100.0)))
        }
        domain::Action::TileWindow { window, region } => {
            Ok(floating_frame(*window, region.frame_percent()))
        }
        domain::Action::FullscreenWindow { window, .. }
        | domain::Action::CenterWindow { window } => Ok(vec![
            wire::Action::FocusWindow { id: window.0 },
            convert_action(action),
        ]),
        _ => Ok(vec![convert_action(action)]),
    }
}

pub(super) fn action_error(error: Error) -> domain::ActionError {
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

/// Execute one action using the session's own niri socket.
pub async fn execute_action(action: &domain::Action) -> Result<(), domain::ActionError> {
    let path = env::var_os(SOCKET_PATH_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or(domain::ActionError {
            kind: domain::ActionErrorKind::Unavailable,
            message: format!("{SOCKET_PATH_ENV} is not set"),
        })?;
    execute_at(&path, action).await
}

/// Read exactly one coherent compositor snapshot without leaving a watcher
/// running. Hosts use this to resolve a window before sending an action.
pub async fn snapshot() -> Result<domain::Snapshot, Error> {
    let path = env::var_os(SOCKET_PATH_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or(Error::MissingSocketPath)?;
    snapshot_at(&path).await
}

pub async fn snapshot_at(path: &Path) -> Result<domain::Snapshot, Error> {
    let (sender, receiver) = async_channel::bounded(16);
    let stream = futures_util::FutureExt::boxed_local(stream_once(path, &sender));
    let reader = futures_util::FutureExt::boxed_local(async move {
        loop {
            match receiver.recv().await {
                Ok(domain::Event::Snapshot { snapshot }) => return Ok(snapshot),
                Ok(_) => continue,
                Err(_) => return Err(Error::ConsumerClosed),
            }
        }
    });
    futures_util::pin_mut!(stream, reader);
    match futures_util::future::select(stream, reader).await {
        futures_util::future::Either::Right((snapshot, _stream)) => snapshot,
        futures_util::future::Either::Left((stream_result, _reader)) => match stream_result {
            Ok(()) => Err(Error::InitialStateIncomplete),
            Err(error) => Err(error),
        },
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
        let refresh_outputs = decoded.source_kind == "WorkspacesChanged";
        for event in translate(decoded.event, decoded.source_kind, decoded.payload, &state) {
            state.apply(event.clone());
            send(sender, event).await?;
        }
        if refresh_outputs {
            let outputs = match request_once(path, &Request::Outputs).await? {
                Response::Outputs(outputs) => convert_outputs(outputs),
                _ => return Err(Error::UnexpectedResponse("outputs")),
            };
            let event = domain::Event::OutputsReplaced { outputs };
            state.apply(event.clone());
            send(sender, event).await?;
        }
    }
}
