use super::*;

fn output(id: &str, enabled: bool) -> rmac_compositor::Output {
    rmac_compositor::Output {
        id: id.into(),
        make: "private make".into(),
        model: "private model".into(),
        serial: Some("private serial".into()),
        physical_size_mm: None,
        modes: Vec::new(),
        current_mode: enabled.then_some(0),
        custom_mode: false,
        vrr_supported: false,
        vrr_enabled: false,
        logical: enabled.then_some(rmac_compositor::LogicalOutput {
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
        title: None,
        app_id: None,
        pid: None,
        workspace: None,
        focused,
        floating: false,
        urgent: false,
        focus_timestamp: None,
        layout: rmac_compositor::WindowLayout::default(),
    }
}

fn compositor() -> rmac_compositor::Snapshot {
    rmac_compositor::Snapshot {
        outputs: vec![output("private-output-27", true)],
        windows: vec![window(8, true)],
        focus: rmac_compositor::FocusState {
            output: Some("private-output-27".into()),
            window: Some(rmac_compositor::WindowId(8)),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn single_seat_shortcut_resolves_exact_context_privately() {
    let seats = SeatInventory::new(vec!["private-seat-19".into()]).unwrap();
    let invocation = global_shortcut(&compositor(), &seats).unwrap();
    assert_eq!(invocation.output().0, "private-output-27");
    assert_eq!(invocation.seat().as_str(), "private-seat-19");
    assert_eq!(
        invocation.restore_window(),
        Some(rmac_compositor::WindowId(8))
    );
    let diagnostics = format!("{seats:?} {invocation:?}");
    assert!(!diagnostics.contains("private-output-27"));
    assert!(!diagnostics.contains("private-seat-19"));
}

#[test]
fn shortcut_refuses_missing_or_ambiguous_seat() {
    assert_eq!(
        global_shortcut(&compositor(), &SeatInventory::default()),
        Err(ResolveError::NoSeat)
    );
    let seats = SeatInventory::new(vec!["seat-a".into(), "seat-b".into()]).unwrap();
    assert_eq!(
        global_shortcut(&compositor(), &seats),
        Err(ResolveError::AmbiguousSeat)
    );
}

#[test]
fn surface_control_revalidates_exact_seat_and_output() {
    let seats = SeatInventory::new(vec!["seat-a".into(), "seat-b".into()]).unwrap();
    let seat = SeatId::new("seat-b").unwrap();
    let invocation =
        surface_control(&"private-output-27".into(), &seat, &compositor(), &seats).unwrap();
    assert_eq!(invocation.seat(), &seat);
    assert_eq!(
        surface_control(
            &"private-output-27".into(),
            &SeatId::new("seat-c").unwrap(),
            &compositor(),
            &seats,
        ),
        Err(ResolveError::SeatUnavailable)
    );
}

#[test]
fn invalid_inventory_and_unavailable_focus_fail_closed() {
    assert_eq!(
        SeatInventory::new(vec!["seat-a".into(), "seat-a".into()]),
        Err(InventoryError::DuplicateSeat)
    );
    let mut snapshot = compositor();
    snapshot.outputs[0] = output("private-output-27", false);
    assert_eq!(
        global_shortcut(
            &snapshot,
            &SeatInventory::new(vec!["seat-a".into()]).unwrap()
        ),
        Err(ResolveError::OutputUnavailable)
    );
    snapshot.focus.output = None;
    assert_eq!(
        global_shortcut(
            &snapshot,
            &SeatInventory::new(vec!["seat-a".into()]).unwrap()
        ),
        Err(ResolveError::NoFocusedOutput)
    );
}

#[test]
fn shortcut_uses_only_enabled_output_when_focus_is_temporarily_absent() {
    let seats = SeatInventory::new(vec!["seat-a".into()]).unwrap();
    let mut snapshot = compositor();
    snapshot.focus.output = None;
    assert_eq!(
        global_shortcut(&snapshot, &seats).unwrap().output().0,
        "private-output-27"
    );

    snapshot.outputs.push(output("private-output-28", true));
    assert_eq!(
        global_shortcut(&snapshot, &seats),
        Err(ResolveError::NoFocusedOutput)
    );
}

#[test]
fn registry_publishes_only_complete_hotplug_snapshots() {
    let mut registry = SeatRegistry::default();
    assert_eq!(registry.require_complete().unwrap().unwrap().len(), 0);
    registry.add(8, REQUIRED_WL_SEAT_VERSION).unwrap();
    assert!(registry.snapshot().unwrap().is_empty());
    assert!(registry.name(8, "seat-a").unwrap().is_some());
    registry.add(9, REQUIRED_WL_SEAT_VERSION).unwrap();
    assert!(registry.name(8, "seat-a").unwrap().is_none());
    let two = registry.name(9, "seat-b").unwrap().unwrap();
    assert_eq!(two.len(), 2);
    let one = registry.remove(8).unwrap().unwrap();
    assert_eq!(one.len(), 1);
}

#[test]
fn registry_rejects_incomplete_duplicate_and_renamed_seats() {
    let mut registry = SeatRegistry::default();
    assert_eq!(
        registry.add(1, REQUIRED_WL_SEAT_VERSION - 1),
        Err(RegistryError::SeatVersion {
            advertised: REQUIRED_WL_SEAT_VERSION - 1,
            required: REQUIRED_WL_SEAT_VERSION,
        })
    );
    registry.add(1, REQUIRED_WL_SEAT_VERSION).unwrap();
    assert_eq!(
        registry.require_complete(),
        Err(RegistryError::IncompleteSeat)
    );
    registry.name(1, "seat-a").unwrap();
    assert_eq!(registry.name(1, "seat-b"), Err(RegistryError::SeatRenamed));
    registry.add(2, REQUIRED_WL_SEAT_VERSION).unwrap();
    assert_eq!(
        registry.name(2, "seat-a"),
        Err(RegistryError::Inventory(InventoryError::DuplicateSeat))
    );
}
