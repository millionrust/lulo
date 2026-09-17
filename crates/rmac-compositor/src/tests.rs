use super::*;
use crate::actions::MAX_SPAWN_ARGUMENTS;

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
fn overview_updates_are_idempotent_and_survive_snapshot_projection() {
    let mut state = State::default();
    let opened = state.apply(Event::OverviewChanged { visible: true });
    let duplicate = state.apply(Event::OverviewChanged { visible: true });

    assert!(opened.visible);
    assert_eq!(duplicate, Change::default());
    assert!(state.snapshot().overview_visible);
}

#[test]
fn actions_report_stable_kinds_and_capabilities() {
    let action = Action::MoveWindowToWorkspace {
        window: WindowId(7),
        workspace: WorkspaceId(3),
        follow: false,
    };
    let capabilities = ActionCapabilities {
        supported: vec![ActionKind::MoveWindowToWorkspace],
    };

    assert_eq!(action.kind(), ActionKind::MoveWindowToWorkspace);
    assert!(capabilities.supports(action.kind()));
    assert!(!capabilities.supports(ActionKind::SetOverview));
}

fn app_window(id: u64, app_id: &str, workspace: WorkspaceId) -> Window {
    Window {
        id: WindowId(id),
        title: None,
        app_id: Some(app_id.to_string()),
        pid: None,
        workspace: Some(workspace),
        focused: false,
        floating: false,
        urgent: false,
        focus_timestamp: None,
        layout: WindowLayout::default(),
    }
}

#[test]
fn hide_and_show_desktop_expand_to_minimize_per_visible_window() {
    let desktop = WorkspaceId(1);
    let parking = Workspace {
        name: Some(crate::actions::PARKING_WORKSPACE.to_string()),
        ..workspace(9, None)
    };
    let snapshot = Snapshot {
        workspaces: vec![workspace(1, Some("DP-1")), parking],
        windows: vec![
            app_window(1, "org.rmac.Notes", desktop),
            app_window(2, "org.rmac.Notes", WorkspaceId(9)),
            app_window(3, "org.mozilla.firefox", desktop),
        ],
        ..Default::default()
    };

    // The parked Notes window is skipped; only the visible one is hidden.
    assert_eq!(
        hide_application(&snapshot, "org.rmac.Notes"),
        vec![Action::MinimizeWindow {
            window: WindowId(1)
        }]
    );
    assert_eq!(
        show_desktop(&snapshot),
        vec![
            Action::MinimizeWindow {
                window: WindowId(1)
            },
            Action::MinimizeWindow {
                window: WindowId(3)
            },
        ]
    );
    assert!(window_is_parked(&snapshot, &snapshot.windows[1]));
    assert_eq!(
        restore_all(&[(WindowId(1), desktop)]),
        vec![Action::RestoreWindow {
            window: WindowId(1),
            workspace: desktop,
        }]
    );
}

#[test]
fn spawn_commands_are_bounded_round_trippable_and_debug_redacted() {
    let command = SpawnCommand::new(vec![
        "demo".into(),
        "--open".into(),
        "/home/user/private.txt".into(),
    ])
    .unwrap();
    let debug = format!("{command:?}");
    assert!(debug.contains("argument_count: 3"));
    assert!(!debug.contains("private.txt"));
    let json = serde_json::to_string(&command).unwrap();
    assert_eq!(
        serde_json::from_str::<SpawnCommand>(&json).unwrap(),
        command
    );

    assert_eq!(SpawnCommand::new(Vec::new()), Err(SpawnCommandError::Empty));
    assert_eq!(
        SpawnCommand::new(vec![String::new()]),
        Err(SpawnCommandError::EmptyProgram)
    );
    assert_eq!(
        SpawnCommand::new(vec!["demo".into(), "bad\0argument".into()]),
        Err(SpawnCommandError::InteriorNul)
    );
    assert_eq!(
        SpawnCommand::new(vec!["demo".into(); MAX_SPAWN_ARGUMENTS + 1]),
        Err(SpawnCommandError::TooManyArguments)
    );
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
