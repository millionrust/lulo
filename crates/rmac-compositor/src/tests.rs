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
fn process_lookup_prefers_the_matching_app_id() {
    let desktop = WorkspaceId(1);
    let mut first = app_window(1, "org.rmac.Notes", desktop);
    first.pid = Some(4242);
    let mut second = app_window(2, "org.rmac.Other", desktop);
    second.pid = Some(4242);
    let snapshot = Snapshot {
        workspaces: vec![workspace(1, Some("DP-1"))],
        windows: vec![first, second],
        ..Default::default()
    };
    assert_eq!(
        window_id_for_process(&snapshot, 4242, Some("org.rmac.Other")),
        Some(WindowId(2))
    );
    assert_eq!(
        window_id_for_process(&snapshot, 4242, None),
        Some(WindowId(1))
    );
    assert_eq!(window_id_for_process(&snapshot, 9999, None), None);
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

#[test]
fn hide_others_skips_the_app_and_already_parked_windows() {
    let desktop = WorkspaceId(1);
    let parking = Workspace {
        name: Some(crate::actions::PARKING_WORKSPACE.to_string()),
        ..workspace(9, None)
    };
    let snapshot = Snapshot {
        workspaces: vec![workspace(1, Some("DP-1")), parking],
        windows: vec![
            app_window(1, "org.rmac.Files", desktop),
            app_window(2, "org.rmac.Notes", desktop),
            app_window(3, "org.rmac.Notes", WorkspaceId(9)),
            app_window(4, "org.mozilla.firefox", desktop),
        ],
        ..Default::default()
    };

    assert_eq!(
        visible_windows_except(&snapshot, "org.rmac.Files"),
        vec![WindowId(2), WindowId(4)]
    );
}

#[test]
fn quitting_an_app_closes_its_hidden_windows_too() {
    let desktop = WorkspaceId(1);
    let parking = Workspace {
        name: Some(crate::actions::PARKING_WORKSPACE.to_string()),
        ..workspace(9, None)
    };
    let snapshot = Snapshot {
        workspaces: vec![workspace(1, Some("DP-1")), parking],
        windows: vec![
            app_window(1, "org.rmac.Notes", WorkspaceId(9)),
            app_window(2, "org.rmac.Notes", desktop),
            app_window(3, "org.rmac.Files", desktop),
        ],
        ..Default::default()
    };

    assert_eq!(
        windows_of_application(&snapshot, "org.rmac.Notes"),
        vec![WindowId(1), WindowId(2)]
    );
    assert_eq!(
        application_windows(&snapshot, "org.rmac.Notes"),
        vec![WindowId(2)]
    );
}

#[test]
fn parked_entries_carry_the_metadata_a_minimized_tile_needs() {
    let snapshot = parking_snapshot();
    let mut store = ParkingStore::new();
    store.record_from(&snapshot, &[WindowId(1)]);

    let entry = store.entry(WindowId(1)).expect("entry recorded");
    assert_eq!(entry.app_id.as_deref(), Some("org.rmac.Test"));
    assert_eq!(entry.title.as_deref(), Some("Window 1"));
    assert_eq!(entry.thumbnail, None);

    let thumbnail = std::path::PathBuf::from("/run/user/1000/rmac/thumbnails/1.png");
    assert!(store.set_thumbnail(WindowId(1), thumbnail.clone()));
    assert_eq!(
        store
            .entry(WindowId(1))
            .and_then(|entry| entry.thumbnail.clone()),
        Some(thumbnail)
    );
    // A window that is not parked cannot receive a thumbnail.
    assert!(!store.set_thumbnail(WindowId(42), std::path::PathBuf::from("/tmp/x.png")));
}

#[test]
fn thumbnails_are_cached_beside_the_store() {
    assert_eq!(
        ParkingStore::thumbnail_path_beside(
            std::path::Path::new("/run/user/1000/rmac/parking.json"),
            WindowId(7)
        ),
        std::path::PathBuf::from("/run/user/1000/rmac/thumbnails/7.png")
    );
}

#[test]
fn window_logical_rect_places_a_window_on_its_output() {
    let mut snapshot = Snapshot {
        outputs: vec![Output {
            id: OutputId::from("eDP-1"),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: vec![],
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(LogicalOutput {
                position: LogicalPoint { x: 0.0, y: 0.0 },
                size: LogicalSize {
                    width: 1536.0,
                    height: 864.0,
                },
                scale: 1.25,
                transform: "normal".into(),
            }),
        }],
        workspaces: vec![workspace(1, Some("eDP-1"))],
        ..Default::default()
    };
    let mut placed = window(1, Some(1));
    placed.layout = WindowLayout {
        tile_position_in_view: Some(LogicalPoint { x: 218.4, y: 84.8 }),
        tile_size: LogicalSize {
            width: 1100.0,
            height: 720.0,
        },
        ..Default::default()
    };
    snapshot.windows = vec![placed];

    assert_eq!(
        window_logical_rect(&snapshot, WindowId(1)),
        Some(LogicalRect {
            x: 218.4,
            y: 84.8,
            width: 1100.0,
            height: 720.0,
        })
    );
    // A window with no known placement cannot be captured.
    snapshot.windows = vec![window(2, Some(1))];
    assert_eq!(window_logical_rect(&snapshot, WindowId(2)), None);
}

fn parking_snapshot() -> Snapshot {
    Snapshot {
        workspaces: vec![
            workspace(1, Some("eDP-1")),
            workspace(2, Some("eDP-1")),
            Workspace {
                name: Some(PARKING_WORKSPACE.to_string()),
                ..workspace(9, None)
            },
        ],
        windows: vec![window(1, Some(1)), window(2, Some(2)), window(3, Some(9))],
        ..Default::default()
    }
}

#[test]
fn parking_store_records_and_forgets_origin_snapshots() {
    let mut store = ParkingStore::new();
    let snapshot = parking_snapshot();

    store.record_from(&snapshot, &[WindowId(1), WindowId(3)]);
    assert_eq!(store.origin(WindowId(1)), Some(WorkspaceId(1)));
    // A window already on the parking workspace has no origin to remember.
    assert_eq!(store.origin(WindowId(3)), None);

    // A later park of the same window keeps the first origin.
    store.record(WindowId(1), WorkspaceId(2));
    assert_eq!(store.origin(WindowId(1)), Some(WorkspaceId(1)));

    assert_eq!(
        store.restore_actions(&[WindowId(1), WindowId(2)]),
        vec![Action::RestoreWindow {
            window: WindowId(1),
            workspace: WorkspaceId(1),
        }]
    );
    assert_eq!(store.origin(WindowId(1)), None);
    assert!(store.is_empty());
}

#[test]
fn parking_store_prunes_windows_the_compositor_no_longer_parks() {
    let mut store = ParkingStore::new();
    store.record(WindowId(1), WorkspaceId(1));
    store.record(WindowId(99), WorkspaceId(2));

    // Window 1 is parked, window 99 is gone, and window 2 is visible again:
    // only the parked entry survives.
    let snapshot = Snapshot {
        workspaces: vec![
            workspace(1, Some("eDP-1")),
            Workspace {
                name: Some(PARKING_WORKSPACE.to_string()),
                ..workspace(9, None)
            },
        ],
        windows: vec![window(1, Some(9)), window(2, Some(1))],
        ..Default::default()
    };
    store.prune(&snapshot);

    assert_eq!(store.entries().len(), 1);
    assert_eq!(store.origin(WindowId(1)), Some(WorkspaceId(1)));
    assert_eq!(store.origin(WindowId(99)), None);
}

#[test]
fn parking_store_round_trips_through_disk() {
    let path = std::env::temp_dir().join(format!(
        "rmac-parking-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut store = ParkingStore::new();
    store.record(WindowId(7), WorkspaceId(3));
    store.save(&path).unwrap();

    let loaded = ParkingStore::load(&path);
    assert_eq!(loaded, store);

    let _ = std::fs::remove_file(path);
}
