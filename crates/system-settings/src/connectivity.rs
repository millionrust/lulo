//! Interaction state shared by the Wi-Fi, Bluetooth, Network, and VPN panes.

use gpui::{Entity, SharedString};
use rmac_ui::InputState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WifiJoinAction {
    Direct,
    PersonalPassword,
    EnterpriseSetup,
    Unavailable,
}

pub(super) fn wifi_join_action(network: &rmac_network::WifiNetwork) -> WifiJoinAction {
    if !network.can_connect() {
        WifiJoinAction::Unavailable
    } else if network.needs_enterprise_setup() {
        WifiJoinAction::EnterpriseSetup
    } else if network.needs_password() {
        WifiJoinAction::PersonalPassword
    } else {
        WifiJoinAction::Direct
    }
}

pub(super) struct WifiPasswordPrompt {
    pub(super) network: rmac_network::WifiNetworkId,
    pub(super) ssid: SharedString,
    pub(super) editor: Entity<InputState>,
    pub(super) validation_error: Option<SharedString>,
}

pub(super) struct WifiEnterprisePrompt {
    pub(super) network: rmac_network::WifiNetworkId,
    pub(super) ssid: SharedString,
    pub(super) identity: Entity<InputState>,
    pub(super) anonymous_identity: Entity<InputState>,
    pub(super) certificate_domain: Entity<InputState>,
    pub(super) password: Entity<InputState>,
    pub(super) validation_error: Option<SharedString>,
}

pub(super) struct WifiForgetPrompt {
    pub(super) network: rmac_network::WifiNetworkId,
    pub(super) ssid: SharedString,
}

pub(super) enum BluetoothPairingDisplay {
    PinCode(String),
    Passkey { passkey: u32, entered: u16 },
}

pub(super) struct BluetoothPairingState {
    pub(super) device_id: String,
    pub(super) name: SharedString,
    pub(super) session: rmac_bluetooth::PairingSession,
    pub(super) prompt: Option<rmac_bluetooth::PairingPrompt>,
    pub(super) display: Option<BluetoothPairingDisplay>,
    pub(super) editor: Entity<InputState>,
    pub(super) validation_error: Option<SharedString>,
    pub(super) stopping: bool,
}

pub(super) struct BluetoothForgetPrompt {
    pub(super) device_id: String,
    pub(super) name: SharedString,
}

pub(super) struct NetworkEditorState {
    pub(super) interface: SharedString,
    pub(super) configuration: rmac_network::NetworkConfiguration,
    pub(super) ipv4_method: rmac_network::IpMethod,
    pub(super) ipv4_addresses: Entity<InputState>,
    pub(super) ipv4_gateway: Entity<InputState>,
    pub(super) ipv4_dns: Entity<InputState>,
    pub(super) ipv4_ignore_auto_dns: bool,
    pub(super) ipv6_method: rmac_network::IpMethod,
    pub(super) ipv6_addresses: Entity<InputState>,
    pub(super) ipv6_gateway: Entity<InputState>,
    pub(super) ipv6_dns: Entity<InputState>,
    pub(super) ipv6_ignore_auto_dns: bool,
    pub(super) proxy_method: rmac_network::ProxyMethod,
    pub(super) proxy_url: Entity<InputState>,
    pub(super) proxy_browser_only: bool,
    pub(super) validation_error: Option<SharedString>,
}

pub(super) struct VpnEditorState {
    pub(super) configuration: rmac_network::VpnProfileConfiguration,
    pub(super) name: Entity<InputState>,
    pub(super) username: Entity<InputState>,
    pub(super) timeout: Entity<InputState>,
    pub(super) persistent: bool,
    pub(super) validation_error: Option<SharedString>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_rows_route_each_security_authority_without_bypassing_enterprise_setup() {
        let network = |security, known, connected| rmac_network::WifiNetwork {
            id: rmac_network::WifiNetworkId::from_bytes(b"Network".to_vec(), security).unwrap(),
            ssid: "Network".into(),
            strength: 80,
            security,
            known,
            connected,
        };
        assert_eq!(
            wifi_join_action(&network(rmac_network::WifiSecurity::Open, false, false)),
            WifiJoinAction::Direct
        );
        assert_eq!(
            wifi_join_action(&network(
                rmac_network::WifiSecurity::Personal(rmac_network::WifiPersonalMode::Sae),
                false,
                false,
            )),
            WifiJoinAction::PersonalPassword
        );
        assert_eq!(
            wifi_join_action(&network(
                rmac_network::WifiSecurity::Enterprise,
                false,
                false,
            )),
            WifiJoinAction::EnterpriseSetup
        );
        assert_eq!(
            wifi_join_action(&network(
                rmac_network::WifiSecurity::Enterprise,
                true,
                false,
            )),
            WifiJoinAction::Direct
        );
        assert_eq!(
            wifi_join_action(&network(rmac_network::WifiSecurity::Legacy, false, false)),
            WifiJoinAction::Unavailable
        );
        assert_eq!(
            wifi_join_action(&network(rmac_network::WifiSecurity::Open, false, true)),
            WifiJoinAction::Unavailable
        );
    }
}
