//! Cross-platform Wi-Fi, network, VPN, cancellation, secret, and error model.

use std::fmt;
use zeroize::Zeroize as _;

use super::NetworkConfiguration;

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
        matches!(
            self,
            Self::Open | Self::EnhancedOpen | Self::Personal(_) | Self::Enterprise
        )
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
    pub(crate) ssid: Vec<u8>,
    pub(crate) security: WifiSecurity,
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
    pub(crate) fn matches_profile(&self, profile: &Self) -> bool {
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

    pub fn needs_enterprise_setup(&self) -> bool {
        !self.known && self.security == WifiSecurity::Enterprise
    }
}

pub struct WifiPassword {
    pub(crate) bytes: Vec<u8>,
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
    pub(crate) fn expose<R>(&self, use_password: impl FnOnce(&str) -> R) -> R {
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

pub struct WifiEnterpriseCredentials {
    pub(crate) identity: String,
    pub(crate) anonymous_identity: Option<String>,
    pub(crate) domain_suffix: String,
    pub(crate) password: Vec<u8>,
}

impl WifiEnterpriseCredentials {
    pub fn new(
        identity: String,
        anonymous_identity: String,
        domain_suffix: String,
        mut password: String,
        network: &WifiNetworkId,
    ) -> Result<Self, WifiEnterpriseCredentialsError> {
        if network.security != WifiSecurity::Enterprise {
            password.zeroize();
            return Err(WifiEnterpriseCredentialsError::UnsupportedSecurity);
        }
        if !valid_enterprise_identity(&identity) {
            password.zeroize();
            return Err(WifiEnterpriseCredentialsError::InvalidIdentity);
        }
        let anonymous_identity = if anonymous_identity.is_empty() {
            None
        } else if valid_enterprise_identity(&anonymous_identity) {
            Some(anonymous_identity)
        } else {
            password.zeroize();
            return Err(WifiEnterpriseCredentialsError::InvalidAnonymousIdentity);
        };
        let domain_suffix = domain_suffix.to_ascii_lowercase();
        if !valid_certificate_domain(&domain_suffix) {
            password.zeroize();
            return Err(WifiEnterpriseCredentialsError::InvalidDomain);
        }
        if password.is_empty() || password.len() > 1_024 || password.contains('\0') {
            password.zeroize();
            return Err(WifiEnterpriseCredentialsError::InvalidPassword);
        }
        Ok(Self {
            identity,
            anonymous_identity,
            domain_suffix,
            password: password.into_bytes(),
        })
    }

    #[cfg(any(not(target_os = "macos"), test))]
    pub(crate) fn expose_password<R>(&self, use_password: impl FnOnce(&str) -> R) -> R {
        let value =
            std::str::from_utf8(&self.password).expect("enterprise password must remain UTF-8");
        use_password(value)
    }
}

impl fmt::Debug for WifiEnterpriseCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WifiEnterpriseCredentials(<redacted>)")
    }
}

impl Drop for WifiEnterpriseCredentials {
    fn drop(&mut self) {
        self.identity.zeroize();
        if let Some(identity) = &mut self.anonymous_identity {
            identity.zeroize();
        }
        self.domain_suffix.zeroize();
        self.password.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiEnterpriseCredentialsError {
    UnsupportedSecurity,
    InvalidIdentity,
    InvalidAnonymousIdentity,
    InvalidDomain,
    InvalidPassword,
}

impl fmt::Display for WifiEnterpriseCredentialsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedSecurity => "this network is not an enterprise network",
            Self::InvalidIdentity => {
                "enter an identity without leading, trailing, or control characters"
            }
            Self::InvalidAnonymousIdentity => "enter a valid anonymous identity or leave it empty",
            Self::InvalidDomain => "enter an ASCII certificate domain such as example.com",
            Self::InvalidPassword => "enter a password between 1 and 1,024 bytes",
        })
    }
}

impl std::error::Error for WifiEnterpriseCredentialsError {}

pub(crate) fn valid_enterprise_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

pub(crate) fn valid_certificate_domain(value: &str) -> bool {
    if value.len() > 253 || value.starts_with('.') || value.ends_with('.') || !value.contains('.') {
        return false;
    }
    value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

#[derive(Clone, Default)]
pub struct WifiCancellation(pub(crate) std::sync::Arc<std::sync::atomic::AtomicBool>);

impl WifiCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
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
    pub(crate) object_path: String,
    pub(crate) uuid: String,
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
pub struct VpnCancellation(pub(crate) std::sync::Arc<std::sync::atomic::AtomicBool>);

impl VpnCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
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
    pub(crate) operation: &'static str,
    pub(crate) detail: String,
    pub(crate) cancelled: bool,
}

impl Error {
    pub(crate) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
            cancelled: false,
        }
    }

    pub(crate) fn cancelled(operation: &'static str) -> Self {
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
