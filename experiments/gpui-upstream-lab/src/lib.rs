use std::{env, fs, time::Duration};

use gpui::Window;

const READY_FILE_ENV: &str = "RMAC_SMOKE_READY_FILE";

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub mod output_surfaces {
    use std::collections::{BTreeMap, BTreeSet};
    use std::rc::Rc;

    use gpui::{AnyWindowHandle, App, PlatformDisplay};
    use uuid::Uuid;

    #[derive(Default)]
    pub struct Tracker {
        windows: BTreeMap<Uuid, AnyWindowHandle>,
    }

    impl Tracker {
        pub fn len(&self) -> usize {
            self.windows.len()
        }

        pub fn reconcile(
            &mut self,
            desired: Option<&BTreeSet<Uuid>>,
            cx: &mut App,
            mut open: impl FnMut(Rc<dyn PlatformDisplay>, &mut App) -> AnyWindowHandle,
        ) {
            let displays = cx.displays();
            let available = displays
                .iter()
                .filter_map(|display| display.uuid().ok())
                .collect::<BTreeSet<_>>();
            let active = desired
                .map(|desired| {
                    desired
                        .intersection(&available)
                        .copied()
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or(available);

            let removed = self
                .windows
                .keys()
                .filter(|uuid| !active.contains(uuid))
                .copied()
                .collect::<Vec<_>>();
            for uuid in removed {
                if let Some(handle) = self.windows.remove(&uuid) {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }

            for display in displays {
                let Ok(uuid) = display.uuid() else {
                    continue;
                };
                if active.contains(&uuid) && !self.windows.contains_key(&uuid) {
                    self.windows.insert(uuid, open(display, cx));
                }
            }
        }
    }

    pub async fn watch_enabled(
        sender: async_channel::Sender<BTreeSet<Uuid>>,
    ) -> Result<(), String> {
        let (event_tx, event_rx) = async_channel::bounded(64);
        let watcher = async {
            rmac_compositor_niri::watch(event_tx)
                .await
                .map_err(|error| error.to_string())
        };
        let consumer = async {
            let mut state = rmac_compositor::State::default();
            let mut published = BTreeSet::new();
            while let Ok(event) = event_rx.recv().await {
                state.apply(event);
                let next = state
                    .snapshot()
                    .outputs
                    .into_iter()
                    .filter(|output| output.enabled())
                    .map(|output| Uuid::new_v5(&Uuid::NAMESPACE_DNS, output.id.0.as_bytes()))
                    .collect::<BTreeSet<_>>();
                if next != published {
                    published = next.clone();
                    if sender.send(next).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Ok(())
        };
        futures_util::try_join!(watcher, consumer)?;
        Ok(())
    }
}

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

pub fn delay_until_next_clock_tick(epoch_millis: u128, show_seconds: bool) -> Duration {
    if show_seconds {
        let remaining = 1_000 - epoch_millis % 1_000;
        Duration::from_millis(remaining as u64)
    } else {
        delay_until_next_minute(epoch_millis)
    }
}

pub fn top_bar_clock_pattern(settings: &rmac_shell_settings::ClockSettings) -> &'static str {
    use rmac_shell_settings::ClockFormat;

    match (settings.show_date, settings.show_seconds, settings.format) {
        (true, false, ClockFormat::TwentyFourHour) => "%a %-d %b %H:%M",
        (true, true, ClockFormat::TwentyFourHour) => "%a %-d %b %H:%M:%S",
        (false, false, ClockFormat::TwentyFourHour) => "%H:%M",
        (false, true, ClockFormat::TwentyFourHour) => "%H:%M:%S",
        (true, false, ClockFormat::Locale | ClockFormat::TwelveHour) => "%a %-d %b %-I:%M %p",
        (true, true, ClockFormat::Locale | ClockFormat::TwelveHour) => "%a %-d %b %-I:%M:%S %p",
        (false, false, ClockFormat::Locale | ClockFormat::TwelveHour) => "%-I:%M %p",
        (false, true, ClockFormat::Locale | ClockFormat::TwelveHour) => "%-I:%M:%S %p",
    }
}

pub fn top_bar_workspace_label(snapshot: &rmac_shell_status::Snapshot) -> Option<String> {
    snapshot.clock.show_workspace.then(|| {
        snapshot
            .focused
            .workspace_label
            .clone()
            .or_else(|| {
                snapshot
                    .focused
                    .workspace_id
                    .map(|workspace| workspace.0.to_string())
            })
            .unwrap_or_else(|| "No workspace".into())
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TopBarIndicatorLabel {
    pub kind: TopBarIndicatorKind,
    pub visible: String,
    pub accessible: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopBarIndicatorKind {
    Focus,
    Vpn,
    Network,
    Bluetooth,
    Sound,
    Battery,
    Notifications,
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
            kind: TopBarIndicatorKind::Focus,
            visible: String::new(),
            accessible: format!("Focus enabled: {mode}"),
        });
    }
    if let Some(vpn) = snapshot
        .vpn
        .as_ref()
        .filter(|vpn| vpn.transitioning || !vpn.active_names.is_empty())
    {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Vpn,
            visible: String::new(),
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
            kind: TopBarIndicatorKind::Network,
            visible: String::new(),
            accessible: format!("Wi-Fi {}{strength}", network_state_label(network.state)),
        });
    }
    if let Some(bluetooth) = snapshot
        .bluetooth
        .as_ref()
        .filter(|bluetooth| bluetooth.powered)
    {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Bluetooth,
            visible: String::new(),
            accessible: format!(
                "Bluetooth on, {} connected devices",
                bluetooth.connected_devices
            ),
        });
    }
    if let Some(sound) = snapshot.sound.filter(|sound| sound.available) {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Sound,
            visible: String::new(),
            accessible: if sound.muted {
                "Sound muted".into()
            } else {
                format!("Sound volume {} percent", sound.volume)
            },
        });
    }
    if let Some(battery) = snapshot.battery {
        let percentage = format!("{}%", battery.percentage);
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Battery,
            visible: if snapshot.show_battery_percentage {
                percentage.clone()
            } else {
                String::new()
            },
            accessible: format!("Battery {percentage}, {}", battery.state.label()),
        });
    }
    if let Some(notifications) = snapshot
        .notifications
        .filter(|notifications| notifications.unread_count > 0)
    {
        labels.push(TopBarIndicatorLabel {
            kind: TopBarIndicatorKind::Notifications,
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
    fn clock_tick_and_pattern_follow_visible_precision() {
        let mut settings = rmac_shell_settings::ClockSettings::default();
        assert_eq!(
            delay_until_next_clock_tick(59_999, false),
            Duration::from_millis(1)
        );
        assert_eq!(top_bar_clock_pattern(&settings), "%a %-d %b %-I:%M %p");

        settings.show_date = false;
        settings.show_seconds = true;
        settings.format = rmac_shell_settings::ClockFormat::TwentyFourHour;
        assert_eq!(
            delay_until_next_clock_tick(1_999, true),
            Duration::from_millis(1)
        );
        assert_eq!(top_bar_clock_pattern(&settings), "%H:%M:%S");
    }

    #[test]
    fn workspace_label_is_only_projected_when_enabled() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.focused.workspace_label = Some("Writing".into());
        assert_eq!(top_bar_workspace_label(&snapshot), None);
        snapshot.clock.show_workspace = true;
        assert_eq!(
            top_bar_workspace_label(&snapshot).as_deref(),
            Some("Writing")
        );
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
    fn unavailable_sound_is_not_shown_as_a_meaningless_status_item() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.sound = Some(rmac_shell_status::SoundIndicator::default());

        let labels = top_bar_indicator_labels(&snapshot);
        assert!(labels.is_empty());
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
