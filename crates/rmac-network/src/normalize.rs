//! Platform-neutral network and VPN normalization helpers.

use super::*;

#[cfg(any(not(target_os = "macos"), test))]
pub(super) struct RawNetwork {
    pub(super) id: WifiNetworkId,
    pub(super) strength: u8,
    pub(super) known: bool,
    pub(super) connected: bool,
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn normalize_networks(network_data: Vec<RawNetwork>) -> Vec<WifiNetwork> {
    let mut networks = HashMap::<WifiNetworkId, WifiNetwork>::new();
    for raw in network_data {
        let ssid = display_ssid(&raw.id.ssid);
        let id = raw.id;
        let network = WifiNetwork {
            id: id.clone(),
            ssid,
            strength: raw.strength.min(100),
            security: id.security,
            known: raw.known,
            connected: raw.connected,
        };
        networks
            .entry(id)
            .and_modify(|existing| {
                let known = existing.known || network.known;
                if !existing.connected
                    && (network.connected || network.strength > existing.strength)
                {
                    *existing = network.clone();
                }
                existing.known = known;
            })
            .or_insert(network);
    }
    let mut networks = networks.into_values().collect::<Vec<_>>();
    networks.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.strength.cmp(&left.strength))
            .then_with(|| left.ssid.to_lowercase().cmp(&right.ssid.to_lowercase()))
            .then_with(|| right.security.is_secure().cmp(&left.security.is_secure()))
    });
    networks
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn normalize_saved_networks(
    profiles: impl IntoIterator<Item = (WifiNetworkId, u64)>,
) -> Vec<WifiSavedNetwork> {
    let mut latest = HashMap::<WifiNetworkId, u64>::new();
    for (id, timestamp) in profiles {
        latest
            .entry(id)
            .and_modify(|current| *current = (*current).max(timestamp))
            .or_insert(timestamp);
    }
    let mut saved = latest
        .into_iter()
        .map(|(id, timestamp)| {
            (
                WifiSavedNetwork {
                    ssid: display_ssid(&id.ssid),
                    id,
                },
                timestamp,
            )
        })
        .collect::<Vec<_>>();
    saved.sort_by(|(left, left_timestamp), (right, right_timestamp)| {
        right_timestamp
            .cmp(left_timestamp)
            .then_with(|| left.ssid.to_lowercase().cmp(&right.ssid.to_lowercase()))
            .then_with(|| left.id.security.cmp(&right.id.security))
    });
    saved.into_iter().map(|(network, _)| network).collect()
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn display_ssid(ssid: &[u8]) -> String {
    let display = String::from_utf8_lossy(ssid)
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect::<String>();
    if display.trim().is_empty() {
        "Unnamed Network".to_string()
    } else {
        display
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn connectivity_from_network_manager(value: u32) -> Connectivity {
    match value {
        1 => Connectivity::None,
        2 => Connectivity::Portal,
        3 => Connectivity::Limited,
        4 => Connectivity::Full,
        _ => Connectivity::Unknown,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn device_state_from_network_manager(value: u32) -> DeviceState {
    match value {
        20 | 30 => DeviceState::Unavailable,
        40 => DeviceState::Disconnected,
        50..=90 => DeviceState::Connecting,
        100 => DeviceState::Connected,
        110 => DeviceState::Deactivating,
        120 => DeviceState::Failed,
        _ => DeviceState::Unknown,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn format_address(address: &str, prefix: Option<u32>) -> String {
    prefix.map_or_else(
        || address.to_string(),
        |prefix| format!("{address}/{prefix}"),
    )
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn sort_devices(devices: &mut [NetworkDevice]) {
    devices.sort_by(|left, right| {
        right
            .primary
            .cmp(&left.primary)
            .then_with(|| right.state.is_connected().cmp(&left.state.is_connected()))
            .then_with(|| device_kind_order(left.kind).cmp(&device_kind_order(right.kind)))
            .then_with(|| left.interface.cmp(&right.interface))
    });
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn device_kind_order(kind: DeviceKind) -> u8 {
    match kind {
        DeviceKind::Ethernet => 0,
        DeviceKind::WiFi => 1,
        DeviceKind::Other => 2,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn is_vpn_connection_type(connection_type: &str) -> bool {
    matches!(connection_type, "vpn" | "wireguard")
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn vpn_state_from_network_manager(value: u32) -> VpnState {
    match value {
        1 => VpnState::Connecting,
        2 => VpnState::Connected,
        3 => VpnState::Disconnecting,
        4 => VpnState::Disconnected,
        _ => VpnState::Unknown,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn vpn_state_from_vpn_connection(value: u32) -> VpnState {
    match value {
        1..=4 => VpnState::Connecting,
        5 => VpnState::Connected,
        6 => VpnState::Failed,
        7 => VpnState::Disconnected,
        _ => VpnState::Unknown,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn vpn_service_label(connection_type: &str, service_type: Option<&str>) -> String {
    if connection_type == "wireguard" {
        return "WireGuard".to_string();
    }
    match service_type.and_then(|service| service.rsplit('.').next()) {
        Some("openvpn") => "OpenVPN".to_string(),
        Some("openconnect") => "OpenConnect".to_string(),
        Some("vpnc") => "Cisco VPN".to_string(),
        Some("pptp") => "PPTP".to_string(),
        Some("strongswan") | Some("libreswan") => "IPsec".to_string(),
        Some(service) if !service.is_empty() => service.to_string(),
        _ => "VPN".to_string(),
    }
}

pub(super) fn sort_vpn_profiles(profiles: &mut [VpnProfile]) {
    profiles.sort_by(|left, right| {
        right
            .state
            .is_enabled()
            .cmp(&left.state.is_enabled())
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.uuid.cmp(&right.id.uuid))
    });
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn parse_macos_vpn_profiles(output: &str) -> Vec<VpnProfile> {
    let mut profiles = output
        .lines()
        .filter_map(|line| {
            let line = line.trim().trim_start_matches('*').trim();
            let (raw_state, remainder) = line.strip_prefix('(')?.split_once(')')?;
            let remainder = remainder.trim();
            let (_, description) = remainder.split_once(char::is_whitespace)?;
            let (name, service) = description
                .rsplit_once(" [")
                .map(|(name, service)| (name.trim(), service.trim_end_matches(']')))
                .unwrap_or((description.trim(), "VPN"));
            if name.is_empty() {
                return None;
            }
            let state = match raw_state.trim() {
                "Connected" => VpnState::Connected,
                "Connecting" => VpnState::Connecting,
                "Disconnecting" => VpnState::Disconnecting,
                "Disconnected" => VpnState::Disconnected,
                _ => VpnState::Unknown,
            };
            Some(VpnProfile {
                id: VpnProfileId {
                    object_path: name.to_string(),
                    uuid: name.to_string(),
                },
                name: name.to_string(),
                service: service.to_string(),
                state,
            })
        })
        .collect::<Vec<_>>();
    sort_vpn_profiles(&mut profiles);
    profiles
}
