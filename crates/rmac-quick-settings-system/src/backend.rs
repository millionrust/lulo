pub trait Backend {
    fn set_wifi_enabled(&self, enabled: bool) -> Result<(), String>;
    fn join_wifi(&self, network: &rmac_network::WifiNetworkId) -> Result<(), String>;
    fn wifi(&self) -> Result<rmac_network::WifiSnapshot, String>;

    fn set_bluetooth_powered(&self, powered: bool) -> Result<(), String>;
    fn set_bluetooth_device_connected(&self, device: &str, connected: bool) -> Result<(), String>;
    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String>;

    fn set_output_volume(&self, volume: u8) -> Result<(), String>;
    fn set_output_muted(&self, muted: bool) -> Result<(), String>;
    fn set_default_output(&self, device: &str) -> Result<(), String>;
    fn audio(&self) -> Result<rmac_audio::Snapshot, String>;

    fn set_power_profile(&self, profile: rmac_power::PowerProfile) -> Result<(), String>;
    fn power(&self) -> Result<rmac_power::Snapshot, String>;

    fn set_focus_enabled(&self, enabled: bool) -> Result<(), String>;
    fn focus(&self) -> Result<rmac_shell_settings::FocusSettings, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackend;

impl Backend for SystemBackend {
    fn set_wifi_enabled(&self, enabled: bool) -> Result<(), String> {
        rmac_network::set_enabled(enabled).map_err(|error| error.to_string())
    }

    fn wifi(&self) -> Result<rmac_network::WifiSnapshot, String> {
        rmac_network::snapshot().map_err(|error| error.to_string())
    }

    fn join_wifi(&self, network: &rmac_network::WifiNetworkId) -> Result<(), String> {
        rmac_network::connect(network)
            .map(drop)
            .map_err(|error| error.to_string())
    }

    fn set_bluetooth_powered(&self, powered: bool) -> Result<(), String> {
        rmac_bluetooth::set_powered(powered).map_err(|error| error.to_string())
    }

    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
        rmac_bluetooth::snapshot().map_err(|error| error.to_string())
    }

    fn set_bluetooth_device_connected(&self, device: &str, connected: bool) -> Result<(), String> {
        rmac_bluetooth::set_connected(device, connected).map_err(|error| error.to_string())
    }

    fn set_output_volume(&self, volume: u8) -> Result<(), String> {
        rmac_audio::set_volume(rmac_audio::DeviceKind::Output, volume)
            .map_err(|error| error.to_string())
    }

    fn set_output_muted(&self, muted: bool) -> Result<(), String> {
        rmac_audio::set_muted(rmac_audio::DeviceKind::Output, muted)
            .map_err(|error| error.to_string())
    }

    fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
        rmac_audio::snapshot().map_err(|error| error.to_string())
    }

    fn set_default_output(&self, device: &str) -> Result<(), String> {
        // Device ids name nodes in one sample of the audio graph, so the
        // switch uses a fresh sample's entry for the same node.
        let snapshot = rmac_audio::snapshot().map_err(|error| error.to_string())?;
        let output = snapshot
            .outputs
            .iter()
            .find(|output| output.id == device)
            .ok_or_else(|| "the output is no longer connected".to_owned())?;
        rmac_audio::set_default_device(rmac_audio::DeviceKind::Output, output)
            .map(drop)
            .map_err(|error| error.to_string())
    }

    fn set_power_profile(&self, profile: rmac_power::PowerProfile) -> Result<(), String> {
        rmac_power::set_profile(profile).map_err(|error| error.to_string())
    }

    fn power(&self) -> Result<rmac_power::Snapshot, String> {
        rmac_power::snapshot().map_err(|error| error.to_string())
    }

    fn set_focus_enabled(&self, enabled: bool) -> Result<(), String> {
        rmac_focus_linux::client::set_enabled(enabled)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn focus(&self) -> Result<rmac_shell_settings::FocusSettings, String> {
        rmac_focus_linux::client::state()
            .map(|snapshot| rmac_shell_settings::FocusSettings {
                enabled: snapshot.projection.enabled,
                selected_mode: snapshot.projection.mode_name,
                ends_at_unix_ms: snapshot.projection.ends_at_unix_ms,
            })
            .map_err(|error| error.to_string())
    }
}
