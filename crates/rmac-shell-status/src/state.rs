use rmac_compositor::FocusTarget;

use crate::{
    BatteryIndicator, BluetoothIndicator, Change, Event, FocusIndicator, FocusedContext,
    NetworkIndicator, NetworkState, NotificationIndicator, OutputContext, Snapshot, SoundIndicator,
    VpnIndicator,
};

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
            outputs: self.output_contexts(),
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
            clock: self.settings.clock.clone(),
        }
    }

    fn output_contexts(&self) -> Vec<OutputContext> {
        let mut outputs = self
            .compositor
            .outputs
            .values()
            .filter_map(|output| {
                let logical = output.logical.as_ref()?;
                (output.current_mode.is_some()
                    && logical.size.is_valid()
                    && logical.size.width > 0.0
                    && logical.size.height > 0.0
                    && logical.scale.is_finite()
                    && logical.scale > 0.0)
                    .then(|| OutputContext {
                        id: output.id.clone(),
                        logical_size: logical.size,
                        scale: logical.scale,
                    })
            })
            .collect::<Vec<_>>();
        outputs.sort_by(|left, right| left.id.cmp(&right.id));
        outputs
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
