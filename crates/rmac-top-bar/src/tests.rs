use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, TimeZone as _};

use super::*;
use crate::model::MAX_WORKSPACE_CHARACTERS;

fn now() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(5 * 3600 + 30 * 60)
        .unwrap()
        .with_ymd_and_hms(2026, 7, 19, 21, 7, 8)
        .single()
        .unwrap()
}

fn output(id: &str, width: f64, scale: f64) -> rmac_shell_status::OutputContext {
    rmac_shell_status::OutputContext {
        id: id.into(),
        logical_size: rmac_compositor::LogicalSize {
            width,
            height: 1080.0,
        },
        scale,
    }
}

#[test]
fn one_noninteractive_exclusive_surface_is_planned_per_sorted_output() {
    let status = rmac_shell_status::Snapshot {
        outputs: vec![
            output("DP-2", 1280.0, 2.0),
            output("BAD", f64::NAN, 0.0),
            output("DP-1", 1920.0, 1.0),
        ],
        ..Default::default()
    };
    let projection = project(&status, now(), LocaleHourCycle::TwelveHour);
    assert_eq!(projection.surfaces.len(), 2);
    assert_eq!(projection.surfaces[0].output.0, "DP-1");
    assert_eq!(projection.surfaces[1].scale, 2.0);
    assert!(projection.surfaces.iter().all(|surface| {
        surface.logical_height == BAR_HEIGHT
            && surface.exclusive_zone == BAR_HEIGHT
            && !surface.keyboard_interactive
    }));
}

#[test]
fn clock_policy_controls_hour_cycle_seconds_date_and_single_deadline() {
    let mut settings = rmac_shell_settings::ClockSettings::default();
    let twelve = clock_label(now(), &settings, LocaleHourCycle::TwelveHour);
    assert_eq!(twelve.visible, "Sun Jul 19  9:07 PM");
    assert!(twelve.accessible.contains("9:07 PM"));
    assert_eq!(twelve.activation, PanelTarget::NotificationCenter);

    settings.format = rmac_shell_settings::ClockFormat::TwentyFourHour;
    settings.show_date = false;
    settings.show_seconds = true;
    let twenty_four = clock_label(now(), &settings, LocaleHourCycle::TwelveHour);
    assert_eq!(twenty_four.visible, "21:07:08");
    assert_eq!(
        delay_until_next_clock_update(8_999, true),
        Duration::from_millis(1)
    );
    assert_eq!(
        delay_until_next_clock_update(60_000, false),
        Duration::from_secs(60)
    );
}

#[test]
fn focused_identity_and_optional_workspace_are_bounded_without_using_title() {
    let mut status = rmac_shell_status::Snapshot::default();
    status.focused.app_id = Some("dev.rmac.Finder".into());
    status.focused.title = Some("Private client file".into());
    status.focused.workspace_label = Some("Build".repeat(20));
    status.clock.show_workspace = true;
    let content = project(&status, now(), LocaleHourCycle::TwelveHour).content;
    assert_eq!(content.system_mark.icon, BuiltinIcon::System);
    assert_eq!(content.system_mark.accessible, "rmac desktop");
    assert_eq!(content.active_app, "Finder");
    assert!(content.workspace.as_ref().unwrap().chars().count() <= MAX_WORKSPACE_CHARACTERS + 1);
    assert!(!format!("{content:?}").contains("Private client"));
}

#[test]
fn every_live_indicator_has_compact_and_accessible_state() {
    let status = rmac_shell_status::Snapshot {
        network: Some(rmac_shell_status::NetworkIndicator {
            state: rmac_shell_status::NetworkState::Connected,
            connection_name: Some("Private SSID".into()),
            wifi_strength: Some(82),
        }),
        bluetooth: Some(rmac_shell_status::BluetoothIndicator {
            available: true,
            powered: false,
            connected_devices: 0,
        }),
        sound: Some(rmac_shell_status::SoundIndicator {
            available: true,
            volume: 37,
            muted: false,
        }),
        notifications: Some(rmac_shell_status::NotificationIndicator {
            unread_count: 120,
            has_urgent: true,
        }),
        ..Default::default()
    };
    let labels = indicator_labels(&status);
    assert_eq!(labels[0].accessible, "Wi-Fi connected, signal 3 of 3 bars");
    assert_eq!(labels[1].accessible, "Bluetooth off");
    assert_eq!(labels[2].accessible, "Sound volume 37 percent");
    assert_eq!(labels[3].visible, "99+");
    assert!(labels[3].urgent);
    assert!(labels
        .iter()
        .all(|label| label.icon == BuiltinIcon::from(label.kind)));
    assert!(labels[..3]
        .iter()
        .all(|label| label.activation == PanelTarget::QuickSettings));
    assert_eq!(labels[3].activation, PanelTarget::NotificationCenter);
    assert!(!format!("{labels:?}").contains("Private SSID"));
}

#[test]
fn original_top_bar_assets_are_embedded_tintable_and_unique() {
    let mut assets = BTreeSet::new();
    for icon in BuiltinIcon::ALL {
        let svg = icon.svg();
        assert!(svg.starts_with("<svg "));
        assert!(svg.contains("viewBox=\"0 0 20 20\""));
        assert!(svg.contains("currentColor"));
        assert!(!svg.contains("<script"));
        assert!(!svg.contains("<image"));
        assert!(!svg.contains("href="));
        assert!(assets.insert(svg));
    }
}

#[test]
fn duplicate_projection_never_requests_an_idle_frame() {
    let status = rmac_shell_status::Snapshot::default();
    let mut state = State::default();
    assert!(
        state
            .apply(&status, now(), LocaleHourCycle::TwelveHour)
            .redraw
    );
    assert!(
        !state
            .apply(&status, now(), LocaleHourCycle::TwelveHour)
            .redraw
    );
}
