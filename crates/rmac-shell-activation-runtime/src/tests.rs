use super::*;

fn output(id: &str) -> rmac_compositor::Output {
    rmac_compositor::Output {
        id: id.into(),
        make: "private".into(),
        model: "private".into(),
        serial: Some("private".into()),
        physical_size_mm: None,
        modes: Vec::new(),
        current_mode: Some(0),
        custom_mode: false,
        vrr_supported: false,
        vrr_enabled: false,
        logical: Some(rmac_compositor::LogicalOutput {
            position: Default::default(),
            size: rmac_compositor::LogicalSize {
                width: 1920.0,
                height: 1080.0,
            },
            scale: 1.0,
            transform: "normal".into(),
        }),
    }
}

fn runtime(seats: &[&str]) -> rmac_shell_invocation_runtime::Snapshot {
    let mut coordinator = rmac_shell_invocation_runtime::Coordinator::default();
    coordinator.apply_compositor(rmac_compositor::Event::Snapshot {
        snapshot: rmac_compositor::Snapshot {
            outputs: vec![output("private-output")],
            focus: rmac_compositor::FocusState {
                output: Some("private-output".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    });
    coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
        state: rmac_compositor::ConnectionState::Connected,
    });
    coordinator.apply_seats(rmac_shell_invocation_runtime::SeatEvent::Snapshot(
        rmac_shell_invocation::SeatInventory::new(seats.iter().map(|seat| (*seat).to_owned()))
            .unwrap(),
    ));
    coordinator.snapshot()
}

fn activated() -> rmac_shortcuts::Event {
    rmac_shortcuts::Event::Activated {
        id: rmac_shortcuts::ShortcutId("launcher".into()),
        timestamp_ms: 1,
    }
}

#[test]
fn readiness_requires_endpoint_and_both_live_sources_once() {
    let mut coordinator = Coordinator::default();
    assert!(!coordinator.endpoint_ready());
    assert!(coordinator.apply_runtime(runtime(&["private-seat"])));
    assert!(!coordinator.apply_runtime(runtime(&["private-seat"])));
}

#[test]
fn activation_carries_one_exact_private_context() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_runtime(runtime(&["private-seat"]));
    let activation = coordinator.activate(activated());
    let context = activation.context().unwrap();
    assert_eq!(context.invocation().output().0, "private-output");
    let diagnostics = format!("{activation:?}");
    assert!(!diagnostics.contains("private-output"));
    assert!(!diagnostics.contains("private-seat"));
}

#[test]
fn exact_output_geometry_drives_top_right_and_centered_bounds() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_runtime(runtime(&["private-seat"]));
    let activation = coordinator.activate(activated());
    let context = activation.context().unwrap();
    assert_eq!(
        context.top_right_bounds(380.0, 548.0, 44.0, 12.0),
        Ok(LogicalBounds {
            x: 1528.0,
            y: 44.0,
            width: 380.0,
            height: 548.0,
        })
    );
    assert_eq!(
        context.centered_bounds(720.0, 540.0),
        Ok(LogicalBounds {
            x: 600.0,
            y: 270.0,
            width: 720.0,
            height: 540.0,
        })
    );
}

#[test]
fn degraded_and_multi_seat_activations_fail_explicitly() {
    let coordinator = Coordinator::default();
    assert!(matches!(
        coordinator.activate(activated()).context(),
        Err(ActivationError::Resolve(
            rmac_shell_invocation_runtime::ResolveError::CompositorUnavailable
        ))
    ));

    let mut coordinator = Coordinator::default();
    coordinator.apply_runtime(runtime(&["seat-a", "seat-b"]));
    assert!(matches!(
        coordinator.activate(activated()).context(),
        Err(ActivationError::Resolve(
            rmac_shell_invocation_runtime::ResolveError::Context(
                rmac_shell_invocation::ResolveError::AmbiguousSeat
            )
        ))
    ));
}

#[test]
fn one_shot_endpoint_readiness_stays_live_after_its_sender_closes() {
    async_io::block_on(async {
        let (updates_tx, updates_rx) = async_channel::bounded(4);
        let (shortcuts_tx, shortcuts_rx) = async_channel::bounded(1);
        let (endpoint_tx, endpoint_rx) = async_channel::bounded(1);
        let (runtime_tx, runtime_rx) = async_channel::bounded(1);
        let consumer = super::watch::consume(updates_tx, shortcuts_rx, endpoint_rx, runtime_rx);
        let scenario = async {
            endpoint_tx.send(()).await.unwrap();
            drop(endpoint_tx);
            runtime_tx.send(runtime(&["private-seat"])).await.unwrap();
            assert!(matches!(updates_rx.recv().await, Ok(Update::Ready)));

            shortcuts_tx.send(activated()).await.unwrap();
            assert!(matches!(updates_rx.recv().await, Ok(Update::Activated(_))));
            drop(updates_rx);
        };
        let (result, ()) = futures_util::join!(consumer, scenario);
        result.unwrap();
    });
}
