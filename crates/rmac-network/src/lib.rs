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

/// NetworkManager's view of the host's overall reachability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Connectivity {
    None,
    Portal,
    Limited,
    Full,
    #[default]
    Unknown,
}

impl Connectivity {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Not Connected",
            Self::Portal => "Sign-in Required",
            Self::Limited => "Limited Connectivity",
            Self::Full => "Connected",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Ethernet,
    WiFi,
    Other,
}

impl DeviceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ethernet => "Ethernet",
            Self::WiFi => "Wi-Fi",
            Self::Other => "Network Interface",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Unavailable,
    Disconnected,
    Connecting,
    Connected,
    Deactivating,
    Failed,
    Unknown,
}

impl DeviceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Unavailable",
            Self::Disconnected => "Not Connected",
            Self::Connecting => "Connecting…",
            Self::Connected => "Connected",
            Self::Deactivating => "Disconnecting…",
            Self::Failed => "Connection Failed",
            Self::Unknown => "Unknown",
        }
    }

    pub fn is_connected(self) -> bool {
        self == Self::Connected
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkDevice {
    pub interface: String,
    pub kind: DeviceKind,
    pub state: DeviceState,
    pub connection: Option<String>,
    pub primary: bool,
    pub addresses: Vec<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    pub hardware_address: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkSnapshot {
    pub available: bool,
    pub connectivity: Connectivity,
    pub primary_connection: Option<String>,
    pub devices: Vec<NetworkDevice>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VpnState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Unknown,
}

impl VpnState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Not Connected",
            Self::Connecting => "Connecting…",
            Self::Connected => "Connected",
            Self::Disconnecting => "Disconnecting…",
            Self::Unknown => "Unknown",
        }
    }

    pub fn is_enabled(self) -> bool {
        matches!(
            self,
            Self::Connecting | Self::Connected | Self::Disconnecting
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VpnProfile {
    /// Stable, platform-owned identifier. Treat as opaque outside this crate.
    pub identifier: String,
    pub name: String,
    pub service: String,
    pub state: VpnState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VpnSnapshot {
    pub available: bool,
    pub profiles: Vec<VpnProfile>,
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

pub fn network_snapshot() -> Result<NetworkSnapshot, Error> {
    system_network_snapshot()
}

pub fn vpn_snapshot() -> Result<VpnSnapshot, Error> {
    system_vpn_snapshot()
}

pub fn set_vpn_enabled(identifier: &str, enabled: bool) -> Result<(), Error> {
    system_set_vpn_enabled(identifier, enabled)
}

#[cfg(not(target_os = "macos"))]
fn system_network_snapshot() -> Result<NetworkSnapshot, Error> {
    linux_network_snapshot()
}

#[cfg(target_os = "macos")]
fn system_network_snapshot() -> Result<NetworkSnapshot, Error> {
    macos_network_snapshot()
}

#[cfg(not(target_os = "macos"))]
fn system_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    linux_vpn_snapshot()
}

#[cfg(target_os = "macos")]
fn system_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    macos_vpn_snapshot()
}

#[cfg(not(target_os = "macos"))]
fn system_set_vpn_enabled(identifier: &str, enabled: bool) -> Result<(), Error> {
    linux_set_vpn_enabled(identifier, enabled)
}

#[cfg(target_os = "macos")]
fn system_set_vpn_enabled(identifier: &str, enabled: bool) -> Result<(), Error> {
    macos_set_vpn_enabled(identifier, enabled)
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
fn linux_network_snapshot() -> Result<NetworkSnapshot, Error> {
    use zbus::zvariant::OwnedObjectPath;

    let connection = system_connection("connect to NetworkManager")?;
    let manager = manager_proxy(&connection)?;
    let connectivity = manager
        .get_property::<u32>("Connectivity")
        .map(connectivity_from_network_manager)
        .unwrap_or_default();
    let primary_path = manager
        .get_property::<OwnedObjectPath>("PrimaryConnection")
        .ok();
    let device_paths = manager
        .call::<_, _, Vec<OwnedObjectPath>>("GetDevices", &())
        .map_err(|error| Error::new("list network devices", error.to_string()))?;

    let mut devices = Vec::new();
    for path in device_paths {
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.NetworkManager",
            path.as_str(),
            "org.freedesktop.NetworkManager.Device",
        )
        .map_err(|error| Error::new("open network device", error.to_string()))?;
        let interface = proxy
            .get_property::<String>("Interface")
            .map_err(|error| Error::new("read network interface", error.to_string()))?;
        let kind = match proxy
            .get_property::<u32>("DeviceType")
            .map_err(|error| Error::new("read network device type", error.to_string()))?
        {
            1 => DeviceKind::Ethernet,
            2 => DeviceKind::WiFi,
            _ => DeviceKind::Other,
        };
        let state = proxy
            .get_property::<u32>("State")
            .map(device_state_from_network_manager)
            .unwrap_or(DeviceState::Unknown);
        let active_path = proxy
            .get_property::<OwnedObjectPath>("ActiveConnection")
            .ok();
        let primary = primary_path
            .as_ref()
            .zip(active_path.as_ref())
            .is_some_and(|(primary, active)| primary == active && active.as_str() != "/");
        let connection_name = active_path
            .as_ref()
            .filter(|active| active.as_str() != "/")
            .and_then(|active| active_connection_name(&connection, active));
        let hardware_address = proxy
            .get_property::<String>("HwAddress")
            .ok()
            .filter(|address| !address.is_empty());

        let mut addresses = Vec::new();
        let mut gateway = None;
        let mut dns = Vec::new();
        for (path_property, interface_name) in [
            ("Ip4Config", "org.freedesktop.NetworkManager.IP4Config"),
            ("Ip6Config", "org.freedesktop.NetworkManager.IP6Config"),
        ] {
            let Some(config_path) = proxy
                .get_property::<OwnedObjectPath>(path_property)
                .ok()
                .filter(|path| path.as_str() != "/")
            else {
                continue;
            };
            read_ip_configuration(
                &connection,
                &config_path,
                interface_name,
                &mut addresses,
                &mut gateway,
                &mut dns,
            );
        }
        addresses.sort();
        addresses.dedup();
        dns.sort();
        dns.dedup();
        devices.push(NetworkDevice {
            interface,
            kind,
            state,
            connection: connection_name,
            primary,
            addresses,
            gateway,
            dns,
            hardware_address,
        });
    }
    sort_devices(&mut devices);
    let primary_connection = devices
        .iter()
        .find(|device| device.primary)
        .and_then(|device| device.connection.clone());
    Ok(NetworkSnapshot {
        available: true,
        connectivity,
        primary_connection,
        devices,
    })
}

#[cfg(not(target_os = "macos"))]
struct VpnRecord {
    profile: VpnProfile,
    connection_path: zbus::zvariant::OwnedObjectPath,
    active_path: Option<zbus::zvariant::OwnedObjectPath>,
}

#[cfg(not(target_os = "macos"))]
fn linux_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    let connection = system_connection("connect to NetworkManager")?;
    let mut records = linux_vpn_records(&connection)?;
    let mut profiles = records
        .drain(..)
        .map(|record| record.profile)
        .collect::<Vec<_>>();
    sort_vpn_profiles(&mut profiles);
    Ok(VpnSnapshot {
        available: true,
        profiles,
    })
}

#[cfg(not(target_os = "macos"))]
fn linux_set_vpn_enabled(identifier: &str, enabled: bool) -> Result<(), Error> {
    use zbus::zvariant::OwnedObjectPath;

    let connection = system_connection("connect to NetworkManager")?;
    let record = linux_vpn_records(&connection)?
        .into_iter()
        .find(|record| record.profile.identifier == identifier)
        .ok_or_else(|| Error::new("find VPN profile", "the profile no longer exists"))?;
    let manager = manager_proxy(&connection)?;
    if enabled {
        if record.active_path.is_some() {
            return Ok(());
        }
        let root = OwnedObjectPath::try_from("/")
            .map_err(|error| Error::new("prepare VPN activation", error.to_string()))?;
        manager
            .call::<_, _, OwnedObjectPath>(
                "ActivateConnection",
                &(record.connection_path, root.clone(), root),
            )
            .map_err(|error| Error::new("connect VPN", error.to_string()))?;
    } else if let Some(active_path) = record.active_path {
        manager
            .call::<_, _, ()>("DeactivateConnection", &(active_path,))
            .map_err(|error| Error::new("disconnect VPN", error.to_string()))?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn linux_vpn_records(connection: &zbus::blocking::Connection) -> Result<Vec<VpnRecord>, Error> {
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    let manager = manager_proxy(connection)?;
    let active_paths = manager
        .get_property::<Vec<OwnedObjectPath>>("ActiveConnections")
        .map_err(|error| Error::new("list active connections", error.to_string()))?;
    let mut active = HashMap::<String, (VpnState, OwnedObjectPath)>::new();
    for path in active_paths {
        let proxy = zbus::blocking::Proxy::new(
            connection,
            "org.freedesktop.NetworkManager",
            path.as_str(),
            "org.freedesktop.NetworkManager.Connection.Active",
        )
        .map_err(|error| Error::new("open active connection", error.to_string()))?;
        let connection_type = proxy.get_property::<String>("Type").unwrap_or_default();
        let is_vpn = proxy
            .get_property::<bool>("Vpn")
            .unwrap_or_else(|_| is_vpn_connection_type(&connection_type));
        if !is_vpn && !is_vpn_connection_type(&connection_type) {
            continue;
        }
        let Some(identifier) = proxy
            .get_property::<String>("Uuid")
            .ok()
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let state = proxy
            .get_property::<u32>("State")
            .map(vpn_state_from_network_manager)
            .unwrap_or(VpnState::Connecting);
        drop(proxy);
        active.insert(identifier, (state, path));
    }

    let settings = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .map_err(|error| Error::new("open saved network connections", error.to_string()))?;
    let connection_paths = settings
        .call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &())
        .map_err(|error| Error::new("list saved network connections", error.to_string()))?;
    let mut records = Vec::new();
    for path in connection_paths {
        let proxy = zbus::blocking::Proxy::new(
            connection,
            "org.freedesktop.NetworkManager",
            path.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .map_err(|error| Error::new("open saved network connection", error.to_string()))?;
        let settings = proxy
            .call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &())
            .map_err(|error| Error::new("read saved network connection", error.to_string()))?;
        drop(proxy);
        let Some(connection_settings) = settings.get("connection") else {
            continue;
        };
        let Some(connection_type) = property_string(connection_settings, "type") else {
            continue;
        };
        if !is_vpn_connection_type(&connection_type) {
            continue;
        }
        let Some(identifier) = property_string(connection_settings, "uuid") else {
            continue;
        };
        let name = property_string(connection_settings, "id")
            .unwrap_or_else(|| "VPN Connection".to_string());
        let service_type = settings
            .get("vpn")
            .and_then(|vpn| property_string(vpn, "service-type"));
        let (state, active_path) = active
            .remove(&identifier)
            .map_or((VpnState::Disconnected, None), |(state, path)| {
                (state, Some(path))
            });
        records.push(VpnRecord {
            profile: VpnProfile {
                identifier,
                name,
                service: vpn_service_label(&connection_type, service_type.as_deref()),
                state,
            },
            connection_path: path,
            active_path,
        });
    }
    Ok(records)
}

#[cfg(not(target_os = "macos"))]
fn active_connection_name(
    connection: &zbus::blocking::Connection,
    path: &zbus::zvariant::OwnedObjectPath,
) -> Option<String> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        path.as_str(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )
    .ok()?
    .get_property::<String>("Id")
    .ok()
    .filter(|name| !name.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn read_ip_configuration(
    connection: &zbus::blocking::Connection,
    path: &zbus::zvariant::OwnedObjectPath,
    interface: &str,
    addresses: &mut Vec<String>,
    gateway: &mut Option<String>,
    dns: &mut Vec<String>,
) {
    let Ok(proxy) = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        path.as_str(),
        interface,
    ) else {
        return;
    };
    if let Ok(data) =
        proxy.get_property::<Vec<HashMap<String, zbus::zvariant::OwnedValue>>>("AddressData")
    {
        for address in data {
            let value = property_string(&address, "address");
            let prefix = property::<u32>(&address, "prefix");
            if let Some(value) = value {
                addresses.push(format_address(&value, prefix));
            }
        }
    }
    if gateway.is_none() {
        *gateway = proxy
            .get_property::<String>("Gateway")
            .ok()
            .filter(|value| !value.is_empty());
    }
    if let Ok(data) =
        proxy.get_property::<Vec<HashMap<String, zbus::zvariant::OwnedValue>>>("NameserverData")
    {
        dns.extend(
            data.iter()
                .filter_map(|server| property_string(server, "address")),
        );
    }
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

#[cfg(not(target_os = "macos"))]
fn property<T>(properties: &HashMap<String, zbus::zvariant::OwnedValue>, key: &str) -> Option<T>
where
    for<'a> T: TryFrom<&'a zbus::zvariant::OwnedValue>,
{
    properties
        .get(key)
        .and_then(|value| T::try_from(value).ok())
}

#[cfg(not(target_os = "macos"))]
fn property_string(
    properties: &HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<String> {
    properties
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
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
fn macos_network_snapshot() -> Result<NetworkSnapshot, Error> {
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
        }],
    })
}

#[cfg(target_os = "macos")]
fn macos_vpn_snapshot() -> Result<VpnSnapshot, Error> {
    let output = network_command("scutil", &["--nc", "list"])?;
    Ok(VpnSnapshot {
        available: true,
        profiles: parse_macos_vpn_profiles(&output),
    })
}

#[cfg(target_os = "macos")]
fn macos_set_vpn_enabled(identifier: &str, enabled: bool) -> Result<(), Error> {
    network_command(
        "scutil",
        &["--nc", if enabled { "start" } else { "stop" }, identifier],
    )?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn network_command(program: &'static str, arguments: &[&str]) -> Result<String, Error> {
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

#[cfg(any(not(target_os = "macos"), test))]
fn connectivity_from_network_manager(value: u32) -> Connectivity {
    match value {
        1 => Connectivity::None,
        2 => Connectivity::Portal,
        3 => Connectivity::Limited,
        4 => Connectivity::Full,
        _ => Connectivity::Unknown,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn device_state_from_network_manager(value: u32) -> DeviceState {
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
fn format_address(address: &str, prefix: Option<u32>) -> String {
    prefix.map_or_else(
        || address.to_string(),
        |prefix| format!("{address}/{prefix}"),
    )
}

#[cfg(any(not(target_os = "macos"), test))]
fn sort_devices(devices: &mut [NetworkDevice]) {
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
fn device_kind_order(kind: DeviceKind) -> u8 {
    match kind {
        DeviceKind::Ethernet => 0,
        DeviceKind::WiFi => 1,
        DeviceKind::Other => 2,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn is_vpn_connection_type(connection_type: &str) -> bool {
    matches!(connection_type, "vpn" | "wireguard")
}

#[cfg(any(not(target_os = "macos"), test))]
fn vpn_state_from_network_manager(value: u32) -> VpnState {
    match value {
        1 => VpnState::Connecting,
        2 => VpnState::Connected,
        3 => VpnState::Disconnecting,
        4 => VpnState::Disconnected,
        _ => VpnState::Unknown,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn vpn_service_label(connection_type: &str, service_type: Option<&str>) -> String {
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

fn sort_vpn_profiles(profiles: &mut [VpnProfile]) {
    profiles.sort_by(|left, right| {
        right
            .state
            .is_enabled()
            .cmp(&left.state.is_enabled())
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.identifier.cmp(&right.identifier))
    });
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_vpn_profiles(output: &str) -> Vec<VpnProfile> {
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
                identifier: name.to_string(),
                name: name.to_string(),
                service: service.to_string(),
                state,
            })
        })
        .collect::<Vec<_>>();
    sort_vpn_profiles(&mut profiles);
    profiles
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

    #[test]
    fn network_manager_values_map_to_stable_ui_states() {
        assert_eq!(connectivity_from_network_manager(4), Connectivity::Full);
        assert_eq!(connectivity_from_network_manager(99), Connectivity::Unknown);
        assert_eq!(
            device_state_from_network_manager(70),
            DeviceState::Connecting
        );
        assert_eq!(
            device_state_from_network_manager(100),
            DeviceState::Connected
        );
        assert_eq!(device_state_from_network_manager(120), DeviceState::Failed);
    }

    #[test]
    fn address_prefix_is_preserved_when_available() {
        assert_eq!(format_address("192.0.2.4", Some(24)), "192.0.2.4/24");
        assert_eq!(format_address("2001:db8::1", None), "2001:db8::1");
    }

    #[test]
    fn devices_are_sorted_by_primary_connection_and_state() {
        let make = |interface: &str, kind, state, primary| NetworkDevice {
            interface: interface.to_string(),
            kind,
            state,
            connection: None,
            primary,
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
            hardware_address: None,
        };
        let mut devices = vec![
            make("wlan0", DeviceKind::WiFi, DeviceState::Connected, false),
            make(
                "eth1",
                DeviceKind::Ethernet,
                DeviceState::Disconnected,
                false,
            ),
            make("eth0", DeviceKind::Ethernet, DeviceState::Connected, true),
        ];
        sort_devices(&mut devices);
        assert_eq!(devices[0].interface, "eth0");
        assert_eq!(devices[1].interface, "wlan0");
        assert_eq!(devices[2].interface, "eth1");
    }

    #[test]
    fn vpn_types_states_and_services_are_normalized() {
        assert!(is_vpn_connection_type("vpn"));
        assert!(is_vpn_connection_type("wireguard"));
        assert!(!is_vpn_connection_type("802-3-ethernet"));
        assert_eq!(vpn_state_from_network_manager(1), VpnState::Connecting);
        assert_eq!(vpn_state_from_network_manager(2), VpnState::Connected);
        assert_eq!(
            vpn_service_label("vpn", Some("org.freedesktop.NetworkManager.openvpn")),
            "OpenVPN"
        );
        assert_eq!(vpn_service_label("wireguard", None), "WireGuard");
    }

    #[test]
    fn macos_vpn_fixture_is_parsed_and_active_profiles_sort_first() {
        let profiles = parse_macos_vpn_profiles(
            "Available network connection services in the current set (*=enabled):\n\
             * (Disconnected) 11111111-1111-1111-1111-111111111111 Office [VPN:IPSec]\n\
             * (Connected) 22222222-2222-2222-2222-222222222222 Home Tunnel [VPN:L2TP]",
        );
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "Home Tunnel");
        assert_eq!(profiles[0].state, VpnState::Connected);
        assert_eq!(profiles[0].service, "VPN:L2TP");
        assert_eq!(profiles[1].name, "Office");
    }
}
