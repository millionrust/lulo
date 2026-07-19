//! Framework-neutral, per-output presentation model for the rmac top bar.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, FixedOffset};

pub const BAR_HEIGHT: f64 = 32.0;
const MAX_ACTIVE_APP_CHARACTERS: usize = 48;
const MAX_WORKSPACE_CHARACTERS: usize = 32;
const MAX_MODE_CHARACTERS: usize = 48;
const MAX_VPN_NAMES: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocaleHourCycle {
    TwelveHour,
    TwentyFourHour,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockLabel {
    pub visible: String,
    pub accessible: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorKind {
    Focus,
    Vpn,
    Network,
    Bluetooth,
    Sound,
    Battery,
    Notifications,
}

#[derive(Clone, Eq, PartialEq)]
pub struct IndicatorLabel {
    pub kind: IndicatorKind,
    pub visible: String,
    pub accessible: String,
    pub urgent: bool,
}

impl fmt::Debug for IndicatorLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndicatorLabel")
            .field("kind", &self.kind)
            .field("visible", &"<redacted>")
            .field("accessible", &"<redacted>")
            .field("urgent", &self.urgent)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub output: rmac_compositor::OutputId,
    pub logical_width: f64,
    pub logical_height: f64,
    pub scale: f64,
    pub exclusive_zone: f64,
    pub keyboard_interactive: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Content {
    pub active_app: String,
    pub workspace: Option<String>,
    pub clock: ClockLabel,
    pub indicators: Vec<IndicatorLabel>,
}

impl fmt::Debug for Content {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Content")
            .field("active_app", &"<redacted>")
            .field("workspace", &self.workspace.as_ref().map(|_| "<redacted>"))
            .field("clock", &"<redacted>")
            .field("indicator_count", &self.indicators.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    pub surfaces: Vec<Surface>,
    pub content: Content,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub projection: Projection,
    /// True only when a renderer-visible value or output surface changed.
    pub redraw: bool,
    /// One event-driven clock deadline; never a frame-loop interval.
    pub next_clock_update: Duration,
}

#[derive(Default)]
pub struct State {
    accepted: Option<Projection>,
}

impl State {
    pub fn apply(
        &mut self,
        status: &rmac_shell_status::Snapshot,
        now: DateTime<FixedOffset>,
        locale_hour_cycle: LocaleHourCycle,
    ) -> Update {
        let projection = project(status, now, locale_hour_cycle);
        let redraw = self.accepted.as_ref() != Some(&projection);
        if redraw {
            self.accepted = Some(projection.clone());
        }
        Update {
            projection,
            redraw,
            next_clock_update: delay_until_next_clock_update(
                now.timestamp_millis(),
                status.clock.show_seconds,
            ),
        }
    }
}

pub fn project(
    status: &rmac_shell_status::Snapshot,
    now: DateTime<FixedOffset>,
    locale_hour_cycle: LocaleHourCycle,
) -> Projection {
    let mut surfaces = status
        .outputs
        .iter()
        .filter(|output| {
            output.logical_size.is_valid()
                && output.logical_size.width > 0.0
                && output.logical_size.height > 0.0
                && output.scale.is_finite()
                && output.scale > 0.0
        })
        .map(|output| Surface {
            output: output.id.clone(),
            logical_width: output.logical_size.width,
            logical_height: BAR_HEIGHT,
            scale: output.scale,
            exclusive_zone: BAR_HEIGHT,
            keyboard_interactive: false,
        })
        .collect::<Vec<_>>();
    surfaces.sort_by(|left, right| left.output.cmp(&right.output));
    Projection {
        surfaces,
        content: Content {
            active_app: active_app_name(status),
            workspace: status
                .clock
                .show_workspace
                .then_some(status.focused.workspace_label.as_deref())
                .flatten()
                .and_then(nonempty)
                .map(|label| bounded(label, MAX_WORKSPACE_CHARACTERS)),
            clock: clock_label(now, &status.clock, locale_hour_cycle),
            indicators: indicator_labels(status),
        },
    }
}

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
        .unwrap_or_else(|| "rmac".into())
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
            .wifi_strength
            .map(|strength| format!(", signal {} percent", strength.min(100)))
            .unwrap_or_default();
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Network,
            visible: "Wi-Fi".into(),
            accessible: format!("Wi-Fi {}{strength}", network_state_label(network.state)),
            urgent: false,
        });
    }
    if let Some(bluetooth) = &snapshot.bluetooth {
        labels.push(IndicatorLabel {
            kind: IndicatorKind::Bluetooth,
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

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn bounded(value: &str, limit: usize) -> String {
    let mut characters = value.chars();
    let mut result = characters.by_ref().take(limit).collect::<String>();
    if characters.next().is_some() {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

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
        assert_eq!(content.active_app, "Finder");
        assert!(
            content.workspace.as_ref().unwrap().chars().count() <= MAX_WORKSPACE_CHARACTERS + 1
        );
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
        assert_eq!(labels[0].accessible, "Wi-Fi connected, signal 82 percent");
        assert_eq!(labels[1].accessible, "Bluetooth off");
        assert_eq!(labels[2].accessible, "Sound volume 37 percent");
        assert_eq!(labels[3].visible, "99+");
        assert!(labels[3].urgent);
        assert!(!format!("{labels:?}").contains("Private SSID"));
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
}
