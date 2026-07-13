//! Framework-neutral, redraw-aware state for shell status surfaces.
//!
//! Platform adapters publish typed events into [`State`]. The reducer owns no
//! D-Bus connection, compositor socket, timer, or GPUI entity, so the top bar,
//! quick settings, tests, and future notification service can share one
//! coherent projection without polling one another.

use rmac_compositor::{FocusTarget, OutputId, WindowId, WorkspaceId};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FocusedContext {
    pub output: Option<OutputId>,
    pub workspace_id: Option<WorkspaceId>,
    pub workspace_label: Option<String>,
    pub window_id: Option<WindowId>,
    pub app_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NetworkState {
    Unavailable,
    Disconnected,
    Portal,
    Limited,
    Connected,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkIndicator {
    pub state: NetworkState,
    pub connection_name: Option<String>,
    pub wifi_strength: Option<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VpnIndicator {
    pub active_names: Vec<String>,
    pub transitioning: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BluetoothIndicator {
    pub available: bool,
    pub powered: bool,
    pub connected_devices: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SoundIndicator {
    pub available: bool,
    pub volume: u8,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatteryIndicator {
    pub percentage: u8,
    pub state: rmac_power::BatteryState,
    pub on_battery: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FocusIndicator {
    pub enabled: bool,
    pub mode: Option<String>,
    pub ends_at_unix_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NotificationIndicator {
    pub unread_count: u32,
    pub has_urgent: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub focused: FocusedContext,
    pub network: Option<NetworkIndicator>,
    pub vpn: Option<VpnIndicator>,
    pub bluetooth: Option<BluetoothIndicator>,
    pub sound: Option<SoundIndicator>,
    pub battery: Option<BatteryIndicator>,
    pub show_battery_percentage: bool,
    pub focus: Option<FocusIndicator>,
    pub notifications: Option<NotificationIndicator>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Compositor(rmac_compositor::Event),
    Network(rmac_network::NetworkSnapshot),
    Wifi(rmac_network::WifiSnapshot),
    Vpn(rmac_network::VpnSnapshot),
    Bluetooth(rmac_bluetooth::Snapshot),
    Audio(rmac_audio::Snapshot),
    Power(rmac_power::Snapshot),
    Settings(rmac_shell_settings::ShellSettings),
    Notifications(NotificationIndicator),
    Focus(Option<FocusIndicator>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Change {
    /// True only when a consumer-visible projection changed.
    pub visible: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    compositor: rmac_compositor::State,
    network: rmac_network::NetworkSnapshot,
    wifi: rmac_network::WifiSnapshot,
    vpn: rmac_network::VpnSnapshot,
    bluetooth: rmac_bluetooth::Snapshot,
    audio: rmac_audio::Snapshot,
    power: rmac_power::Snapshot,
    settings: rmac_shell_settings::ShellSettings,
    notifications: NotificationIndicator,
    focus: Option<FocusIndicator>,
}

impl State {
    pub fn snapshot(&self) -> Snapshot {
        let indicators = &self.settings.indicators;
        Snapshot {
            focused: self.focused_context(),
            network: indicators.network.then(|| self.network_indicator()),
            vpn: indicators.vpn.then(|| self.vpn_indicator()),
            bluetooth: indicators.bluetooth.then(|| BluetoothIndicator {
                available: self.bluetooth.available,
                powered: self.bluetooth.powered,
                connected_devices: self
                    .bluetooth
                    .devices
                    .iter()
                    .filter(|device| device.connected)
                    .count(),
            }),
            sound: indicators.sound.then_some(SoundIndicator {
                available: self.audio.available,
                volume: self.audio.output.volume,
                muted: self.audio.output.muted,
            }),
            battery: if indicators.power {
                self.power.battery.as_ref().map(|battery| BatteryIndicator {
                    percentage: battery.percentage,
                    state: battery.state,
                    on_battery: battery.on_battery,
                })
            } else {
                None
            },
            show_battery_percentage: indicators.power && indicators.battery_percentage,
            focus: indicators.focus.then(|| self.focus.clone()).flatten(),
            notifications: indicators.notifications.then_some(self.notifications),
        }
    }

    pub fn apply(&mut self, event: Event) -> Change {
        let before = self.snapshot();
        match event {
            Event::Compositor(event) => {
                self.compositor.apply(event);
            }
            Event::Network(snapshot) => self.network = snapshot,
            Event::Wifi(snapshot) => self.wifi = snapshot,
            Event::Vpn(snapshot) => self.vpn = snapshot,
            Event::Bluetooth(snapshot) => self.bluetooth = snapshot,
            Event::Audio(snapshot) => self.audio = snapshot,
            Event::Power(snapshot) => self.power = snapshot,
            Event::Settings(settings) => self.settings = settings,
            Event::Notifications(notifications) => self.notifications = notifications,
            Event::Focus(focus) => self.focus = focus,
        }
        Change {
            visible: before != self.snapshot(),
        }
    }

    fn focused_context(&self) -> FocusedContext {
        let focus = &self.compositor.focus;
        let workspace_id = focus.workspace.or(match focus.target {
            Some(FocusTarget::Workspace(id)) => Some(id),
            _ => None,
        });
        let window_id = focus.window.or(match focus.target {
            Some(FocusTarget::Window(id)) => Some(id),
            _ => None,
        });
        let workspace = workspace_id.and_then(|id| self.compositor.workspaces.get(&id));
        let window = window_id.and_then(|id| self.compositor.windows.get(&id));
        FocusedContext {
            output: focus
                .output
                .clone()
                .or_else(|| workspace.and_then(|workspace| workspace.output.clone())),
            workspace_id,
            workspace_label: workspace.map(|workspace| {
                workspace
                    .name
                    .clone()
                    .unwrap_or_else(|| workspace.index.to_string())
            }),
            window_id,
            app_id: window.and_then(|window| window.app_id.clone()),
            title: window.and_then(|window| window.title.clone()),
        }
    }

    fn network_indicator(&self) -> NetworkIndicator {
        let wifi_connected = self.wifi.current_ssid.is_some()
            || self.wifi.networks.iter().any(|network| network.connected);
        let state = if self.network.available {
            match self.network.connectivity {
                rmac_network::Connectivity::None => NetworkState::Disconnected,
                rmac_network::Connectivity::Portal => NetworkState::Portal,
                rmac_network::Connectivity::Limited => NetworkState::Limited,
                rmac_network::Connectivity::Full => NetworkState::Connected,
                rmac_network::Connectivity::Unknown if wifi_connected => NetworkState::Connected,
                rmac_network::Connectivity::Unknown => NetworkState::Unknown,
            }
        } else if self.wifi.available {
            if wifi_connected {
                NetworkState::Connected
            } else {
                NetworkState::Disconnected
            }
        } else {
            NetworkState::Unavailable
        };
        let wifi_strength = self
            .wifi
            .networks
            .iter()
            .find(|network| network.connected)
            .map(|network| network.strength.min(100));
        NetworkIndicator {
            state,
            connection_name: self
                .network
                .primary_connection
                .clone()
                .or_else(|| self.wifi.current_ssid.clone()),
            wifi_strength,
        }
    }

    fn vpn_indicator(&self) -> VpnIndicator {
        let mut active_names = self
            .vpn
            .profiles
            .iter()
            .filter(|profile| profile.state == rmac_network::VpnState::Connected)
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>();
        active_names.sort();
        active_names.dedup();
        VpnIndicator {
            active_names,
            transitioning: self.vpn.profiles.iter().any(|profile| {
                matches!(
                    profile.state,
                    rmac_network::VpnState::Connecting | rmac_network::VpnState::Disconnecting
                )
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_compositor::{
        FocusState, LogicalPoint, LogicalSize, PhysicalSize, Window, WindowLayout, Workspace,
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
    fn connected_wifi_and_vpn_are_normalized_for_compact_consumers() {
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
        state.apply(Event::Vpn(rmac_network::VpnSnapshot {
            available: true,
            profiles: vec![rmac_network::VpnProfile {
                identifier: "work".into(),
                name: "Work".into(),
                service: "wireguard".into(),
                state: rmac_network::VpnState::Connected,
            }],
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
        assert_eq!(snapshot.vpn.unwrap().active_names, vec!["Work"]);
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
}
