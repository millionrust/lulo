//! Focused niri compositor adapter contracts.

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
fn targeted_window_actions_focus_then_act() {
    let cases = [
        (
            domain::Action::FullscreenWindow {
                window: domain::WindowId(7),
                on: true,
            },
            r#"{"FullscreenWindow":{}}"#,
        ),
        (
            domain::Action::FillWindow {
                window: domain::WindowId(7),
            },
            r#"{"ExpandColumnToAvailableWidth":{}}"#,
        ),
        (
            domain::Action::CenterWindow {
                window: domain::WindowId(7),
            },
            r#"{"CenterWindow":{}}"#,
        ),
    ];
    for (action, expected) in cases {
        let sequence = convert_action_sequence(&action).unwrap();
        assert_eq!(sequence.len(), 2);
        assert_eq!(
            serde_json::to_string(&sequence[0]).unwrap(),
            r#"{"FocusWindow":{"id":7}}"#
        );
        assert_eq!(serde_json::to_string(&sequence[1]).unwrap(), expected);
    }
    // Untargeted actions stay a single request.
    let sequence = convert_action_sequence(&domain::Action::SetOverview { visible: true }).unwrap();
    assert_eq!(sequence.len(), 1);
}

#[test]
fn minimize_and_restore_use_the_named_parking_workspace() {
    let minimized = convert_action_sequence(&domain::Action::MinimizeWindow {
        window: domain::WindowId(4),
    })
    .unwrap();
    assert_eq!(
        serde_json::to_string(&minimized[0]).unwrap(),
        r#"{"MoveWindowToWorkspace":{"window_id":4,"reference":{"Name":"rmac-parking"},"focus":false}}"#
    );
    let restored = convert_action_sequence(&domain::Action::RestoreWindow {
        window: domain::WindowId(4),
        workspace: domain::WorkspaceId(2),
    })
    .unwrap();
    assert_eq!(
        serde_json::to_string(&restored[0]).unwrap(),
        r#"{"MoveWindowToWorkspace":{"window_id":4,"reference":{"Id":2},"focus":true}}"#
    );
}

#[test]
fn unsupported_tile_regions_are_rejected_before_any_request() {
    let error = convert_action_sequence(&domain::Action::TileWindow {
        window: domain::WindowId(7),
        region: domain::TileRegion::TopLeft,
    })
    .unwrap_err();
    assert_eq!(error.kind, domain::ActionErrorKind::Unsupported);

    let left = convert_action_sequence(&domain::Action::TileWindow {
        window: domain::WindowId(7),
        region: domain::TileRegion::Left,
    })
    .unwrap();
    assert_eq!(left.len(), 2);
    assert_eq!(
        serde_json::to_string(&left[1]).unwrap(),
        r#"{"MoveColumnToFirst":{}}"#
    );
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
    assert!(events.iter().any(
        |event| matches!(event, domain::Event::OutputsReplaced { outputs } if outputs.is_empty())
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
    stream
        .write_all(b"{\"WorkspacesChanged\":{\"workspaces\":[]}}\n")
        .unwrap();

    let (mut outputs, _) = listener.accept().unwrap();
    assert_eq!(read_sync_line(&mut outputs), "\"Outputs\"");
    outputs.write_all(b"{\"Ok\":{\"Outputs\":{}}}\n").unwrap();
}

fn read_sync_line(stream: &mut UnixStream) -> String {
    let mut line = String::new();
    SyncBufReader::new(stream).read_line(&mut line).unwrap();
    line.trim().to_owned()
}
