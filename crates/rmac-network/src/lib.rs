//! Cross-platform Wi-Fi state and radio control.

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashMap;
use std::fmt;
#[cfg(target_os = "macos")]
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiNetwork {
    pub ssid: String,
    pub strength: u8,
    pub secure: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WifiSnapshot {
    pub available: bool,
    pub enabled: bool,
    pub interface: Option<String>,
    pub current_ssid: Option<String>,
    pub networks: Vec<WifiNetwork>,
}

#[derive(Debug)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

pub trait WifiService {
    fn snapshot(&self) -> Result<WifiSnapshot, Error>;
    fn set_enabled(&self, enabled: bool) -> Result<(), Error>;
    fn request_scan(&self) -> Result<(), Error>;
}

pub struct SystemWifiService;

pub fn snapshot() -> Result<WifiSnapshot, Error> {
    SystemWifiService.snapshot()
}

pub fn set_enabled(enabled: bool) -> Result<(), Error> {
    SystemWifiService.set_enabled(enabled)
}

pub fn request_scan() -> Result<(), Error> {
    SystemWifiService.request_scan()
}

#[cfg(not(target_os = "macos"))]
impl WifiService for SystemWifiService {
    fn snapshot(&self) -> Result<WifiSnapshot, Error> {
        linux_snapshot()
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), Error> {
        let connection = system_connection("connect to NetworkManager")?;
        let proxy = manager_proxy(&connection)?;
        proxy
            .set_property("WirelessEnabled", enabled)
            .map_err(|error| Error::new("change Wi-Fi power", error.to_string()))
    }

    fn request_scan(&self) -> Result<(), Error> {
        let connection = system_connection("connect to NetworkManager")?;
        let Some(device) = wifi_device_path(&connection)? else {
            return Err(Error::new(
                "scan for Wi-Fi networks",
                "no Wi-Fi adapter found",
            ));
        };
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.NetworkManager",
            device.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
        )
        .map_err(|error| Error::new("open Wi-Fi adapter", error.to_string()))?;
        let options = HashMap::<String, zbus::zvariant::OwnedValue>::new();
        proxy
            .call::<_, _, ()>("RequestScan", &(options,))
            .map_err(|error| Error::new("scan for Wi-Fi networks", error.to_string()))
    }
}

#[cfg(not(target_os = "macos"))]
fn linux_snapshot() -> Result<WifiSnapshot, Error> {
    use zbus::zvariant::OwnedObjectPath;

    let connection = system_connection("connect to NetworkManager")?;
    let manager = manager_proxy(&connection)?;
    let enabled = manager
        .get_property("WirelessEnabled")
        .map_err(|error| Error::new("read Wi-Fi power", error.to_string()))?;
    let Some(device) = wifi_device_path(&connection)? else {
        return Ok(WifiSnapshot {
            available: false,
            enabled,
            ..WifiSnapshot::default()
        });
    };
    let device_proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.NetworkManager",
        device.as_str(),
        "org.freedesktop.NetworkManager.Device",
    )
    .map_err(|error| Error::new("open Wi-Fi device", error.to_string()))?;
    let interface = device_proxy
        .get_property::<String>("Interface")
        .map_err(|error| Error::new("read Wi-Fi interface", error.to_string()))?;
    let wireless = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.NetworkManager",
        device.as_str(),
        "org.freedesktop.NetworkManager.Device.Wireless",
    )
    .map_err(|error| Error::new("open Wi-Fi adapter", error.to_string()))?;
    let active = wireless
        .get_property::<OwnedObjectPath>("ActiveAccessPoint")
        .map_err(|error| Error::new("read active Wi-Fi network", error.to_string()))?;
    let access_points = wireless
        .call::<_, _, Vec<OwnedObjectPath>>("GetAllAccessPoints", &())
        .map_err(|error| Error::new("list Wi-Fi networks", error.to_string()))?;
    let mut raw = Vec::new();
    for path in access_points {
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.NetworkManager",
            path.as_str(),
            "org.freedesktop.NetworkManager.AccessPoint",
        )
        .map_err(|error| Error::new("open Wi-Fi network", error.to_string()))?;
        let ssid = proxy
            .get_property::<Vec<u8>>("Ssid")
            .map_err(|error| Error::new("read Wi-Fi network name", error.to_string()))?;
        let strength = proxy
            .get_property::<u8>("Strength")
            .map_err(|error| Error::new("read Wi-Fi signal", error.to_string()))?;
        let flags = proxy
            .get_property::<u32>("Flags")
            .map_err(|error| Error::new("read Wi-Fi security", error.to_string()))?;
        let wpa = proxy
            .get_property::<u32>("WpaFlags")
            .map_err(|error| Error::new("read Wi-Fi security", error.to_string()))?;
        let rsn = proxy
            .get_property::<u32>("RsnFlags")
            .map_err(|error| Error::new("read Wi-Fi security", error.to_string()))?;
        raw.push(RawNetwork {
            ssid,
            strength,
            secure: flags & 0x1 != 0 || wpa != 0 || rsn != 0,
            connected: path == active,
        });
    }
    let networks = normalize_networks(raw);
    let current_ssid = networks
        .iter()
        .find(|network| network.connected)
        .map(|network| network.ssid.clone());
    Ok(WifiSnapshot {
        available: true,
        enabled,
        interface: Some(interface),
        current_ssid,
        networks,
    })
}

#[cfg(not(target_os = "macos"))]
fn system_connection(operation: &'static str) -> Result<zbus::blocking::Connection, Error> {
    zbus::blocking::Connection::system().map_err(|error| Error::new(operation, error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn manager_proxy(
    connection: &zbus::blocking::Connection,
) -> Result<zbus::blocking::Proxy<'_>, Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .map_err(|error| Error::new("open NetworkManager", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn wifi_device_path(
    connection: &zbus::blocking::Connection,
) -> Result<Option<zbus::zvariant::OwnedObjectPath>, Error> {
    let devices = manager_proxy(connection)?
        .call::<_, _, Vec<zbus::zvariant::OwnedObjectPath>>("GetDevices", &())
        .map_err(|error| Error::new("list network devices", error.to_string()))?;
    for path in devices {
        let proxy = zbus::blocking::Proxy::new(
            connection,
            "org.freedesktop.NetworkManager",
            path.as_str(),
            "org.freedesktop.NetworkManager.Device",
        )
        .map_err(|error| Error::new("open network device", error.to_string()))?;
        let device_type = proxy
            .get_property::<u32>("DeviceType")
            .map_err(|error| Error::new("read network device type", error.to_string()))?;
        if device_type == 2 {
            drop(proxy);
            return Ok(Some(path));
        }
    }
    Ok(None)
}

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
        let networks = current_ssid
            .iter()
            .map(|ssid| WifiNetwork {
                ssid: ssid.clone(),
                strength: 100,
                secure: true,
                connected: true,
            })
            .collect();
        Ok(WifiSnapshot {
            available: true,
            enabled,
            interface: Some(device),
            current_ssid,
            networks,
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
}

#[cfg(target_os = "macos")]
fn macos_wifi_device() -> Result<Option<String>, Error> {
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
fn command(program: &'static str, arguments: &[&str]) -> Result<String, Error> {
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

#[cfg(any(not(target_os = "macos"), test))]
struct RawNetwork {
    ssid: Vec<u8>,
    strength: u8,
    secure: bool,
    connected: bool,
}

#[cfg(any(not(target_os = "macos"), test))]
fn normalize_networks(network_data: Vec<RawNetwork>) -> Vec<WifiNetwork> {
    let mut networks = HashMap::<String, WifiNetwork>::new();
    for raw in network_data {
        let ssid = String::from_utf8_lossy(&raw.ssid).trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let network = WifiNetwork {
            ssid: ssid.clone(),
            strength: raw.strength.min(100),
            secure: raw.secure,
            connected: raw.connected,
        };
        networks
            .entry(ssid)
            .and_modify(|existing| {
                let secure = existing.secure || network.secure;
                if !existing.connected
                    && (network.connected || network.strength > existing.strength)
                {
                    *existing = network.clone();
                }
                existing.secure = secure;
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
    });
    networks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks_are_deduplicated_sorted_and_clamped() {
        let networks = normalize_networks(vec![
            RawNetwork {
                ssid: b"Cafe".to_vec(),
                strength: 45,
                secure: false,
                connected: false,
            },
            RawNetwork {
                ssid: b"Home".to_vec(),
                strength: 150,
                secure: true,
                connected: true,
            },
            RawNetwork {
                ssid: b"Cafe".to_vec(),
                strength: 72,
                secure: true,
                connected: false,
            },
            RawNetwork {
                ssid: b"Home".to_vec(),
                strength: 200,
                secure: false,
                connected: false,
            },
            RawNetwork {
                ssid: Vec::new(),
                strength: 99,
                secure: false,
                connected: false,
            },
        ]);

        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].ssid, "Home");
        assert_eq!(networks[0].strength, 100);
        assert_eq!(networks[1].ssid, "Cafe");
        assert_eq!(networks[1].strength, 72);
        assert!(networks[1].secure);
    }

    #[test]
    fn errors_preserve_operation_context() {
        let error = Error::new("read Wi-Fi state", "service unavailable");
        assert_eq!(
            error.to_string(),
            "could not read Wi-Fi state: service unavailable"
        );
    }
}
