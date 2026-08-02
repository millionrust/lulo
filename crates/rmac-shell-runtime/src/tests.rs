use super::*;
use rmac_shell_status_linux::Sources;

use crate::runtime::publication;

struct FakeReader {
    audio: Result<rmac_audio::Snapshot, String>,
    power: Result<rmac_power::Snapshot, String>,
}

impl Default for FakeReader {
    fn default() -> Self {
        Self {
            audio: Ok(rmac_audio::Snapshot::default()),
            power: Ok(rmac_power::Snapshot::default()),
        }
    }
}

impl ServiceReader for FakeReader {
    fn network(
        &self,
    ) -> Result<
        (
            rmac_network::NetworkSnapshot,
            rmac_network::WifiSnapshot,
            rmac_network::VpnSnapshot,
        ),
        String,
    > {
        Err("network unavailable".into())
    }

    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
        Err("Bluetooth unavailable".into())
    }

    fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
        self.audio.clone()
    }

    fn power(&self) -> Result<rmac_power::Snapshot, String> {
        self.power.clone()
    }
}

#[test]
fn service_failure_retains_last_known_good_indicator() {
    let mut coordinator = Coordinator::default();
    let working = FakeReader {
        audio: Ok(rmac_audio::Snapshot {
            available: true,
            output: rmac_audio::Level {
                volume: 67,
                muted: false,
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(coordinator.refresh_services(Sources::audio(), &working));
    let visible = coordinator.snapshot().status.sound;
    let failed = FakeReader {
        audio: Err("PipeWire restarted".into()),
        ..Default::default()
    };
    assert!(coordinator.refresh_services(Sources::audio(), &failed));
    assert_eq!(coordinator.snapshot().status.sound, visible);
    assert_eq!(
        coordinator.snapshot().quick_settings.audio.output.volume,
        67
    );
    assert!(!coordinator.snapshot().quick_settings.audio.available);
    assert_eq!(
        coordinator.snapshot().health.audio,
        SourceHealth::Unavailable {
            detail: "PipeWire restarted".into()
        }
    );
}

#[test]
fn duplicate_refresh_does_not_change_runtime_snapshot() {
    let mut coordinator = Coordinator::default();
    let reader = FakeReader {
        audio: Ok(rmac_audio::Snapshot::default()),
        ..Default::default()
    };
    assert!(coordinator.refresh_services(Sources::audio(), &reader));
    assert!(!coordinator.refresh_services(Sources::audio(), &reader));
}

#[test]
fn hidden_service_failure_changes_health_without_erasing_state() {
    let mut coordinator = Coordinator::default();
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.indicators.sound = false;
    coordinator.apply_settings(Ok(settings));
    assert!(coordinator.apply_service_unavailable(Sources::audio(), "monitor disconnected".into()));
    assert_eq!(coordinator.snapshot().status.sound, None);
}

#[test]
fn health_only_publication_does_not_request_a_frame() {
    let previous = Snapshot::default();
    let mut next = previous.clone();
    next.health.audio = SourceHealth::Unavailable {
        detail: "PipeWire restarted".into(),
    };

    let update = publication(&previous, next).expect("health changed");
    assert!(!update.visible);
    assert!(!update.quick_settings_visible);
}

#[test]
fn status_publication_requests_a_frame() {
    let previous = Snapshot::default();
    let mut next = previous.clone();
    next.status.focus = Some(rmac_shell_status::FocusIndicator {
        enabled: true,
        mode: Some("Work".into()),
        ends_at_unix_ms: None,
    });

    let update = publication(&previous, next).expect("status changed");
    assert!(update.visible);
    assert!(!update.quick_settings_visible);
}

#[test]
fn live_focus_authority_replaces_preferences_and_survives_source_loss() {
    let mut coordinator = Coordinator::default();
    let mut settings = rmac_shell_settings::ShellSettings::default();
    settings.focus.enabled = true;
    settings.focus.selected_mode = Some("Stale preference".into());
    coordinator.apply_settings(Ok(settings));
    assert_eq!(coordinator.snapshot().status.focus, None);
    assert!(!coordinator.snapshot().quick_settings.focus_available);

    coordinator.apply_focus(
        Ok(rmac_focus_runtime::Projection {
            enabled: true,
            mode_name: Some("Work".into()),
            ends_at_unix_ms: Some(5_000),
        }),
        true,
    );
    let live = coordinator.snapshot();
    assert_eq!(
        live.status.focus.as_ref().unwrap().mode.as_deref(),
        Some("Work")
    );
    assert!(live.quick_settings.focus_available);
    assert_eq!(live.health.focus, SourceHealth::Healthy);

    coordinator.apply_focus(Err("Focus service restarted".into()), false);
    let unavailable = coordinator.snapshot();
    assert_eq!(
        unavailable.status.focus.as_ref().unwrap().mode.as_deref(),
        Some("Work")
    );
    assert!(!unavailable.quick_settings.focus_available);
    assert!(matches!(
        unavailable.health.focus,
        SourceHealth::Unavailable { .. }
    ));
}

#[test]
fn live_focus_projection_does_not_imply_a_writable_command_channel() {
    let mut coordinator = Coordinator::default();
    coordinator.apply_focus(
        Ok(rmac_focus_runtime::Projection {
            enabled: true,
            mode_name: Some("Work".into()),
            ends_at_unix_ms: None,
        }),
        false,
    );
    let snapshot = coordinator.snapshot();
    assert_eq!(snapshot.status.focus.unwrap().mode.as_deref(), Some("Work"));
    assert!(!snapshot.quick_settings.focus_available);
    assert_eq!(snapshot.health.focus, SourceHealth::Healthy);
}

#[test]
fn notification_center_indicator_survives_a_source_restart() {
    let mut coordinator = Coordinator::default();
    assert!(
        coordinator.apply_notifications(Ok(rmac_notifications::Indicator {
            unread_count: 4,
            has_urgent: true,
        }))
    );
    let live = coordinator.snapshot();
    assert_eq!(live.status.notifications.unwrap().unread_count, 4);
    assert_eq!(live.health.notifications, SourceHealth::Healthy);

    assert!(coordinator.apply_notifications(Err("Center restarted".into())));
    let unavailable = coordinator.snapshot();
    assert_eq!(unavailable.status.notifications.unwrap().unread_count, 4);
    assert!(matches!(
        unavailable.health.notifications,
        SourceHealth::Unavailable { .. }
    ));
}

#[test]
fn quick_settings_change_does_not_redraw_the_compact_bar() {
    let previous = Snapshot::default();
    let mut next = previous.clone();
    next.quick_settings.audio = rmac_audio::Snapshot {
        available: true,
        output: rmac_audio::Level {
            volume: 55,
            muted: false,
        },
        ..Default::default()
    };

    let update = publication(&previous, next).expect("Quick Settings changed");
    assert!(!update.visible);
    assert!(update.quick_settings_visible);
}
