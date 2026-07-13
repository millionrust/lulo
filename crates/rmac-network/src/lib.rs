//! Cross-platform Wi-Fi state and radio control.

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashMap;
use std::fmt;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(not(target_os = "macos"))]
use std::time::Duration;
use zeroize::Zeroize as _;

mod network_editor;
#[cfg(any(not(target_os = "macos"), test))]
mod secret_agent;

pub use network_editor::{
    IpAddress, IpConfiguration, IpFamily, IpMethod, NetworkConfiguration, NetworkConnectionId,
    NetworkEdit, NetworkValidationError, ProxyConfiguration, ProxyMethod,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WifiPersonalMode {
    Psk,
    Sae,
    Transition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WifiSecurity {
    Open,
    EnhancedOpen,
    Personal(WifiPersonalMode),
    Enterprise,
    Legacy,
    Protected,
}

impl WifiSecurity {
    pub fn is_secure(self) -> bool {
        self != Self::Open
    }

    pub fn needs_password(self) -> bool {
        matches!(self, Self::Personal(_))
    }

    pub fn can_create(self) -> bool {
        matches!(self, Self::Open | Self::EnhancedOpen | Self::Personal(_))
    }
}

/// Exact NetworkManager identity for a visible Wi-Fi network.
///
/// SSIDs are byte arrays, not necessarily UTF-8. Keeping those bytes private
/// prevents callers from accidentally activating a lossy display string. The
/// security class is part of the identity so an open and protected AP using
/// the same visible name cannot be conflated.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct WifiNetworkId {
    ssid: Vec<u8>,
    security: WifiSecurity,
}

impl WifiNetworkId {
    pub fn from_bytes(ssid: impl Into<Vec<u8>>, security: WifiSecurity) -> Option<Self> {
        let ssid = ssid.into();
        (!ssid.is_empty() && ssid.len() <= 32).then_some(Self { ssid, security })
    }

    pub fn is_secure(&self) -> bool {
        self.security.is_secure()
    }

    pub fn security(&self) -> WifiSecurity {
        self.security
    }

    #[cfg(any(not(target_os = "macos"), test))]
    fn matches_profile(&self, profile: &Self) -> bool {
        self.ssid == profile.ssid
            && match (self.security, profile.security) {
                (
                    WifiSecurity::Personal(WifiPersonalMode::Transition),
                    WifiSecurity::Personal(_),
                ) => true,
                (left, right) => left == right,
            }
    }
}

impl fmt::Debug for WifiNetworkId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WifiNetworkId")
            .field("ssid_bytes", &self.ssid.len())
            .field("security", &self.security)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiNetwork {
    pub id: WifiNetworkId,
    pub ssid: String,
    pub strength: u8,
    pub security: WifiSecurity,
    pub known: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiSavedNetwork {
    pub id: WifiNetworkId,
    pub ssid: String,
}

impl WifiNetwork {
    pub fn can_connect(&self) -> bool {
        !self.connected && (self.known || self.security.can_create())
    }

    pub fn needs_password(&self) -> bool {
        !self.known && self.security.needs_password()
    }
}

pub struct WifiPassword {
    bytes: Vec<u8>,
}

impl WifiPassword {
    pub fn new(mut value: String, network: &WifiNetworkId) -> Result<Self, WifiPasswordError> {
        let valid = match network.security {
            WifiSecurity::Personal(WifiPersonalMode::Psk | WifiPersonalMode::Transition) => {
                let is_hex_key =
                    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
                is_hex_key
                    || ((8..=63).contains(&value.len())
                        && value.is_ascii()
                        && !value.chars().any(char::is_control))
            }
            WifiSecurity::Personal(WifiPersonalMode::Sae) => {
                (1..=63).contains(&value.len()) && !value.chars().any(char::is_control)
            }
            _ => {
                value.zeroize();
                return Err(WifiPasswordError::UnsupportedSecurity);
            }
        };
        if !valid {
            value.zeroize();
            return Err(WifiPasswordError::Invalid);
        }
        Ok(Self {
            bytes: value.into_bytes(),
        })
    }

    #[cfg(any(not(target_os = "macos"), test))]
    fn expose<R>(&self, use_password: impl FnOnce(&str) -> R) -> R {
        // Construction accepts only a valid String, so this can fail only if
        // memory was corrupted. Never substitute a partial or lossy password.
        let value = std::str::from_utf8(&self.bytes).expect("Wi-Fi password must remain UTF-8");
        use_password(value)
    }
}

impl fmt::Debug for WifiPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WifiPassword(<redacted>)")
    }
}

impl Drop for WifiPassword {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiPasswordError {
    Invalid,
    UnsupportedSecurity,
}

impl fmt::Display for WifiPasswordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "enter a valid Wi-Fi password",
            Self::UnsupportedSecurity => "this network needs advanced security configuration",
        })
    }
}

impl std::error::Error for WifiPasswordError {}

#[derive(Clone, Default)]
pub struct WifiCancellation(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl WifiCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WifiSnapshot {
    pub available: bool,
    pub enabled: bool,
    pub interface: Option<String>,
    pub current_ssid: Option<String>,
    pub networks: Vec<WifiNetwork>,
    pub saved_networks: Vec<WifiSavedNetwork>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiWatchEvent {
    Changed,
    Unavailable,
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
    pub configuration: Option<NetworkConfiguration>,
    pub configuration_error: Option<String>,
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
    Failed,
    Unknown,
}

impl VpnState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Not Connected",
            Self::Connecting => "Connecting…",
            Self::Connected => "Connected",
            Self::Disconnecting => "Disconnecting…",
            Self::Failed => "Connection Failed",
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

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct VpnProfileId {
    object_path: String,
    uuid: String,
}

impl fmt::Debug for VpnProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VpnProfileId")
            .field("object", &"<redacted>")
            .field("uuid", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Default)]
pub struct VpnCancellation(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl VpnCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VpnProfile {
    /// Stable, platform-owned identifier. Treat as opaque outside this crate.
    pub id: VpnProfileId,
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
    cancelled: bool,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
            cancelled: false,
        }
    }

    fn cancelled(operation: &'static str) -> Self {
        Self {
            operation,
            detail: "the operation was canceled".to_string(),
            cancelled: true,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
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
    fn connect(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error>;
    fn forget(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error>;
    fn connect_with_password(
        &self,
        network: &WifiNetworkId,
        password: WifiPassword,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error>;
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

pub fn connect(network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
    SystemWifiService.connect(network)
}

pub fn forget(network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
    SystemWifiService.forget(network)
}

pub async fn watch(sender: async_channel::Sender<WifiWatchEvent>) -> Result<(), Error> {
    system_watch_wifi(sender).await
}

pub fn connect_with_password(
    network: &WifiNetworkId,
    password: WifiPassword,
    cancellation: &WifiCancellation,
) -> Result<WifiSnapshot, Error> {
    SystemWifiService.connect_with_password(network, password, cancellation)
}

pub fn network_snapshot() -> Result<NetworkSnapshot, Error> {
    system_network_snapshot()
}

pub fn update_network_connection(edit: &NetworkEdit) -> Result<NetworkSnapshot, Error> {
    network_editor::system_update(edit)
}

pub fn vpn_snapshot() -> Result<VpnSnapshot, Error> {
    system_vpn_snapshot()
}

pub fn set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    system_set_vpn_enabled(id, enabled, cancellation)
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
fn system_set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    linux_set_vpn_enabled(id, enabled, cancellation)
}

#[cfg(target_os = "macos")]
fn system_set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    if enabled && cancellation.is_cancelled() {
        return Err(Error::cancelled("connect VPN"));
    }
    macos_set_vpn_enabled(id, enabled)
}

#[cfg(not(target_os = "macos"))]
const NETWORK_MANAGER_SERVICE: &str = "org.freedesktop.NetworkManager";
#[cfg(not(target_os = "macos"))]
const WIFI_WATCH_RECONNECT_DELAY: Duration = Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
const WIFI_WATCH_QUIET_PERIOD: Duration = Duration::from_millis(75);

#[cfg(not(target_os = "macos"))]
async fn system_watch_wifi(sender: async_channel::Sender<WifiWatchEvent>) -> Result<(), Error> {
    let mut unavailable_reported = false;
    loop {
        match watch_wifi_once(&sender, &mut unavailable_reported).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => {
                publish_wifi_unavailable(&sender, &mut unavailable_reported).await?;
            }
        }
        async_io::Timer::after(WIFI_WATCH_RECONNECT_DELAY).await;
    }
}

#[cfg(target_os = "macos")]
async fn system_watch_wifi(sender: async_channel::Sender<WifiWatchEvent>) -> Result<(), Error> {
    sender
        .send(WifiWatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch Wi-Fi changes", "the event consumer closed"))
}

#[cfg(not(target_os = "macos"))]
async fn watch_wifi_once(
    sender: &async_channel::Sender<WifiWatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system()
        .await
        .map_err(|error| Error::new("connect Wi-Fi event stream", error.to_string()))?;
    let network_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path_namespace("/org/freedesktop/NetworkManager")
        .map_err(|error| Error::new("build Wi-Fi signal filter", error.to_string()))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|error| Error::new("build Wi-Fi owner filter", error.to_string()))?
        .path("/org/freedesktop/DBus")
        .map_err(|error| Error::new("build Wi-Fi owner filter", error.to_string()))?
        .interface("org.freedesktop.DBus")
        .map_err(|error| Error::new("build Wi-Fi owner filter", error.to_string()))?
        .member("NameOwnerChanged")
        .map_err(|error| Error::new("build Wi-Fi owner filter", error.to_string()))?
        .add_arg(NETWORK_MANAGER_SERVICE)
        .map_err(|error| Error::new("build Wi-Fi owner filter", error.to_string()))?
        .build();
    let mut network = MessageStream::for_match_rule(network_rule, &connection, Some(64))
        .await
        .map_err(|error| Error::new("subscribe to Wi-Fi changes", error.to_string()))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(8))
        .await
        .map_err(|error| Error::new("subscribe to NetworkManager restarts", error.to_string()))?
        .fuse();

    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|error| Error::new("inspect NetworkManager service", error.to_string()))?;
    let service = zbus::names::BusName::try_from(NETWORK_MANAGER_SERVICE)
        .map_err(|error| Error::new("inspect NetworkManager service", error.to_string()))?;
    let mut available = dbus
        .name_has_owner(service)
        .await
        .map_err(|error| Error::new("inspect NetworkManager service", error.to_string()))?;
    if available {
        publish_wifi_changed(sender, unavailable_reported).await?;
    } else {
        publish_wifi_unavailable(sender, unavailable_reported).await?;
    }

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let signal = futures_util::select! {
            message = network.next() => {
                read_wifi_signal(message)?;
                Some(true)
            },
            message = owners.next() => read_network_manager_owner(message)?,
            _ = closed => return Ok(()),
        };
        let Some(signal_available) = signal else {
            continue;
        };
        if !signal_available {
            available = false;
            publish_wifi_unavailable(sender, unavailable_reported).await?;
            continue;
        }
        if !available {
            available = true;
        }
        let mut refresh_pending = true;

        loop {
            let quiet =
                futures_util::FutureExt::fuse(async_io::Timer::after(WIFI_WATCH_QUIET_PERIOD));
            let closed = sender.closed().fuse();
            futures_util::pin_mut!(quiet, closed);
            let signal = futures_util::select! {
                message = network.next() => {
                    read_wifi_signal(message)?;
                    Some(true)
                },
                message = owners.next() => read_network_manager_owner(message)?,
                _ = quiet => break,
                _ = closed => return Ok(()),
            };
            if let Some(signal_available) = signal {
                available = signal_available;
                refresh_pending = signal_available;
                if !signal_available {
                    publish_wifi_unavailable(sender, unavailable_reported).await?;
                }
            }
        }
        if available && refresh_pending {
            publish_wifi_changed(sender, unavailable_reported).await?;
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn read_wifi_signal(message: Option<Result<zbus::Message, zbus::Error>>) -> Result<(), Error> {
    match message {
        Some(Ok(_)) => Ok(()),
        Some(Err(error)) => Err(Error::new("read Wi-Fi change", error.to_string())),
        None => Err(Error::new("read Wi-Fi change", "the signal stream ended")),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_network_manager_owner(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<bool>, Error> {
    let message = message
        .ok_or_else(|| Error::new("read NetworkManager owner", "the signal stream ended"))?
        .map_err(|error| Error::new("read NetworkManager owner", error.to_string()))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|error| Error::new("read NetworkManager owner", error.to_string()))?;
    Ok(network_manager_owner_availability(&name, &new_owner))
}

#[cfg(any(not(target_os = "macos"), test))]
fn network_manager_owner_availability(name: &str, new_owner: &str) -> Option<bool> {
    (name == "org.freedesktop.NetworkManager").then_some(!new_owner.is_empty())
}

#[cfg(not(target_os = "macos"))]
async fn publish_wifi_changed(
    sender: &async_channel::Sender<WifiWatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if *unavailable_reported {
        sender
            .send(WifiWatchEvent::Changed)
            .await
            .map_err(|_| Error::new("publish Wi-Fi change", "the event consumer closed"))?;
    } else {
        let _ = sender.try_send(WifiWatchEvent::Changed);
    }
    *unavailable_reported = false;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn publish_wifi_unavailable(
    sender: &async_channel::Sender<WifiWatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if !*unavailable_reported {
        sender
            .send(WifiWatchEvent::Unavailable)
            .await
            .map_err(|_| Error::new("publish Wi-Fi outage", "the event consumer closed"))?;
        *unavailable_reported = true;
    }
    Ok(())
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

    fn connect(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
        linux_connect_wifi(network)
    }

    fn forget(&self, network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
        linux_forget_wifi(network)
    }

    fn connect_with_password(
        &self,
        network: &WifiNetworkId,
        password: WifiPassword,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error> {
        linux_connect_wifi_with_password(network, password, cancellation)
    }
}

#[cfg(not(target_os = "macos"))]
fn linux_snapshot() -> Result<WifiSnapshot, Error> {
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
    let profiles = linux_wifi_profiles(&connection)?;
    let saved_networks = normalize_saved_networks(
        profiles
            .iter()
            .map(|profile| (profile.id.clone(), profile.timestamp)),
    );
    let raw = linux_wifi_access_points(&connection, &device)?
        .into_iter()
        .map(|access_point| RawNetwork {
            known: profiles
                .iter()
                .any(|profile| access_point.id.matches_profile(&profile.id)),
            id: access_point.id,
            strength: access_point.strength,
            connected: access_point.connected,
        })
        .collect();
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
        saved_networks,
    })
}

#[cfg(not(target_os = "macos"))]
struct WifiAccessPointRecord {
    id: WifiNetworkId,
    path: zbus::zvariant::OwnedObjectPath,
    strength: u8,
    connected: bool,
}

#[cfg(not(target_os = "macos"))]
struct WifiProfileRecord {
    id: WifiNetworkId,
    connection_path: zbus::zvariant::OwnedObjectPath,
    timestamp: u64,
}

#[cfg(not(target_os = "macos"))]
fn linux_wifi_access_points(
    connection: &zbus::blocking::Connection,
    device: &zbus::zvariant::OwnedObjectPath,
) -> Result<Vec<WifiAccessPointRecord>, Error> {
    use zbus::zvariant::OwnedObjectPath;

    let wireless = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        device.as_str(),
        "org.freedesktop.NetworkManager.Device.Wireless",
    )
    .map_err(|error| Error::new("open Wi-Fi adapter", error.to_string()))?;
    let active = wireless
        .get_property::<OwnedObjectPath>("ActiveAccessPoint")
        .map_err(|error| Error::new("read active Wi-Fi network", error.to_string()))?;
    let paths = wireless
        .call::<_, _, Vec<OwnedObjectPath>>("GetAllAccessPoints", &())
        .map_err(|error| Error::new("list Wi-Fi networks", error.to_string()))?;
    let mut access_points = Vec::new();
    for path in paths {
        let proxy = zbus::blocking::Proxy::new(
            connection,
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
        drop(proxy);
        let security = wifi_security_from_access_point(flags, wpa, rsn);
        let Some(id) = WifiNetworkId::from_bytes(ssid, security) else {
            continue;
        };
        access_points.push(WifiAccessPointRecord {
            id,
            strength,
            connected: path == active,
            path,
        });
    }
    Ok(access_points)
}

#[cfg(not(target_os = "macos"))]
fn linux_wifi_profiles(
    connection: &zbus::blocking::Connection,
) -> Result<Vec<WifiProfileRecord>, Error> {
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    let settings = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .map_err(|error| Error::new("open saved Wi-Fi connections", error.to_string()))?;
    let paths = settings
        .call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &())
        .map_err(|error| Error::new("list saved Wi-Fi connections", error.to_string()))?;
    let mut profiles = Vec::new();
    for path in paths {
        let Ok(proxy) = zbus::blocking::Proxy::new(
            connection,
            "org.freedesktop.NetworkManager",
            path.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
        ) else {
            continue;
        };
        let Ok(settings) =
            proxy.call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &())
        else {
            // Profiles outside this user's permissions are not activatable and
            // must not make a visible AP appear to be a known network.
            continue;
        };
        drop(proxy);
        let Some((id, timestamp)) = wifi_profile_identity(&settings) else {
            continue;
        };
        profiles.push(WifiProfileRecord {
            id,
            connection_path: path,
            timestamp,
        });
    }
    profiles.sort_by(|left, right| {
        right.timestamp.cmp(&left.timestamp).then_with(|| {
            left.connection_path
                .as_str()
                .cmp(right.connection_path.as_str())
        })
    });
    Ok(profiles)
}

#[cfg(any(not(target_os = "macos"), test))]
fn wifi_profile_identity(
    settings: &HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>,
) -> Option<(WifiNetworkId, u64)> {
    let connection = settings.get("connection")?;
    (property_string(connection, "type").as_deref() == Some("802-11-wireless")).then_some(())?;
    let wireless = settings.get("802-11-wireless")?;
    let ssid = property_bytes(wireless, "ssid")?;
    let security = settings
        .get("802-11-wireless-security")
        .map_or(WifiSecurity::Open, wifi_security_from_profile);
    let id = WifiNetworkId::from_bytes(ssid, security)?;
    Some((id, property::<u64>(connection, "timestamp").unwrap_or(0)))
}

#[cfg(any(not(target_os = "macos"), test))]
fn wifi_security_from_access_point(flags: u32, wpa: u32, rsn: u32) -> WifiSecurity {
    const PSK: u32 = 0x0000_0100;
    const ENTERPRISE: u32 = 0x0000_0200;
    const SAE: u32 = 0x0000_0400;
    const OWE: u32 = 0x0000_0800;
    const OWE_TRANSITION: u32 = 0x0000_1000;
    const SUITE_B: u32 = 0x0000_2000;

    let security = wpa | rsn;
    let has_psk = security & PSK != 0;
    let has_sae = security & SAE != 0;
    if has_psk && has_sae {
        WifiSecurity::Personal(WifiPersonalMode::Transition)
    } else if has_psk {
        WifiSecurity::Personal(WifiPersonalMode::Psk)
    } else if has_sae {
        WifiSecurity::Personal(WifiPersonalMode::Sae)
    } else if security & (ENTERPRISE | SUITE_B) != 0 {
        WifiSecurity::Enterprise
    } else if security & (OWE | OWE_TRANSITION) != 0 {
        WifiSecurity::EnhancedOpen
    } else if flags & 0x1 != 0 {
        WifiSecurity::Legacy
    } else if security != 0 {
        WifiSecurity::Protected
    } else {
        WifiSecurity::Open
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn wifi_security_from_profile(
    security: &HashMap<String, zbus::zvariant::OwnedValue>,
) -> WifiSecurity {
    match property_string(security, "key-mgmt").as_deref() {
        Some("wpa-psk") => WifiSecurity::Personal(WifiPersonalMode::Psk),
        Some("sae") => WifiSecurity::Personal(WifiPersonalMode::Sae),
        Some("owe") => WifiSecurity::EnhancedOpen,
        Some("wpa-eap" | "wpa-eap-suite-b-192" | "ieee8021x") => WifiSecurity::Enterprise,
        Some("none") => WifiSecurity::Legacy,
        _ => WifiSecurity::Protected,
    }
}

#[cfg(not(target_os = "macos"))]
fn linux_connect_wifi(network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    let connection = system_connection("connect to NetworkManager")?;
    let manager = manager_proxy(&connection)?;
    let enabled = manager
        .get_property::<bool>("WirelessEnabled")
        .map_err(|error| Error::new("read Wi-Fi power", error.to_string()))?;
    if !enabled {
        return Err(Error::new("connect Wi-Fi", "Wi-Fi is turned off"));
    }
    let device = wifi_device_path(&connection)?
        .ok_or_else(|| Error::new("connect Wi-Fi", "no Wi-Fi adapter found"))?;
    let access_point = linux_wifi_access_points(&connection, &device)?
        .into_iter()
        .filter(|access_point| access_point.id == *network)
        .max_by_key(|access_point| (access_point.connected, access_point.strength))
        .ok_or_else(|| Error::new("connect Wi-Fi", "the network is no longer in range"))?;
    if access_point.connected {
        return linux_snapshot();
    }

    let profile = linux_wifi_profiles(&connection)?
        .into_iter()
        .find(|profile| network.matches_profile(&profile.id));
    let active_path = if let Some(profile) = profile {
        manager
            .call::<_, _, OwnedObjectPath>(
                "ActivateConnection",
                &(profile.connection_path, device.clone(), access_point.path),
            )
            .map_err(|error| Error::new("connect saved Wi-Fi network", error.to_string()))?
    } else {
        if !matches!(
            network.security,
            WifiSecurity::Open | WifiSecurity::EnhancedOpen
        ) {
            return Err(Error::new(
                "connect Wi-Fi",
                "this network needs security information",
            ));
        }
        let template = HashMap::<String, HashMap<String, OwnedValue>>::new();
        let (_, active_path) = manager
            .call::<_, _, (OwnedObjectPath, OwnedObjectPath)>(
                "AddAndActivateConnection",
                &(template, device.clone(), access_point.path),
            )
            .map_err(|error| Error::new("connect open Wi-Fi network", error.to_string()))?;
        active_path
    };
    wait_for_wifi_activation(&connection, &active_path, &device, network, None)
}

#[cfg(not(target_os = "macos"))]
fn linux_connect_wifi_with_password(
    network: &WifiNetworkId,
    password: WifiPassword,
    cancellation: &WifiCancellation,
) -> Result<WifiSnapshot, Error> {
    use zbus::zvariant::OwnedObjectPath;

    if !network.security.needs_password() {
        return Err(Error::new(
            "connect protected Wi-Fi",
            "the selected network does not use Wi-Fi Personal security",
        ));
    }
    if cancellation.is_cancelled() {
        return Err(Error::cancelled("connect protected Wi-Fi"));
    }
    let agent = secret_agent::RegisteredSecretAgent::register(network.clone(), password)
        .map_err(|error| Error::new("register Wi-Fi secret agent", error.to_string()))?;
    let connection = agent.connection();
    let manager = manager_proxy(connection)?;
    let enabled = manager
        .get_property::<bool>("WirelessEnabled")
        .map_err(|error| Error::new("read Wi-Fi power", error.to_string()))?;
    if !enabled {
        return Err(Error::new("connect protected Wi-Fi", "Wi-Fi is turned off"));
    }
    let device = wifi_device_path(connection)?
        .ok_or_else(|| Error::new("connect protected Wi-Fi", "no Wi-Fi adapter found"))?;
    let access_point = linux_wifi_access_points(connection, &device)?
        .into_iter()
        .filter(|access_point| access_point.id == *network)
        .max_by_key(|access_point| (access_point.connected, access_point.strength))
        .ok_or_else(|| {
            Error::new(
                "connect protected Wi-Fi",
                "the network is no longer in range",
            )
        })?;
    if access_point.connected {
        return linux_snapshot();
    }
    if cancellation.is_cancelled() {
        return Err(Error::cancelled("connect protected Wi-Fi"));
    }

    let profile = linux_wifi_profiles(connection)?
        .into_iter()
        .find(|profile| network.matches_profile(&profile.id));
    let active_path = if let Some(profile) = profile {
        manager
            .call::<_, _, OwnedObjectPath>(
                "ActivateConnection",
                &(profile.connection_path, device.clone(), access_point.path),
            )
            .map_err(|error| Error::new("connect protected Wi-Fi", error.to_string()))?
    } else {
        let template = secret_agent::connection_template(network)
            .map_err(|error| Error::new("prepare protected Wi-Fi", error.to_string()))?;
        let (_, active_path) = manager
            .call::<_, _, (OwnedObjectPath, OwnedObjectPath)>(
                "AddAndActivateConnection",
                &(template, device.clone(), access_point.path),
            )
            .map_err(|error| Error::new("connect protected Wi-Fi", error.to_string()))?;
        active_path
    };
    wait_for_wifi_activation(
        connection,
        &active_path,
        &device,
        network,
        Some(cancellation),
    )
}

#[cfg(not(target_os = "macos"))]
fn linux_forget_wifi(network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
    use zbus::zvariant::OwnedObjectPath;

    let connection = system_connection("connect to NetworkManager")?;
    let manager = manager_proxy(&connection)?;
    let matching_profiles = linux_wifi_profiles(&connection)?
        .into_iter()
        .filter(|profile| network.matches_profile(&profile.id))
        .collect::<Vec<_>>();
    if matching_profiles.is_empty() {
        return Err(Error::new(
            "forget Wi-Fi network",
            "the saved network is no longer available",
        ));
    }

    let device = wifi_device_path(&connection)?;
    let active = if let Some(device) = &device {
        if let Some(active_path) = device_active_connection(&connection, device)? {
            let proxy = zbus::blocking::Proxy::new(
                &connection,
                "org.freedesktop.NetworkManager",
                active_path.as_str(),
                "org.freedesktop.NetworkManager.Connection.Active",
            )
            .map_err(|error| Error::new("open active Wi-Fi connection", error.to_string()))?;
            let profile_path = proxy
                .get_property::<OwnedObjectPath>("Connection")
                .map_err(|error| Error::new("read active Wi-Fi profile", error.to_string()))?;
            matching_profiles
                .iter()
                .any(|profile| profile.connection_path == profile_path)
                .then_some(active_path.clone())
        } else {
            None
        }
    } else {
        None
    };

    let mut mutation_errors = Vec::new();
    if let Some(active_path) = &active {
        if let Err(error) =
            manager.call::<_, _, ()>("DeactivateConnection", &(active_path.clone(),))
        {
            mutation_errors.push(format!("could not disconnect the active profile: {error}"));
        }
    }

    for profile in &matching_profiles {
        let result = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.NetworkManager",
            profile.connection_path.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .and_then(|proxy| proxy.call::<_, _, ()>("Delete", &()));
        if let Err(error) = result {
            mutation_errors.push(format!(
                "could not delete a matching saved profile: {error}"
            ));
        }
    }

    let mut profiles_removed = false;
    for _ in 0..40 {
        let remaining = linux_wifi_profiles(&connection)?
            .into_iter()
            .any(|profile| network.matches_profile(&profile.id));
        if !remaining {
            profiles_removed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !profiles_removed {
        let detail = mutation_errors
            .first()
            .cloned()
            .unwrap_or_else(|| "NetworkManager did not remove every matching profile".to_string());
        return Err(Error::new("forget Wi-Fi network", detail));
    }

    if let (Some(device), Some(active_path)) = (device.as_ref(), active.as_ref()) {
        let mut disconnected = false;
        for _ in 0..40 {
            if device_active_connection(&connection, device)?.as_ref() != Some(active_path) {
                disconnected = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !disconnected {
            return Err(Error::new(
                "forget Wi-Fi network",
                mutation_errors.first().cloned().unwrap_or_else(|| {
                    "the saved profile was removed, but its active connection did not stop"
                        .to_string()
                }),
            ));
        }
    }

    linux_snapshot()
}

#[cfg(not(target_os = "macos"))]
fn device_active_connection(
    connection: &zbus::blocking::Connection,
    device: &zbus::zvariant::OwnedObjectPath,
) -> Result<Option<zbus::zvariant::OwnedObjectPath>, Error> {
    let proxy = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        device.as_str(),
        "org.freedesktop.NetworkManager.Device",
    )
    .map_err(|error| Error::new("open Wi-Fi device", error.to_string()))?;
    let active = proxy
        .get_property::<zbus::zvariant::OwnedObjectPath>("ActiveConnection")
        .map_err(|error| Error::new("read active Wi-Fi connection", error.to_string()))?;
    Ok((active.as_str() != "/").then_some(active))
}

#[cfg(not(target_os = "macos"))]
fn wait_for_wifi_activation(
    connection: &zbus::blocking::Connection,
    active_path: &zbus::zvariant::OwnedObjectPath,
    device: &zbus::zvariant::OwnedObjectPath,
    network: &WifiNetworkId,
    cancellation: Option<&WifiCancellation>,
) -> Result<WifiSnapshot, Error> {
    let proxy = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        active_path.as_str(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )
    .map_err(|error| Error::new("watch Wi-Fi activation", error.to_string()))?;
    for _ in 0..40 {
        if cancellation.is_some_and(WifiCancellation::is_cancelled) {
            if let Ok(manager) = manager_proxy(connection) {
                let _ = manager.call::<_, _, ()>("DeactivateConnection", &(active_path.clone(),));
            }
            return Err(Error::cancelled("connect Wi-Fi"));
        }
        match proxy.get_property::<u32>("State") {
            Ok(2) => {
                let snapshot = linux_snapshot()?;
                if snapshot
                    .networks
                    .iter()
                    .any(|candidate| candidate.id == *network && candidate.connected)
                {
                    return Ok(snapshot);
                }
            }
            Ok(3 | 4) => {
                return Err(Error::new(
                    "connect Wi-Fi",
                    wifi_activation_failure(connection, device, network),
                ));
            }
            Ok(_) => {}
            Err(_) => {
                let snapshot = linux_snapshot()?;
                if snapshot
                    .networks
                    .iter()
                    .any(|candidate| candidate.id == *network && candidate.connected)
                {
                    return Ok(snapshot);
                }
                return Err(Error::new(
                    "connect Wi-Fi",
                    "the connection attempt disappeared before completion",
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if let Ok(manager) = manager_proxy(connection) {
        let _ = manager.call::<_, _, ()>("DeactivateConnection", &(active_path.clone(),));
    }
    Err(Error::new(
        "connect Wi-Fi",
        "the connection did not finish within 10 seconds",
    ))
}

#[cfg(not(target_os = "macos"))]
fn wifi_activation_failure(
    connection: &zbus::blocking::Connection,
    device: &zbus::zvariant::OwnedObjectPath,
    network: &WifiNetworkId,
) -> &'static str {
    let reason = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        device.as_str(),
        "org.freedesktop.NetworkManager.Device",
    )
    .ok()
    .and_then(|proxy| proxy.get_property::<(u32, u32)>("StateReason").ok())
    .map(|(_, reason)| reason);
    match reason {
        Some(7) => "NetworkManager could not obtain the Wi-Fi password",
        Some(8..=11) if network.security.needs_password() => {
            "the password may be incorrect or network authentication failed"
        }
        Some(53) => "the network is no longer in range",
        _ => "NetworkManager rejected the connection",
    }
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
        let (configuration, configuration_error) = active_path
            .as_ref()
            .filter(|active| active.as_str() != "/")
            .map_or(
                (None, None),
                |active| match network_editor::linux_active_configuration(
                    &connection,
                    &path,
                    active,
                ) {
                    Ok(configuration) => (configuration, None),
                    Err(error) => (None, Some(error.to_string())),
                },
            );
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
            configuration,
            configuration_error,
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
const VPN_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(not(target_os = "macos"))]
const VPN_DEACTIVATION_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(target_os = "macos"))]
const VPN_STATE_INTERVAL: Duration = Duration::from_millis(250);

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
fn linux_set_vpn_enabled(
    id: &VpnProfileId,
    enabled: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    use zbus::zvariant::OwnedObjectPath;

    if enabled && cancellation.is_cancelled() {
        return Err(Error::cancelled("connect VPN"));
    }
    let connection = system_connection("connect to NetworkManager")?;
    let record = linux_vpn_records(&connection)?
        .into_iter()
        .find(|record| record.profile.id == *id)
        .ok_or_else(|| Error::new("find VPN profile", "the profile no longer exists"))?;
    let manager = manager_proxy(&connection)?;
    if enabled {
        if record.profile.state == VpnState::Connected {
            return linux_vpn_snapshot();
        }
        if let Some(active_path) = record.active_path {
            return wait_for_vpn_activation(&connection, id, &active_path, false, cancellation);
        }
        let root = OwnedObjectPath::try_from("/")
            .map_err(|error| Error::new("prepare VPN activation", error.to_string()))?;
        let active_path = manager
            .call::<_, _, OwnedObjectPath>(
                "ActivateConnection",
                &(record.connection_path, root.clone(), root),
            )
            .map_err(|error| Error::new("connect VPN", error.to_string()))?;
        wait_for_vpn_activation(&connection, id, &active_path, true, cancellation)
    } else if let Some(active_path) = record.active_path {
        manager
            .call::<_, _, ()>("DeactivateConnection", &(active_path.clone(),))
            .map_err(|error| Error::new("disconnect VPN", error.to_string()))?;
        wait_for_vpn_deactivation(&connection, id, &active_path)
    } else {
        linux_vpn_snapshot()
    }
}

#[cfg(not(target_os = "macos"))]
fn wait_for_vpn_activation(
    connection: &zbus::blocking::Connection,
    id: &VpnProfileId,
    active_path: &zbus::zvariant::OwnedObjectPath,
    owns_activation: bool,
    cancellation: &VpnCancellation,
) -> Result<VpnSnapshot, Error> {
    let deadline = std::time::Instant::now() + VPN_ACTIVATION_TIMEOUT;
    loop {
        if cancellation.is_cancelled() {
            if owns_activation {
                stop_exact_vpn_activation(connection, id, active_path, "cancel VPN activation")?;
            }
            return Err(Error::cancelled("connect VPN"));
        }
        let records = linux_vpn_records(connection)?;
        let record = records
            .iter()
            .find(|record| record.profile.id == *id)
            .ok_or_else(|| Error::new("connect VPN", "the profile disappeared"))?;
        match record.active_path.as_ref() {
            Some(current) if current != active_path => {
                return Err(Error::new(
                    "connect VPN",
                    "a different activation replaced this request",
                ));
            }
            None => {
                return Err(Error::new(
                    "connect VPN",
                    "NetworkManager ended the connection attempt",
                ));
            }
            Some(_) => {}
        }
        match record.profile.state {
            VpnState::Connected => return linux_vpn_snapshot(),
            VpnState::Failed | VpnState::Disconnected => {
                if owns_activation {
                    stop_exact_vpn_activation(
                        connection,
                        id,
                        active_path,
                        "clean up failed VPN activation",
                    )?;
                }
                return Err(Error::new(
                    "connect VPN",
                    "the VPN plugin rejected the connection",
                ));
            }
            _ => {}
        }
        if std::time::Instant::now() >= deadline {
            if owns_activation {
                stop_exact_vpn_activation(
                    connection,
                    id,
                    active_path,
                    "clean up timed-out VPN activation",
                )?;
            }
            return Err(Error::new(
                "connect VPN",
                "the connection did not finish within 60 seconds",
            ));
        }
        std::thread::sleep(VPN_STATE_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
fn stop_exact_vpn_activation(
    connection: &zbus::blocking::Connection,
    id: &VpnProfileId,
    active_path: &zbus::zvariant::OwnedObjectPath,
    operation: &'static str,
) -> Result<(), Error> {
    let still_active = linux_vpn_records(connection)?
        .into_iter()
        .any(|record| record.profile.id == *id && record.active_path.as_ref() == Some(active_path));
    if !still_active {
        return Ok(());
    }
    if !exact_active_vpn(connection, id, active_path)? {
        return Ok(());
    }
    manager_proxy(connection)?
        .call::<_, _, ()>("DeactivateConnection", &(active_path.clone(),))
        .map_err(|error| Error::new(operation, error.to_string()))?;
    wait_for_vpn_deactivation(connection, id, active_path)
        .map(|_| ())
        .map_err(|error| Error::new(operation, error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn wait_for_vpn_deactivation(
    connection: &zbus::blocking::Connection,
    id: &VpnProfileId,
    active_path: &zbus::zvariant::OwnedObjectPath,
) -> Result<VpnSnapshot, Error> {
    let deadline = std::time::Instant::now() + VPN_DEACTIVATION_TIMEOUT;
    loop {
        let records = linux_vpn_records(connection)?;
        let Some(record) = records.iter().find(|record| record.profile.id == *id) else {
            return linux_vpn_snapshot();
        };
        if record.active_path.as_ref() != Some(active_path) {
            return linux_vpn_snapshot();
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                "disconnect VPN",
                "the connection did not stop within 10 seconds",
            ));
        }
        std::thread::sleep(VPN_STATE_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
fn exact_active_vpn(
    connection: &zbus::blocking::Connection,
    id: &VpnProfileId,
    active_path: &zbus::zvariant::OwnedObjectPath,
) -> Result<bool, Error> {
    let active = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        active_path.as_str(),
        "org.freedesktop.NetworkManager.Connection.Active",
    )
    .map_err(|error| Error::new("open active VPN connection", error.to_string()))?;
    let profile_path = active
        .get_property::<zbus::zvariant::OwnedObjectPath>("Connection")
        .map_err(|error| Error::new("identify active VPN profile", error.to_string()))?;
    let uuid = active
        .get_property::<String>("Uuid")
        .map_err(|error| Error::new("identify active VPN profile", error.to_string()))?;
    let vpn = active
        .get_property::<bool>("Vpn")
        .map_err(|error| Error::new("identify active VPN connection", error.to_string()))?;
    let connection_type = active.get_property::<String>("Type").unwrap_or_default();
    Ok((vpn || is_vpn_connection_type(&connection_type))
        && profile_path.as_str() == id.object_path
        && uuid == id.uuid)
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
        let state = zbus::blocking::Proxy::new(
            connection,
            "org.freedesktop.NetworkManager",
            path.as_str(),
            "org.freedesktop.NetworkManager.VPN.Connection",
        )
        .ok()
        .and_then(|vpn| vpn.get_property::<u32>("VpnState").ok())
        .map(vpn_state_from_vpn_connection)
        .or_else(|| {
            proxy
                .get_property::<u32>("State")
                .ok()
                .map(vpn_state_from_network_manager)
        })
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
                id: VpnProfileId {
                    object_path: path.to_string(),
                    uuid: identifier,
                },
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

#[cfg(any(not(target_os = "macos"), test))]
fn property<T>(properties: &HashMap<String, zbus::zvariant::OwnedValue>, key: &str) -> Option<T>
where
    for<'a> T: TryFrom<&'a zbus::zvariant::OwnedValue>,
{
    properties
        .get(key)
        .and_then(|value| T::try_from(value).ok())
}

#[cfg(any(not(target_os = "macos"), test))]
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

#[cfg(any(not(target_os = "macos"), test))]
fn property_bytes(
    properties: &HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<Vec<u8>> {
    properties
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| Vec::<u8>::try_from(value).ok())
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
            configuration: None,
            configuration_error: Some(
                "Connection editing is available on the Linux product target".into(),
            ),
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
fn macos_set_vpn_enabled(id: &VpnProfileId, enabled: bool) -> Result<VpnSnapshot, Error> {
    network_command(
        "scutil",
        &["--nc", if enabled { "start" } else { "stop" }, &id.uuid],
    )?;
    macos_vpn_snapshot()
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
    id: WifiNetworkId,
    strength: u8,
    known: bool,
    connected: bool,
}

#[cfg(any(not(target_os = "macos"), test))]
fn normalize_networks(network_data: Vec<RawNetwork>) -> Vec<WifiNetwork> {
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
fn normalize_saved_networks(
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
fn display_ssid(ssid: &[u8]) -> String {
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
fn vpn_state_from_vpn_connection(value: u32) -> VpnState {
    match value {
        1..=4 => VpnState::Connecting,
        5 => VpnState::Connected,
        6 => VpnState::Failed,
        7 => VpnState::Disconnected,
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
            .then_with(|| left.id.uuid.cmp(&right.id.uuid))
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

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{DynamicType, OwnedValue, Str, Value};

    fn owned<T>(value: T) -> OwnedValue
    where
        T: Into<Value<'static>> + DynamicType,
    {
        OwnedValue::try_from(Value::new(value)).unwrap()
    }

    fn string(value: &str) -> OwnedValue {
        OwnedValue::from(Str::from(value.to_owned()))
    }

    #[test]
    fn networks_are_deduplicated_sorted_and_clamped() {
        let networks = normalize_networks(vec![
            RawNetwork {
                id: WifiNetworkId::from_bytes(
                    b"Cafe".to_vec(),
                    WifiSecurity::Personal(WifiPersonalMode::Psk),
                )
                .unwrap(),
                strength: 45,
                known: false,
                connected: false,
            },
            RawNetwork {
                id: WifiNetworkId::from_bytes(
                    b"Home".to_vec(),
                    WifiSecurity::Personal(WifiPersonalMode::Psk),
                )
                .unwrap(),
                strength: 150,
                known: true,
                connected: true,
            },
            RawNetwork {
                id: WifiNetworkId::from_bytes(
                    b"Cafe".to_vec(),
                    WifiSecurity::Personal(WifiPersonalMode::Psk),
                )
                .unwrap(),
                strength: 72,
                known: true,
                connected: false,
            },
            RawNetwork {
                id: WifiNetworkId::from_bytes(
                    b"Home".to_vec(),
                    WifiSecurity::Personal(WifiPersonalMode::Psk),
                )
                .unwrap(),
                strength: 200,
                known: false,
                connected: false,
            },
        ]);

        assert_eq!(networks.len(), 2);
        assert_eq!(networks[0].ssid, "Home");
        assert_eq!(networks[0].strength, 100);
        assert!(networks[0].known);
        assert!(!networks[0].can_connect());
        assert_eq!(networks[1].ssid, "Cafe");
        assert_eq!(networks[1].strength, 72);
        assert!(networks[1].security.is_secure());
        assert!(networks[1].known);
        assert!(networks[1].can_connect());
    }

    #[test]
    fn open_and_protected_networks_with_the_same_ssid_stay_distinct() {
        let networks = normalize_networks(vec![
            RawNetwork {
                id: WifiNetworkId::from_bytes(b"Shared Name".to_vec(), WifiSecurity::Open).unwrap(),
                strength: 80,
                known: false,
                connected: false,
            },
            RawNetwork {
                id: WifiNetworkId::from_bytes(
                    b"Shared Name".to_vec(),
                    WifiSecurity::Personal(WifiPersonalMode::Psk),
                )
                .unwrap(),
                strength: 70,
                known: true,
                connected: false,
            },
        ]);

        assert_eq!(networks.len(), 2);
        assert!(!networks[0].security.is_secure());
        assert!(networks[0].can_connect());
        assert!(networks[1].security.is_secure());
        assert!(networks[1].known);
        assert!(networks[1].can_connect());
    }

    #[test]
    fn wifi_network_ids_validate_length_and_redact_ssid_bytes() {
        assert!(WifiNetworkId::from_bytes(Vec::new(), WifiSecurity::Open).is_none());
        assert!(WifiNetworkId::from_bytes(vec![b'x'; 33], WifiSecurity::Open).is_none());
        let id = WifiNetworkId::from_bytes(
            b"Private Network".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Psk),
        )
        .unwrap();
        let debug = format!("{id:?}");
        assert!(!debug.contains("Private Network"));
        assert!(debug.contains("ssid_bytes: 15"));
    }

    #[test]
    fn access_point_security_flags_map_to_supported_flows() {
        assert_eq!(
            wifi_security_from_access_point(0, 0x100, 0),
            WifiSecurity::Personal(WifiPersonalMode::Psk)
        );
        assert_eq!(
            wifi_security_from_access_point(0, 0x100, 0x400),
            WifiSecurity::Personal(WifiPersonalMode::Transition)
        );
        assert_eq!(
            wifi_security_from_access_point(0, 0, 0x400),
            WifiSecurity::Personal(WifiPersonalMode::Sae)
        );
        assert_eq!(
            wifi_security_from_access_point(0, 0, 0x800),
            WifiSecurity::EnhancedOpen
        );
        assert_eq!(
            wifi_security_from_access_point(0, 0, 0x200),
            WifiSecurity::Enterprise
        );
        assert_eq!(
            wifi_security_from_access_point(1, 0, 0),
            WifiSecurity::Legacy
        );
        assert_eq!(wifi_security_from_access_point(0, 0, 0), WifiSecurity::Open);
    }

    #[test]
    fn personal_passwords_are_validated_without_debug_exposure() {
        let psk = WifiNetworkId::from_bytes(
            b"Studio".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Psk),
        )
        .unwrap();
        assert!(WifiPassword::new("12345678".to_string(), &psk).is_ok());
        assert!(WifiPassword::new("a".repeat(63), &psk).is_ok());
        assert!(WifiPassword::new("01".repeat(32), &psk).is_ok());
        assert!(WifiPassword::new("1234567".to_string(), &psk).is_err());
        assert!(WifiPassword::new("z".repeat(64), &psk).is_err());
        assert!(WifiPassword::new("password\n".to_string(), &psk).is_err());

        let password = WifiPassword::new("correct-horse".to_string(), &psk).unwrap();
        assert_eq!(format!("{password:?}"), "WifiPassword(<redacted>)");

        let sae = WifiNetworkId::from_bytes(
            b"Modern".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Sae),
        )
        .unwrap();
        assert!(WifiPassword::new("é".to_string(), &sae).is_ok());
        assert!(WifiPassword::new(String::new(), &sae).is_err());
        assert!(WifiPassword::new("a".repeat(64), &sae).is_err());
    }

    #[test]
    fn transition_access_points_match_saved_personal_profiles() {
        let transition = WifiNetworkId::from_bytes(
            b"Studio".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Transition),
        )
        .unwrap();
        let psk = WifiNetworkId::from_bytes(
            b"Studio".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Psk),
        )
        .unwrap();
        let sae = WifiNetworkId::from_bytes(
            b"Studio".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Sae),
        )
        .unwrap();
        assert!(transition.matches_profile(&psk));
        assert!(transition.matches_profile(&sae));
    }

    #[test]
    fn saved_networks_are_deduplicated_by_exact_identity_and_sorted_by_recency() {
        let studio_psk = WifiNetworkId::from_bytes(
            b"Studio".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Psk),
        )
        .unwrap();
        let studio_open =
            WifiNetworkId::from_bytes(b"Studio".to_vec(), WifiSecurity::Open).unwrap();
        let cafe = WifiNetworkId::from_bytes(b"Cafe".to_vec(), WifiSecurity::Open).unwrap();
        let saved = normalize_saved_networks([
            (studio_psk.clone(), 2),
            (studio_open.clone(), 1),
            (cafe.clone(), 5),
            (studio_psk.clone(), 9),
        ]);

        assert_eq!(saved.len(), 3);
        assert_eq!(saved[0].id, studio_psk);
        assert_eq!(saved[1].id, cafe);
        assert_eq!(saved[2].id, studio_open);
        assert_eq!(saved[0].ssid, "Studio");
    }

    #[test]
    fn saved_profile_matching_never_crosses_ssid_or_security_identity() {
        let selected = WifiNetworkId::from_bytes(
            b"Studio".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Psk),
        )
        .unwrap();
        let other_ssid = WifiNetworkId::from_bytes(
            b"Visitor".to_vec(),
            WifiSecurity::Personal(WifiPersonalMode::Psk),
        )
        .unwrap();
        let open = WifiNetworkId::from_bytes(b"Studio".to_vec(), WifiSecurity::Open).unwrap();

        assert!(selected.matches_profile(&selected));
        assert!(!selected.matches_profile(&other_ssid));
        assert!(!selected.matches_profile(&open));
    }

    #[test]
    fn saved_wifi_profiles_preserve_exact_ssid_security_and_recency() {
        let settings = HashMap::from([
            (
                "connection".to_string(),
                HashMap::from([
                    ("type".to_string(), string("802-11-wireless")),
                    ("timestamp".to_string(), owned(42_u64)),
                ]),
            ),
            (
                "802-11-wireless".to_string(),
                HashMap::from([("ssid".to_string(), owned(b"Studio".to_vec()))]),
            ),
            (
                "802-11-wireless-security".to_string(),
                HashMap::from([("key-mgmt".to_string(), string("wpa-psk"))]),
            ),
        ]);

        let (id, timestamp) = wifi_profile_identity(&settings).unwrap();
        assert_eq!(id.ssid, b"Studio");
        assert!(id.is_secure());
        assert_eq!(timestamp, 42);
    }

    #[test]
    fn non_wifi_profiles_are_not_treated_as_known_networks() {
        let settings = HashMap::from([
            (
                "connection".to_string(),
                HashMap::from([("type".to_string(), string("802-3-ethernet"))]),
            ),
            (
                "802-11-wireless".to_string(),
                HashMap::from([("ssid".to_string(), owned(b"Studio".to_vec()))]),
            ),
        ]);

        assert!(wifi_profile_identity(&settings).is_none());
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
    fn network_manager_owner_changes_distinguish_outage_and_recovery() {
        assert_eq!(
            network_manager_owner_availability("org.freedesktop.NetworkManager", ""),
            Some(false)
        );
        assert_eq!(
            network_manager_owner_availability("org.freedesktop.NetworkManager", ":1.42"),
            Some(true)
        );
        assert_eq!(
            network_manager_owner_availability("org.example.Other", ":1.42"),
            None
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
            configuration: None,
            configuration_error: None,
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
        assert_eq!(vpn_state_from_vpn_connection(2), VpnState::Connecting);
        assert_eq!(vpn_state_from_vpn_connection(5), VpnState::Connected);
        assert_eq!(vpn_state_from_vpn_connection(6), VpnState::Failed);
        assert_eq!(vpn_state_from_vpn_connection(7), VpnState::Disconnected);
        assert_eq!(vpn_state_from_vpn_connection(99), VpnState::Unknown);
        assert_eq!(
            vpn_service_label("vpn", Some("org.freedesktop.NetworkManager.openvpn")),
            "OpenVPN"
        );
        assert_eq!(vpn_service_label("wireguard", None), "WireGuard");
    }

    #[test]
    fn vpn_profile_identity_is_opaque_and_cancellation_is_shared() {
        let id = VpnProfileId {
            object_path: "/org/freedesktop/NetworkManager/Settings/42".to_string(),
            uuid: "12345678-1234-1234-1234-123456789abc".to_string(),
        };
        let debug = format!("{id:?}");
        assert_eq!(
            debug,
            "VpnProfileId { object: \"<redacted>\", uuid: \"<redacted>\" }"
        );
        assert!(!debug.contains("Settings/42"));
        assert!(!debug.contains("12345678"));

        let cancellation = VpnCancellation::new();
        let observer = cancellation.clone();
        assert!(!observer.is_cancelled());
        cancellation.cancel();
        assert!(observer.is_cancelled());
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
