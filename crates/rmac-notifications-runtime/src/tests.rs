use super::*;
use rmac_compositor::{
    FocusState, LogicalOutput, LogicalPoint, LogicalSize, Output, OutputId as CompositorOutputId,
    OutputMode, PhysicalSize,
};
use rmac_notifications::banner::{Config, PlacementPolicy};
use rmac_notifications::protocol::{self, PortalInput};
use rmac_notifications::{DeliveryPolicy, Notification, PostOutcome, Server, Time, TimeoutPolicy};

fn output(name: &str) -> Output {
    Output {
        id: CompositorOutputId(name.into()),
        make: "private".into(),
        model: "private".into(),
        serial: Some("private".into()),
        physical_size_mm: None,
        modes: vec![OutputMode {
            physical_size: PhysicalSize {
                width: 1920,
                height: 1080,
            },
            refresh_millihz: 60_000,
            preferred: true,
        }],
        current_mode: Some(0),
        custom_mode: false,
        vrr_supported: false,
        vrr_enabled: false,
        logical: Some(LogicalOutput {
            position: LogicalPoint { x: 0.0, y: 0.0 },
            size: LogicalSize {
                width: 1920.0,
                height: 1080.0,
            },
            scale: 1.0,
            transform: "normal".into(),
        }),
    }
}

fn posted(timeout: rmac_notifications::Timeout) -> (PostOutcome, Vec<Notification>) {
    let request = protocol::portal(PortalInput {
        app_id: "org.example.App".into(),
        id: "one".into(),
        title: Some("Private".into()),
        ..PortalInput::default()
    })
    .unwrap();
    let mut request = request;
    request.timeout = timeout;
    let mut server = Server::new(10, TimeoutPolicy::default());
    let outcome = server
        .post(request, Time(100), DeliveryPolicy::default())
        .unwrap();
    (outcome, server.active().cloned().collect())
}

fn coordinator(motion: rmac_appearance::MotionPreference) -> Coordinator {
    let appearance = rmac_appearance::Snapshot {
        motion,
        ..rmac_appearance::Snapshot::default()
    };
    Coordinator::new(
        Config::default(),
        PlacementPolicy::ActiveOutput,
        &appearance,
    )
    .unwrap()
}

#[test]
fn post_uses_focused_connected_output_and_authoritative_timeout_duration() {
    let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
    let compositor = rmac_compositor::Snapshot {
        outputs: vec![output("eDP-private"), output("HDMI-private")],
        focus: FocusState {
            output: Some(CompositorOutputId("HDMI-private".into())),
            ..FocusState::default()
        },
        ..rmac_compositor::Snapshot::default()
    };
    coordinator.apply_compositor(&compositor, Time(0)).unwrap();
    let (outcome, notifications) = posted(rmac_notifications::Timeout::Milliseconds(2_000));
    let update = coordinator
        .apply_post(outcome, &notifications, Time(500))
        .unwrap();
    assert_eq!(update.commands, vec![Command::Redraw]);
    assert_eq!(
        coordinator.snapshot().banners[0].output.as_str(),
        "HDMI-private"
    );
    assert_eq!(update.schedule.wake_at, Some(Time(2_500)));
}

#[test]
fn paused_banner_expires_through_one_specific_service_command() {
    let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
    coordinator
        .apply_compositor(
            &rmac_compositor::Snapshot {
                outputs: vec![output("one")],
                ..Default::default()
            },
            Time(0),
        )
        .unwrap();
    let (outcome, notifications) = posted(rmac_notifications::Timeout::Milliseconds(1_000));
    let id = outcome.id;
    coordinator
        .apply_post(outcome, &notifications, Time(0))
        .unwrap();
    coordinator.set_hovered(id, true, Time(400)).unwrap();
    assert!(coordinator.advance(Time(2_000)).commands.is_empty());
    coordinator.set_hovered(id, false, Time(2_000)).unwrap();
    let update = coordinator.advance(Time(2_600));
    assert_eq!(update.commands, vec![Command::Redraw, Command::Expire(id)]);
}

#[test]
fn hotplug_moves_banner_without_copying_or_closing_notification() {
    let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
    coordinator
        .apply_compositor(
            &rmac_compositor::Snapshot {
                outputs: vec![output("gone")],
                ..Default::default()
            },
            Time(0),
        )
        .unwrap();
    let (outcome, notifications) = posted(rmac_notifications::Timeout::Never);
    coordinator
        .apply_post(outcome, &notifications, Time(0))
        .unwrap();
    let update = coordinator
        .apply_compositor(
            &rmac_compositor::Snapshot {
                outputs: vec![output("fallback")],
                ..Default::default()
            },
            Time(1),
        )
        .unwrap();
    assert_eq!(update.commands, vec![Command::Redraw]);
    assert_eq!(
        coordinator.snapshot().banners[0].output.as_str(),
        "fallback"
    );
}

#[test]
fn reduced_motion_change_completes_exit_and_preserves_focus_commands() {
    let mut coordinator = coordinator(rmac_appearance::MotionPreference::Full);
    coordinator
        .apply_compositor(
            &rmac_compositor::Snapshot {
                outputs: vec![output("one")],
                ..Default::default()
            },
            Time(0),
        )
        .unwrap();
    let (outcome, notifications) = posted(rmac_notifications::Timeout::Never);
    let id = outcome.id;
    coordinator
        .apply_post(outcome, &notifications, Time(0))
        .unwrap();
    coordinator.set_focus(Some(id), Time(1)).unwrap();
    coordinator.request_dismiss(id, Time(2)).unwrap();
    let reduced = rmac_appearance::Snapshot {
        motion: rmac_appearance::MotionPreference::Reduced,
        ..Default::default()
    };
    let update = coordinator.apply_appearance(&reduced, Time(3));
    assert_eq!(
        update.commands,
        vec![
            Command::Redraw,
            Command::Dismiss(id),
            Command::RestorePreviousFocus
        ]
    );
}

#[test]
fn unavailable_topology_keeps_last_known_good_output() {
    let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
    coordinator
        .apply_compositor(
            &rmac_compositor::Snapshot {
                outputs: vec![output("known")],
                ..Default::default()
            },
            Time(0),
        )
        .unwrap();
    let empty = coordinator
        .apply_compositor(&rmac_compositor::Snapshot::default(), Time(1))
        .unwrap();
    assert!(empty.commands.is_empty());
    let (outcome, notifications) = posted(rmac_notifications::Timeout::Never);
    coordinator
        .apply_post(outcome, &notifications, Time(2))
        .unwrap();
    assert_eq!(coordinator.snapshot().banners[0].output.as_str(), "known");
}

#[test]
fn authoritative_close_disarms_a_pending_dismiss_command() {
    let mut coordinator = coordinator(rmac_appearance::MotionPreference::Full);
    coordinator
        .apply_compositor(
            &rmac_compositor::Snapshot {
                outputs: vec![output("one")],
                ..Default::default()
            },
            Time(0),
        )
        .unwrap();
    let (outcome, notifications) = posted(rmac_notifications::Timeout::Never);
    let id = outcome.id;
    coordinator
        .apply_post(outcome, &notifications, Time(0))
        .unwrap();
    coordinator.advance(Time(220));
    let dismissing = coordinator.request_dismiss(id, Time(300)).unwrap();
    assert_eq!(dismissing.schedule.wake_at, Some(Time(480)));

    let reconciled = coordinator.apply_closed(id, Time(350));
    assert!(reconciled.commands.is_empty());
    assert_eq!(reconciled.schedule.wake_at, Some(Time(480)));
    assert_eq!(
        coordinator.advance(Time(480)).commands,
        vec![Command::Redraw]
    );
    assert!(coordinator.apply_closed(id, Time(500)).commands.is_empty());
}

#[test]
fn policy_suppressed_replacement_removes_an_existing_visual() {
    let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
    coordinator
        .apply_compositor(
            &rmac_compositor::Snapshot {
                outputs: vec![output("one")],
                ..Default::default()
            },
            Time(0),
        )
        .unwrap();
    let (mut outcome, notifications) = posted(rmac_notifications::Timeout::Never);
    coordinator
        .apply_post(outcome, &notifications, Time(0))
        .unwrap();
    assert_eq!(coordinator.snapshot().banners.len(), 1);

    outcome.delivery.banner = false;
    let update = coordinator
        .apply_post(outcome, &notifications, Time(1))
        .unwrap();
    assert_eq!(update.commands, vec![Command::Redraw]);
    assert!(coordinator.snapshot().banners.is_empty());
}
