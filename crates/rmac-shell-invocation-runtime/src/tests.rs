use super::*;

fn output(id: &str) -> rmac_compositor::Output {
    rmac_compositor::Output {
        id: id.into(),
        make: "private make".into(),
        model: "private model".into(),
        serial: Some("private serial".into()),
        physical_size_mm: None,
        modes: Vec::new(),
        current_mode: Some(0),
        custom_mode: false,
        vrr_supported: false,
        vrr_enabled: false,
        logical: Some(rmac_compositor::LogicalOutput {
            position: rmac_compositor::LogicalPoint::default(),
            size: rmac_compositor::LogicalSize {
                width: 1920.0,
                height: 1080.0,
            },
            scale: 1.0,
            transform: "normal".into(),
        }),
    }
}

fn window(id: u64, focused: bool) -> rmac_compositor::Window {
    rmac_compositor::Window {
        id: rmac_compositor::WindowId(id),
        title: Some("private window title".into()),
        app_id: Some("private.app".into()),
        pid: None,
        workspace: None,
        focused,
        floating: false,
        urgent: false,
        focus_timestamp: None,
        layout: rmac_compositor::WindowLayout::default(),
    }
}

fn compositor(output_id: &str, window_id: u64) -> rmac_compositor::Snapshot {
    rmac_compositor::Snapshot {
        outputs: vec![output(output_id)],
        windows: vec![window(window_id, true)],
        focus: rmac_compositor::FocusState {
            output: Some(output_id.into()),
            window: Some(rmac_compositor::WindowId(window_id)),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn connect(coordinator: &mut Coordinator, output_id: &str, seat_ids: &[&str]) -> Snapshot {
    coordinator.apply_compositor(rmac_compositor::Event::Snapshot {
        snapshot: compositor(output_id, 9),
    });
    coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
        state: rmac_compositor::ConnectionState::Connected,
    });
    coordinator.apply_seats(SeatEvent::Snapshot(
        rmac_shell_invocation::SeatInventory::new(seat_ids.iter().map(|seat| (*seat).to_string()))
            .unwrap(),
    ));
    coordinator.snapshot()
}

#[test]
fn resolution_waits_for_both_live_sources() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_compositor(rmac_compositor::Event::Snapshot {
        snapshot: compositor("private-output", 9),
    });
    coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
        state: rmac_compositor::ConnectionState::Connected,
    });
    assert_eq!(
        coordinator.snapshot().global_shortcut(),
        Err(ResolveError::SeatsUnavailable)
    );
    let ready = connect(&mut coordinator, "private-output", &["private-seat"]);
    assert!(ready.ready());
    assert_eq!(
        ready.global_shortcut().unwrap().restore_window(),
        Some(rmac_compositor::WindowId(9))
    );
}

#[test]
fn source_loss_retains_last_known_good_but_fails_new_invocations() {
    let mut coordinator = Coordinator::default();
    let ready = connect(
        &mut coordinator,
        "old-private-output",
        &["old-private-seat"],
    );
    assert!(ready.global_shortcut().is_ok());

    coordinator.apply_seats(SeatEvent::Unavailable);
    let degraded = coordinator.snapshot();
    assert!(degraded.compositor().is_some());
    assert!(degraded.seats().is_some());
    assert_eq!(
        degraded.global_shortcut(),
        Err(ResolveError::SeatsUnavailable)
    );

    coordinator.apply_seats(SeatEvent::Snapshot(
        rmac_shell_invocation::SeatInventory::new(vec!["new-private-seat".into()]).unwrap(),
    ));
    coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
        state: rmac_compositor::ConnectionState::Reconnecting,
    });
    assert_eq!(
        coordinator.snapshot().global_shortcut(),
        Err(ResolveError::CompositorUnavailable)
    );

    let recovered = connect(
        &mut coordinator,
        "new-private-output",
        &["new-private-seat"],
    );
    assert_eq!(
        recovered.global_shortcut().unwrap().output().0,
        "new-private-output"
    );
}

#[test]
fn multi_seat_shortcuts_fail_while_exact_pointer_context_resolves() {
    let mut coordinator = Coordinator::default();
    let snapshot = connect(
        &mut coordinator,
        "private-output",
        &["private-seat-a", "private-seat-b"],
    );
    assert_eq!(
        snapshot.global_shortcut(),
        Err(ResolveError::Context(
            rmac_shell_invocation::ResolveError::AmbiguousSeat
        ))
    );
    let seat = rmac_shell_invocation::SeatId::new("private-seat-b").unwrap();
    assert_eq!(
        snapshot
            .surface_control(&"private-output".into(), &seat)
            .unwrap()
            .seat(),
        &seat
    );
}

#[test]
fn diagnostics_never_expose_output_seat_or_window_identity() {
    let mut coordinator = Coordinator::default();
    let snapshot = connect(
        &mut coordinator,
        "private-output-secret",
        &["private-seat-secret"],
    );
    let diagnostics = format!("{snapshot:?}");
    assert!(!diagnostics.contains("private-output-secret"));
    assert!(!diagnostics.contains("private-seat-secret"));
    assert!(!diagnostics.contains("private window title"));
    assert!(diagnostics.contains("outputs: 1"));
    assert!(diagnostics.contains("seats: 1"));
}

#[test]
fn unchanged_source_updates_do_not_change_the_snapshot() {
    let mut coordinator = Coordinator::default();
    connect(&mut coordinator, "private-output", &["private-seat"]);
    assert!(!coordinator.apply_seats(SeatEvent::Snapshot(
        rmac_shell_invocation::SeatInventory::new(vec!["private-seat".into()]).unwrap()
    )));
    assert!(
        !coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
            state: rmac_compositor::ConnectionState::Connected,
        })
    );
}
