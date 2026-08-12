use std::{collections::BTreeMap, env, fs, time::Duration};

use gpui::Window;
use uuid::Uuid;

const READY_FILE_ENV: &str = "RMAC_SMOKE_READY_FILE";
pub const WAYLAND_OUTPUT_RESTART_EXIT_CODE: i32 = 75;

/// Renderer-neutral shell materials shared by the menu bar, Dock and OSD.
///
/// The compositor owns backdrop sampling and blur. These colors provide the
/// tint and hierarchy above that sampled backdrop without baking wallpaper
/// pixels into an application texture.
pub mod shell_visuals {
    pub const PRIMARY_TEXT: u32 = 0xf7f8faff;
    pub const SECONDARY_TEXT: u32 = 0xf7f8faaa;
    pub const DISABLED_TEXT: u32 = 0xf7f8fa66;

    pub const TOP_BAR_TINT: u32 = 0x0b0d143d;
    pub const REGULAR_DARK_TINT: u32 = 0x202630f4;
    pub const HUD_TINT: u32 = 0x18202b9c;
    pub const DOCK_TINT: u32 = 0xe7ecf18c;

    pub const LIGHT_HOVER: u32 = 0xffffff22;
    pub const LIGHT_SELECTION: u32 = 0xffffff2d;
    pub const LIGHT_BORDER: u32 = 0xffffff35;
    pub const DOCK_BORDER: u32 = 0xffffffb8;
    pub const SEPARATOR: u32 = 0x4a56646b;
    pub const ACCENT: u32 = 0x2878d4ff;
    pub const ACCENT_HOVER: u32 = 0x3488e8ff;

    pub const MENU_RADIUS: f32 = 9.0;
    pub const MENU_ITEM_RADIUS: f32 = 5.0;
    pub const DOCK_RADIUS: f32 = 26.0;
    pub const TOOLTIP_RADIUS: f32 = 8.0;
    pub const HUD_RADIUS: f32 = 28.0;
}

pub fn output_reappeared(
    previous: &std::collections::BTreeSet<Uuid>,
    current: &std::collections::BTreeSet<Uuid>,
    removed: &mut std::collections::BTreeSet<Uuid>,
) -> bool {
    removed.extend(previous.difference(current).copied());
    current.iter().any(|output| removed.contains(output))
}

/// Stable per-output menu-bar policy derived from niri's authoritative
/// workspace/window geometry. Niri does not publish a separate fullscreen
/// boolean: a fullscreen tile is the one active tile whose logical extent
/// exactly covers its complete output instead of the compositor work area.
pub fn top_bar_output_policies(snapshot: &rmac_compositor::Snapshot) -> BTreeMap<Uuid, bool> {
    snapshot
        .outputs
        .iter()
        .filter_map(|output| {
            let logical = output.logical.as_ref()?;
            if !output.enabled() || !logical.size.is_valid() {
                return None;
            }
            let fullscreen = !snapshot.overview_visible
                && snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| {
                        workspace.active && workspace.output.as_ref() == Some(&output.id)
                    })
                    .and_then(|workspace| workspace.active_window)
                    .and_then(|window_id| {
                        snapshot
                            .windows
                            .iter()
                            .find(|window| window.id == window_id && window.workspace.is_some())
                    })
                    .is_some_and(|window| {
                        nearly_equal(window.layout.tile_size.width, logical.size.width)
                            && nearly_equal(window.layout.tile_size.height, logical.size.height)
                    });
            Some((stable_output_uuid(&output.id), fullscreen))
        })
        .collect()
}

fn nearly_equal(left: f64, right: f64) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= 0.5
}

pub fn stable_output_uuid(output: &rmac_compositor::OutputId) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, output.0.as_bytes())
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub mod output_surfaces {
    use std::collections::{BTreeMap, BTreeSet};
    use std::rc::Rc;

    use gpui::{AnyWindowHandle, App, PlatformDisplay};
    use uuid::Uuid;

    pub fn newest_displays(cx: &App) -> BTreeMap<Uuid, Rc<dyn PlatformDisplay>> {
        let mut displays: BTreeMap<Uuid, Rc<dyn PlatformDisplay>> = BTreeMap::new();
        for display in cx.displays() {
            let Ok(uuid) = display.uuid() else {
                continue;
            };
            let replace = displays
                .get(&uuid)
                .is_some_and(|current| u32::from(display.id()) > u32::from(current.id()));
            if replace || !displays.contains_key(&uuid) {
                displays.insert(uuid, display);
            }
        }
        displays
    }

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
            let displays = newest_displays(cx);
            let available = displays.keys().copied().collect::<BTreeSet<_>>();
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

            for (uuid, display) in displays {
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
                    .map(|output| crate::stable_output_uuid(&output.id))
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

    pub async fn watch_top_bar(
        sender: async_channel::Sender<BTreeMap<Uuid, bool>>,
    ) -> Result<(), String> {
        let (event_tx, event_rx) = async_channel::bounded(64);
        let watcher = async {
            rmac_compositor_niri::watch(event_tx)
                .await
                .map_err(|error| error.to_string())
        };
        let consumer = async {
            let mut state = rmac_compositor::State::default();
            let mut published = BTreeMap::new();
            while let Ok(event) = event_rx.recv().await {
                state.apply(event);
                let next = crate::top_bar_output_policies(&state.snapshot());
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
        .map(|app_id| {
            rmac_apps::identity::window_title(app_id)
                .map(str::to_owned)
                .unwrap_or_else(|| humanize_app_id(app_id))
        })
        .filter(|name| !name.is_empty())
        // Like macOS Finder, the first-party file manager owns the desktop
        // identity when no application window has focus.
        .unwrap_or_else(|| "Finder".to_owned())
}

fn humanize_app_id(app_id: &str) -> String {
    let leaf = app_id.rsplit(['.', '/']).next().unwrap_or(app_id);
    leaf.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) if first.is_lowercase() => {
                    first.to_uppercase().chain(characters).collect()
                }
                Some(first) => first.to_string() + characters.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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

    fn compositor_snapshot(
        tile_height: f64,
        active_window: Option<u64>,
    ) -> rmac_compositor::Snapshot {
        rmac_compositor::Snapshot {
            outputs: vec![rmac_compositor::Output {
                id: rmac_compositor::OutputId::from("eDP-1"),
                make: String::new(),
                model: String::new(),
                serial: None,
                physical_size_mm: None,
                modes: vec![rmac_compositor::OutputMode {
                    physical_size: rmac_compositor::PhysicalSize {
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
                logical: Some(rmac_compositor::LogicalOutput {
                    position: rmac_compositor::LogicalPoint::default(),
                    size: rmac_compositor::LogicalSize {
                        width: 1536.0,
                        height: 864.0,
                    },
                    scale: 1.25,
                    transform: "normal".into(),
                }),
            }],
            workspaces: vec![rmac_compositor::Workspace {
                id: rmac_compositor::WorkspaceId(1),
                index: 1,
                name: None,
                output: Some(rmac_compositor::OutputId::from("eDP-1")),
                urgent: false,
                active: true,
                focused: true,
                active_window: active_window.map(rmac_compositor::WindowId),
            }],
            windows: active_window
                .map(|id| rmac_compositor::Window {
                    id: rmac_compositor::WindowId(id),
                    title: None,
                    app_id: Some("org.rmac.Test".into()),
                    pid: None,
                    workspace: Some(rmac_compositor::WorkspaceId(1)),
                    focused: true,
                    floating: false,
                    urgent: false,
                    focus_timestamp: None,
                    layout: rmac_compositor::WindowLayout {
                        tile_size: rmac_compositor::LogicalSize {
                            width: 1536.0,
                            height: tile_height,
                        },
                        ..Default::default()
                    },
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn minute_delay_is_bounded_and_never_busy_loops() {
        assert_eq!(delay_until_next_minute(0), Duration::from_secs(60));
        assert_eq!(delay_until_next_minute(59_999), Duration::from_millis(1));
        assert_eq!(delay_until_next_minute(60_000), Duration::from_secs(60));
    }

    #[test]
    fn top_bar_fullscreen_policy_uses_complete_output_geometry() {
        let uuid = stable_output_uuid(&rmac_compositor::OutputId::from("eDP-1"));
        assert_eq!(
            top_bar_output_policies(&compositor_snapshot(836.0, Some(7))).get(&uuid),
            Some(&false)
        );
        assert_eq!(
            top_bar_output_policies(&compositor_snapshot(864.0, Some(7))).get(&uuid),
            Some(&true)
        );
        assert_eq!(
            top_bar_output_policies(&compositor_snapshot(864.0, None)).get(&uuid),
            Some(&false)
        );

        let mut overview = compositor_snapshot(864.0, Some(7));
        overview.overview_visible = true;
        assert_eq!(top_bar_output_policies(&overview).get(&uuid), Some(&false));
    }

    #[test]
    fn output_reappearance_requires_a_fresh_wayland_registry() {
        let first = Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"first");
        let second = Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"second");
        let mut removed = std::collections::BTreeSet::new();
        let both = [first, second].into_iter().collect();
        let first_only = [first].into_iter().collect();

        assert!(!output_reappeared(&both, &first_only, &mut removed));
        assert!(output_reappeared(&first_only, &both, &mut removed));
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
    fn first_party_identity_uses_its_reviewed_application_name() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.focused.app_id = Some(rmac_apps::identity::APP_DRAWER.into());
        assert_eq!(top_bar_active_app_name(&snapshot), "Apps");
    }

    #[test]
    fn desktop_without_a_focused_window_is_owned_by_finder() {
        assert_eq!(
            top_bar_active_app_name(&rmac_shell_status::Snapshot::default()),
            "Finder"
        );
    }

    #[test]
    fn generic_desktop_ids_are_humanized_without_using_document_titles() {
        let mut snapshot = rmac_shell_status::Snapshot::default();
        snapshot.focused.app_id = Some("org.mozilla.firefox-nightly".into());
        snapshot.focused.title = Some("Private document title".into());
        assert_eq!(top_bar_active_app_name(&snapshot), "Firefox Nightly");
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
