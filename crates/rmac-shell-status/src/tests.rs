use super::*;
use rmac_compositor::{
    FocusState, FocusTarget, LogicalPoint, LogicalSize, OutputId, PhysicalSize, Window, WindowId,
    WindowLayout, Workspace, WorkspaceId,
};

fn focused_fixture() -> rmac_compositor::Snapshot {
    rmac_compositor::Snapshot {
        workspaces: vec![Workspace {
            id: WorkspaceId(7),
            index: 2,
            name: Some("Build".into()),
            output: Some(OutputId("DP-1".into())),
            urgent: false,
            active: true,
            focused: true,
            active_window: Some(WindowId(9)),
        }],
        windows: vec![Window {
            id: WindowId(9),
            title: Some("rmac — terminal".into()),
            app_id: Some("dev.rmac.Terminal".into()),
            pid: Some(42),
            workspace: Some(WorkspaceId(7)),
            focused: true,
            floating: false,
            urgent: false,
            focus_timestamp: None,
            layout: WindowLayout {
                scrolling_position: None,
                tile_size: LogicalSize::default(),
                tile_position_in_view: None,
                window_size: PhysicalSize::default(),
                window_offset_in_tile: LogicalPoint::default(),
            },
        }],
        focus: FocusState {
            target: Some(FocusTarget::Window(WindowId(9))),
            output: Some(OutputId("DP-1".into())),
            workspace: Some(WorkspaceId(7)),
            window: Some(WindowId(9)),
        },
        ..Default::default()
    }
}

#[test]
fn focused_app_and_workspace_are_resolved_from_compositor_identity() {
    let mut state = State::default();
    let change = state.apply(Event::Compositor(rmac_compositor::Event::Snapshot {
        snapshot: focused_fixture(),
    }));
    assert!(change.visible);
    assert_eq!(
        state.snapshot().focused,
        FocusedContext {
            output: Some(OutputId("DP-1".into())),
            workspace_id: Some(WorkspaceId(7)),
            workspace_label: Some("Build".into()),
            window_id: Some(WindowId(9)),
            app_id: Some("dev.rmac.Terminal".into()),
            title: Some("rmac — terminal".into()),
        }
    );
}

#[test]
fn unknown_compositor_and_duplicate_hardware_events_do_not_redraw() {
    let mut state = State::default();
    assert!(
        !state
            .apply(Event::Compositor(rmac_compositor::Event::Unknown {
                source_kind: "future".into(),
                payload: Default::default(),
            }))
            .visible
    );
    assert!(
        !state
            .apply(Event::Audio(rmac_audio::Snapshot::default()))
            .visible
    );
}

#[test]
fn visibility_settings_remove_hidden_indicators_from_projection() {
    let mut state = State::default();
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.indicators.network = false;
    settings.indicators.sound = false;
    assert!(state.apply(Event::Settings(settings)).visible);
    let snapshot = state.snapshot();
    assert_eq!(snapshot.network, None);
    assert_eq!(snapshot.sound, None);
    assert!(snapshot.bluetooth.is_some());
    assert!(
        !state
            .apply(Event::Audio(rmac_audio::Snapshot {
                available: true,
                output: rmac_audio::Level {
                    volume: 80,
                    muted: false,
                },
                ..Default::default()
            }))
            .visible
    );
}

#[test]
fn connected_wifi_is_normalized_for_compact_consumers() {
    let mut state = State::default();
    state.apply(Event::Network(rmac_network::NetworkSnapshot {
        available: true,
        connectivity: rmac_network::Connectivity::Full,
        primary_connection: Some("Office Wi-Fi".into()),
        devices: vec![],
    }));
    state.apply(Event::Wifi(rmac_network::WifiSnapshot {
        available: true,
        enabled: true,
        interface: Some("wlan0".into()),
        current_ssid: Some("Office Wi-Fi".into()),
        networks: vec![rmac_network::WifiNetwork {
            id: rmac_network::WifiNetworkId::from_bytes(
                b"Office Wi-Fi".to_vec(),
                rmac_network::WifiSecurity::Personal(rmac_network::WifiPersonalMode::Psk),
            )
            .unwrap(),
            ssid: "Office Wi-Fi".into(),
            strength: 76,
            security: rmac_network::WifiSecurity::Personal(rmac_network::WifiPersonalMode::Psk),
            known: true,
            connected: true,
        }],
        saved_networks: Vec::new(),
    }));
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.network,
        Some(NetworkIndicator {
            state: NetworkState::Connected,
            connection_name: Some("Office Wi-Fi".into()),
            wifi_strength: Some(76),
        })
    );
    assert_eq!(snapshot.vpn.unwrap().active_names, Vec::<String>::new());
}

#[test]
fn notification_and_live_focus_state_are_gated_by_visibility_settings() {
    let mut state = State::default();
    assert!(
        state
            .apply(Event::Notifications(NotificationIndicator {
                unread_count: 3,
                has_urgent: true,
            }))
            .visible
    );
    state.apply(Event::Focus(Some(FocusIndicator {
        enabled: true,
        mode: Some("Work".into()),
        ends_at_unix_ms: Some(5000),
    })));
    assert_eq!(state.snapshot().notifications.unwrap().unread_count, 3);
    assert_eq!(
        state.snapshot().focus.unwrap().mode.as_deref(),
        Some("Work")
    );

    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.indicators.focus = false;
    state.apply(Event::Settings(settings));
    assert_eq!(state.snapshot().focus, None);
    state.apply(Event::Focus(Some(FocusIndicator {
        enabled: false,
        mode: None,
        ends_at_unix_ms: None,
    })));
    assert_eq!(state.snapshot().focus, None);
}

#[test]
fn output_projection_is_sorted_valid_and_hardware_private() {
    let output = |id: &str, scale: f64| rmac_compositor::Output {
        id: id.into(),
        make: "Private manufacturer".into(),
        model: "Private model".into(),
        serial: Some("PRIVATE-SERIAL".into()),
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
            scale,
            transform: "normal".into(),
        }),
    };
    let mut invalid = output("BAD", 0.0);
    invalid.logical.as_mut().unwrap().size.width = f64::NAN;
    let mut state = State::default();
    state.apply(Event::Compositor(rmac_compositor::Event::OutputsReplaced {
        outputs: vec![output("DP-2", 2.0), invalid, output("DP-1", 1.0)],
    }));

    let snapshot = state.snapshot();
    assert_eq!(
        snapshot
            .outputs
            .iter()
            .map(|output| output.id.0.as_str())
            .collect::<Vec<_>>(),
        ["DP-1", "DP-2"]
    );
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("PRIVATE-SERIAL"));
    assert!(!debug.contains("Private manufacturer"));
}

#[test]
fn clock_policy_changes_are_consumer_visible() {
    let mut state = State::default();
    let before = state.snapshot();
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.clock.show_seconds = true;
    settings.clock.format = rmac_shell_settings::ClockFormat::TwentyFourHour;
    assert!(state.apply(Event::Settings(settings)).visible);
    assert_ne!(state.snapshot().clock, before.clock);
}
