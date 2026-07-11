use std::{env, fs, time::Duration};

use gpui::Window;

const READY_FILE_ENV: &str = "RMAC_SMOKE_READY_FILE";

/// Writes an opt-in marker after GPUI finishes the window's first frame.
///
/// The nested-Wayland smoke harness uses this to distinguish a rendered
/// surface from a process that merely reached `open_window`. Normal runs do
/// not set the environment variable and perform no filesystem I/O.
pub fn mark_first_frame(window: &Window, probe: &'static str) {
    let Some(path) = env::var_os(READY_FILE_ENV) else {
        return;
    };

    window.on_next_frame(move |_, _| {
        fs::write(&path, format!("{probe}\n"))
            .unwrap_or_else(|error| panic!("write first-frame marker {path:?}: {error}"));
    });
}

/// Delay to the next wall-clock minute without a periodic redraw loop.
pub fn delay_until_next_minute(epoch_millis: u128) -> Duration {
    const MINUTE_MILLIS: u128 = 60_000;
    let remaining = MINUTE_MILLIS - epoch_millis % MINUTE_MILLIS;
    Duration::from_millis(remaining as u64)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopBarIndicatorLabel {
    pub visible: String,
    pub accessible: String,
}

pub fn top_bar_active_app_name(snapshot: &rmac_shell_status::Snapshot) -> String {
    snapshot
        .focused
        .app_id
        .as_deref()
        .and_then(|app_id| app_id.rsplit(['.', '/']).next())
        .filter(|name| !name.is_empty())
        .unwrap_or("rmac")
        .to_owned()
}

pub fn top_bar_indicator_labels(
    snapshot: &rmac_shell_status::Snapshot,
) -> Vec<TopBarIndicatorLabel> {
    let mut labels = Vec::new();
    if let Some(focus) = snapshot.focus.as_ref().filter(|focus| focus.enabled) {
        let mode = focus.mode.as_deref().unwrap_or("Focus");
        labels.push(TopBarIndicatorLabel {
            visible: "◐".into(),
            accessible: format!("Focus enabled: {mode}"),
        });
    }
    if let Some(vpn) = snapshot
        .vpn
        .as_ref()
        .filter(|vpn| vpn.transitioning || !vpn.active_names.is_empty())
    {
        labels.push(TopBarIndicatorLabel {
            visible: "VPN".into(),
            accessible: if vpn.active_names.is_empty() {
                "VPN connecting".into()
            } else {
                format!("VPN connected: {}", vpn.active_names.join(", "))
            },
        });
    }
    if let Some(network) = &snapshot.network {
        let strength = network
            .wifi_strength
            .map(|strength| format!(", signal {strength} percent"))
            .unwrap_or_default();
        labels.push(TopBarIndicatorLabel {
            visible: "Wi-Fi".into(),
            accessible: format!("Wi-Fi {}{strength}", network_state_label(network.state)),
        });
    }
    if let Some(bluetooth) = snapshot
        .bluetooth
        .as_ref()
        .filter(|bluetooth| bluetooth.powered)
    {
        labels.push(TopBarIndicatorLabel {
            visible: "ᛒ".into(),
            accessible: format!(
                "Bluetooth on, {} connected devices",
                bluetooth.connected_devices
            ),
        });
    }
    if let Some(sound) = snapshot.sound {
        labels.push(TopBarIndicatorLabel {
            visible: if !sound.available {
                "Sound".into()
            } else if sound.muted {
                "Mute".into()
            } else {
                format!("Vol {}%", sound.volume)
            },
            accessible: if !sound.available {
                "Sound unavailable".into()
            } else if sound.muted {
                "Sound muted".into()
            } else {
                format!("Sound volume {} percent", sound.volume)
            },
        });
    }
    if let Some(battery) = snapshot.battery {
        let percentage = format!("{}%", battery.percentage);
        labels.push(TopBarIndicatorLabel {
            visible: if snapshot.show_battery_percentage {
                percentage.clone()
            } else {
                "Battery".into()
            },
            accessible: format!("Battery {percentage}, {}", battery.state.label()),
        });
    }
    if let Some(notifications) = snapshot
        .notifications
        .filter(|notifications| notifications.unread_count > 0)
    {
        labels.push(TopBarIndicatorLabel {
            visible: notifications.unread_count.to_string(),
            accessible: format!("{} unread notifications", notifications.unread_count),
        });
    }
    labels
}

fn network_state_label(state: rmac_shell_status::NetworkState) -> &'static str {
    match state {
        rmac_shell_status::NetworkState::Unavailable => "unavailable",
        rmac_shell_status::NetworkState::Disconnected => "disconnected",
        rmac_shell_status::NetworkState::Portal => "requires sign-in",
        rmac_shell_status::NetworkState::Limited => "limited",
        rmac_shell_status::NetworkState::Connected => "connected",
        rmac_shell_status::NetworkState::Unknown => "state unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minute_delay_is_bounded_and_never_busy_loops() {
        assert_eq!(delay_until_next_minute(0), Duration::from_secs(60));
        assert_eq!(delay_until_next_minute(59_999), Duration::from_millis(1));
        assert_eq!(delay_until_next_minute(60_000), Duration::from_secs(60));
    }

    #[test]
    fn focused_application_uses_the_stable_app_identity() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.focused.app_id = Some("dev.rmac.Finder".into());
        snapshot.focused.title = Some("Downloads".into());
        assert_eq!(top_bar_active_app_name(&snapshot), "Finder");
    }

    #[test]
    fn hidden_indicators_do_not_create_placeholder_items() {
        let snapshot = rmac_shell_status::Snapshot::default();
        assert!(top_bar_indicator_labels(&snapshot).is_empty());
    }

    #[test]
    fn unavailable_sound_is_not_presented_as_zero_volume() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.sound = Some(rmac_shell_status::SoundIndicator::default());

        let labels = top_bar_indicator_labels(&snapshot);
        assert_eq!(labels[0].visible, "Sound");
        assert_eq!(labels[0].accessible, "Sound unavailable");
    }

    #[test]
    fn live_indicators_have_human_accessible_state() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.network = Some(rmac_shell_status::NetworkIndicator {
            state: rmac_shell_status::NetworkState::Connected,
            connection_name: Some("Home".into()),
            wifi_strength: Some(82),
        });
        snapshot.sound = Some(rmac_shell_status::SoundIndicator {
            available: true,
            volume: 37,
            muted: false,
        });

        let labels = top_bar_indicator_labels(&snapshot);
        assert_eq!(labels[0].accessible, "Wi-Fi connected, signal 82 percent");
        assert_eq!(labels[1].accessible, "Sound volume 37 percent");
    }
}
