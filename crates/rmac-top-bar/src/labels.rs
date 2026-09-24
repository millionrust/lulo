use std::time::Duration;

use chrono::{DateTime, FixedOffset};

use crate::model::{MAX_ACTIVE_APP_CHARACTERS, MAX_MODE_CHARACTERS, MAX_VPN_NAMES};
use crate::{ClockLabel, IndicatorKind, IndicatorLabel, LocaleHourCycle, PanelTarget};

pub fn delay_until_next_clock_update(epoch_millis: i64, show_seconds: bool) -> Duration {
    let interval = if show_seconds { 1_000 } else { 60_000 };
    let remaining = interval - epoch_millis.rem_euclid(interval);
    Duration::from_millis(remaining as u64)
}

pub fn clock_label(
    now: DateTime<FixedOffset>,
    settings: &rmac_shell_settings::ClockSettings,
    locale_hour_cycle: LocaleHourCycle,
) -> ClockLabel {
    let twenty_four_hour = match settings.format {
        rmac_shell_settings::ClockFormat::Locale => {
            locale_hour_cycle == LocaleHourCycle::TwentyFourHour
        }
        rmac_shell_settings::ClockFormat::TwelveHour => false,
        rmac_shell_settings::ClockFormat::TwentyFourHour => true,
    };
    let time_format = match (twenty_four_hour, settings.show_seconds) {
        (true, true) => "%H:%M:%S",
        (true, false) => "%H:%M",
        (false, true) => "%-I:%M:%S %p",
        (false, false) => "%-I:%M %p",
    };
    let time = now.format(time_format).to_string();
    let visible = if settings.show_date {
        format!("{}  {time}", now.format("%a %b %-d"))
    } else {
        time
    };
    ClockLabel {
        visible,
        accessible: now
            .format(if twenty_four_hour {
                if settings.show_seconds {
                    "%A, %B %-d, %Y, %H:%M:%S"
                } else {
                    "%A, %B %-d, %Y, %H:%M"
                }
            } else if settings.show_seconds {
                "%A, %B %-d, %Y, %-I:%M:%S %p"
            } else {
                "%A, %B %-d, %Y, %-I:%M %p"
            })
            .to_string(),
        activation: PanelTarget::NotificationCenter,
    }
}

pub fn active_app_name(snapshot: &rmac_shell_status::Snapshot) -> String {
    snapshot
        .focused
        .app_id
        .as_deref()
        .and_then(|app_id| {
            app_id
                .rsplit(['.', '/'])
                .find(|part| !part.trim().is_empty())
        })
        .map(|name| bounded(name.trim(), MAX_ACTIVE_APP_CHARACTERS))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Lulo OS".into())
}

pub fn indicator_labels(snapshot: &rmac_shell_status::Snapshot) -> Vec<IndicatorLabel> {
    let mut labels = Vec::new();
    if let Some(focus) = snapshot.focus.as_ref().filter(|focus| focus.enabled) {
        let mode = focus
            .mode
            .as_deref()
            .and_then(nonempty)
            .map(|mode| bounded(mode, MAX_MODE_CHARACTERS))
            .unwrap_or_else(|| "Focus".into());
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Focus,
            icon: IndicatorKind::Focus.into(),
            activation: IndicatorKind::Focus.panel_target(),
            visible: "Focus".into(),
            accessible: format!("Focus enabled: {mode}"),
            urgent: false,
        });
    }
    if let Some(vpn) = snapshot
        .vpn
        .as_ref()
        .filter(|vpn| vpn.transitioning || !vpn.active_names.is_empty())
    {
        let mut names = vpn
            .active_names
            .iter()
            .filter_map(|name| nonempty(name))
            .take(MAX_VPN_NAMES)
            .map(|name| bounded(name, MAX_MODE_CHARACTERS))
            .collect::<Vec<_>>();
        let omitted = vpn.active_names.len().saturating_sub(names.len());
        if omitted > 0 {
            names.push(format!("and {omitted} more"));
        }
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Vpn,
            icon: IndicatorKind::Vpn.into(),
            activation: IndicatorKind::Vpn.panel_target(),
            visible: "VPN".into(),
            accessible: if names.is_empty() {
                "VPN connecting".into()
            } else {
                format!("VPN connected: {}", names.join(", "))
            },
            urgent: false,
        });
    }
    if let Some(network) = &snapshot.network {
        let strength = network
            .wifi_bars
            .map(|bars| format!(", signal {bars} of 3 bars"))
            .unwrap_or_default();
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Network,
            icon: IndicatorKind::Network.into(),
            activation: IndicatorKind::Network.panel_target(),
            visible: "Wi-Fi".into(),
            accessible: format!("Wi-Fi {}{strength}", network_state_label(network.state)),
            urgent: false,
        });
    }
    if let Some(bluetooth) = &snapshot.bluetooth {
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Bluetooth,
            icon: IndicatorKind::Bluetooth.into(),
            activation: IndicatorKind::Bluetooth.panel_target(),
            visible: "Bluetooth".into(),
            accessible: if !bluetooth.available {
                "Bluetooth unavailable".into()
            } else if !bluetooth.powered {
                "Bluetooth off".into()
            } else {
                format!(
                    "Bluetooth on, {} connected {}",
                    bluetooth.connected_devices,
                    if bluetooth.connected_devices == 1 {
                        "device"
                    } else {
                        "devices"
                    }
                )
            },
            urgent: false,
        });
    }
    if let Some(sound) = snapshot.sound {
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Sound,
            icon: IndicatorKind::Sound.into(),
            activation: IndicatorKind::Sound.panel_target(),
            visible: if !sound.available {
                "Sound".into()
            } else if sound.muted {
                "Mute".into()
            } else {
                format!("{}%", sound.volume.min(100))
            },
            accessible: if !sound.available {
                "Sound unavailable".into()
            } else if sound.muted {
                "Sound muted".into()
            } else {
                format!("Sound volume {} percent", sound.volume.min(100))
            },
            urgent: false,
        });
    }
    if let Some(battery) = snapshot.battery {
        let percentage = format!("{}%", battery.percentage.min(100));
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Battery,
            icon: IndicatorKind::Battery.into(),
            activation: IndicatorKind::Battery.panel_target(),
            visible: if snapshot.show_battery_percentage {
                percentage.clone()
            } else {
                "Battery".into()
            },
            accessible: format!("Battery {percentage}, {}", battery.state.label()),
            urgent: false,
        });
    }
    if let Some(notifications) = snapshot
        .notifications
        .filter(|notifications| notifications.unread_count > 0)
    {
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Notifications,
            icon: IndicatorKind::Notifications.into(),
            activation: IndicatorKind::Notifications.panel_target(),
            visible: if notifications.unread_count > 99 {
                "99+".into()
            } else {
                notifications.unread_count.to_string()
            },
            accessible: format!(
                "{} unread {}",
                notifications.unread_count,
                if notifications.unread_count == 1 {
                    "notification"
                } else {
                    "notifications"
                }
            ),
            urgent: notifications.has_urgent,
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

pub(crate) fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

pub(crate) fn bounded(value: &str, limit: usize) -> String {
    let mut characters = value.chars();
    let mut result = characters.by_ref().take(limit).collect::<String>();
    if characters.next().is_some() {
        result.push('…');
    }
    result
}
