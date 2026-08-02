//! macOS development backend for Wi-Fi, network, and VPN state.

use super::*;

#[cfg(target_os = "macos")]
impl WifiService for SystemWifiService {
    fn snapshot(&self) -> Result<WifiSnapshot, Error> {
        let Some(device) = macos_wifi_device()? else {
            return Ok(WifiSnapshot::default());
        };
        let power = command("networksetup", &["-getairportpower", &device])?;
        let enabled = power.to_ascii_lowercase().ends_with("on");
        let current_ssid = if enabled {
            command("networksetup", &["-getairportnetwork", &device])
                .ok()
                .and_then(|output| {
                    output
                        .split_once(':')
                        .map(|(_, value)| value.trim().to_string())
                })
                .filter(|ssid| !ssid.is_empty() && !ssid.contains("not associated"))
        } else {
            None
        };
        let networks: Vec<WifiNetwork> = current_ssid
            .iter()
            .filter_map(|ssid| {
                let id = WifiNetworkId::from_bytes(
                    ssid.as_bytes().to_vec(),
                    WifiSecurity::Personal(WifiPersonalMode::Psk),
                )?;
                Some(WifiNetwork {
                    id,
                    ssid: ssid.clone(),
                    strength: 100,
                    security: WifiSecurity::Personal(WifiPersonalMode::Psk),
                    known: true,
                    connected: true,
                })
            })
            .collect();
        let saved_networks = networks
            .iter()
            .map(|network| WifiSavedNetwork {
                id: network.id.clone(),
                ssid: network.ssid.clone(),
            })
            .collect();
        Ok(WifiSnapshot {
            available: true,
            enabled,
            interface: Some(device),
            current_ssid,
            networks,
            saved_networks,
        })
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), Error> {
        let device = macos_wifi_device()?
            .ok_or_else(|| Error::new("change Wi-Fi power", "no Wi-Fi adapter found"))?;
        command(
            "networksetup",
            &[
                "-setairportpower",
                &device,
                if enabled { "on" } else { "off" },
            ],
        )?;
        Ok(())
    }

    fn request_scan(&self) -> Result<(), Error> {
        Ok(())
    }

    fn connect(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
        let snapshot = self.snapshot()?;
        if snapshot
            .networks
            .iter()
            .any(|candidate| candidate.id == *network && candidate.connected)
        {
            return Ok(snapshot);
        }
        Err(Error::new(
            "connect Wi-Fi",
            "network activation is provided by the Linux NetworkManager session",
        ))
    }

    fn forget(&self, _network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
        Err(Error::new(
            "forget Wi-Fi network",
            "saved-network removal is provided by the Linux NetworkManager session",
        ))
    }

    fn connect_with_password(
        &self,
        _network: &WifiNetworkId,
        _password: WifiPassword,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error> {
        if cancellation.is_cancelled() {
            return Err(Error::cancelled("connect protected Wi-Fi"));
        }
        Err(Error::new(
            "connect protected Wi-Fi",
            "password activation is provided by the Linux NetworkManager session",
        ))
    }

    fn connect_enterprise(
        &self,
        _network: &WifiNetworkId,
        _credentials: WifiEnterpriseCredentials,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error> {
        if cancellation.is_cancelled() {
            return Err(Error::cancelled("connect enterprise Wi-Fi"));
        }
        Err(Error::new(
            "connect enterprise Wi-Fi",
            "enterprise activation is provided by the Linux NetworkManager session",
        ))
    }
}

#[cfg(target_os = "macos")]
pub(super) fn macos_network_snapshot() -> Result<NetworkSnapshot, Error> {
    let default_route = network_command("route", &["-n", "get", "default"])?;
    let route_field = |key: &str| {
        default_route.lines().find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .map(|value| value.trim().to_string())
        })
    };
    let interface = route_field("interface:").unwrap_or_default();
    if interface.is_empty() {
        return Ok(NetworkSnapshot {
            available: true,
            connectivity: Connectivity::None,
            ..NetworkSnapshot::default()
        });
    }

    let ports = network_command("networksetup", &["-listallhardwareports"])?;
    let mut service = "Network".to_string();
    let mut hardware_address = None;
    for block in ports.split("Hardware Port:") {
        if block
            .lines()
            .any(|line| line.trim() == format!("Device: {interface}"))
        {
            if let Some(name) = block.lines().next() {
                service = name.trim().to_string();
            }
            hardware_address = block.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("Ethernet Address:")
                    .map(|value| value.trim().to_string())
            });
            break;
        }
    }
    let address = network_command("ipconfig", &["getifaddr", &interface]).ok();
    let dns = network_command("scutil", &["--dns"])
        .ok()
        .into_iter()
        .flat_map(|output| {
            output
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix("nameserver[")
                        .and_then(|line| line.split_once(':'))
                        .map(|(_, address)| address.trim().to_string())
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let state = if address.is_some() {
        DeviceState::Connected
    } else {
        DeviceState::Disconnected
    };
    let kind = if service == "Wi-Fi" {
        DeviceKind::WiFi
    } else if service.to_ascii_lowercase().contains("ethernet") {
        DeviceKind::Ethernet
    } else {
        DeviceKind::Other
    };
    Ok(NetworkSnapshot {
        available: true,
        connectivity: if state.is_connected() {
            Connectivity::Full
        } else {
            Connectivity::None
        },
        primary_connection: Some(service.clone()),
        devices: vec![NetworkDevice {
            interface,
            kind,
            state,
            connection: Some(service),
            primary: true,
            addresses: address.into_iter().collect(),
            gateway: route_field("gateway:"),
            dns,
            hardware_address,
            configuration: None,
            configuration_error: Some(
                "Connection editing is available on the Linux product target".into(),
            ),
        }],
    })
}

#[cfg(target_os = "macos")]
pub(super) fn macos_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    let output = network_command("scutil", &["--nc", "list"])?;
    Ok(VpnSnapshot {
        available: true,
        profiles: parse_macos_vpn_profiles(&output),
    })
}

#[cfg(target_os = "macos")]
pub(super) fn macos_set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
) -> Result<VpnSnapshot, Error> {
    network_command(
        "scutil",
        &["--nc", if enabled { "start" } else { "stop" }, &id.uuid],
    )?;
    macos_vpn_snapshot()
}

#[cfg(target_os = "macos")]
pub(super) fn network_command(program: &'static str, arguments: &[&str]) -> Result<String, Error> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| Error::new("start network helper", error.to_string()))?;
    if !output.status.success() {
        return Err(Error::new(
            "run network helper",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "macos")]
pub(super) fn macos_wifi_device() -> Result<Option<String>, Error> {
    let output = command("networksetup", &["-listallhardwareports"])?;
    let mut wifi = false;
    for line in output.lines() {
        if line.trim_start().starts_with("Hardware Port:") {
            wifi = line.contains("Wi-Fi");
        } else if wifi {
            if let Some(device) = line.trim_start().strip_prefix("Device:") {
                return Ok(Some(device.trim().to_string()));
            }
        }
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
pub(super) fn command(program: &'static str, arguments: &[&str]) -> Result<String, Error> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| Error::new("start Wi-Fi helper", error.to_string()))?;
    if !output.status.success() {
        return Err(Error::new(
            "run Wi-Fi helper",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
