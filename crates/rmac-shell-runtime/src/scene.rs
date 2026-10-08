//! The fixed status readings of the shared-view checks (ADR 0023, "Phase
//! 3 revised: shared shell views"): with `RMAC_SHELL_SCENE=1` the bar on
//! Lulo OS and on Windows draws the same Wi-Fi, battery and sound items,
//! whatever the machine running the check has, so the two screens can be
//! compared pixel for pixel.

use crate::runtime::ServiceReader;

/// Whether the fixed scene is asked for.
pub fn active() -> bool {
    std::env::var_os("RMAC_SHELL_SCENE").is_some_and(|value| value == "1")
}

/// Wi-Fi "Lulo" at full signal, the battery at 80 % on battery power,
/// sound at half volume, and no Bluetooth.
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneServiceReader;

impl ServiceReader for SceneServiceReader {
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
        let security = rmac_network::WifiSecurity::Protected;
        let network = rmac_network::WifiNetwork {
            id: rmac_network::WifiNetworkId::from_bytes(b"Lulo".to_vec(), security)
                .ok_or("scene network")?,
            ssid: "Lulo".into(),
            strength: 100,
            security,
            known: true,
            connected: true,
        };
        Ok((
            rmac_network::NetworkSnapshot::default(),
            rmac_network::WifiSnapshot {
                available: true,
                enabled: true,
                interface: None,
                current_ssid: Some("Lulo".into()),
                networks: vec![network],
                saved_networks: Vec::new(),
            },
            rmac_network::VpnSnapshot::default(),
        ))
    }

    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
        Ok(rmac_bluetooth::Snapshot::default())
    }

    fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
        Ok(rmac_audio::Snapshot {
            available: true,
            has_output: true,
            output: rmac_audio::Level {
                volume: 50,
                muted: false,
            },
            ..rmac_audio::Snapshot::default()
        })
    }

    fn power(&self) -> Result<rmac_power::Snapshot, String> {
        Ok(rmac_power::Snapshot {
            battery: Some(rmac_power::Battery {
                percentage: 80,
                state: rmac_power::BatteryState::Discharging,
                on_battery: true,
                seconds_remaining: None,
                capacity: None,
                charge_cycles: None,
                energy_rate_watts: None,
                model: None,
                charge_threshold: rmac_power::ChargeThreshold::default(),
                history: rmac_power::BatteryHistory::default(),
            }),
            profiles: rmac_power::Profiles::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scene_reads_the_same_everywhere() {
        let reader = SceneServiceReader;
        let (_, wifi, _) = reader.network().unwrap();
        assert_eq!(wifi.current_ssid.as_deref(), Some("Lulo"));
        assert_eq!(reader.power().unwrap().battery.unwrap().percentage, 80);
        assert_eq!(reader.audio().unwrap().output.volume, 50);
    }
}
