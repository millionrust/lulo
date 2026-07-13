use std::fmt;
use std::net::IpAddr;
use std::str::FromStr as _;

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashMap;
#[cfg(not(target_os = "macos"))]
use std::time::{Duration, Instant};

#[cfg(not(target_os = "macos"))]
use zbus::zvariant::OwnedObjectPath;
#[cfg(any(not(target_os = "macos"), test))]
use zbus::zvariant::{OwnedValue, Str, Value};

use super::{Error, NetworkSnapshot};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct NetworkConnectionId {
    device_path: String,
    active_path: String,
    profile_path: String,
    uuid: String,
}

impl fmt::Debug for NetworkConnectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkConnectionId")
            .field("device", &"<object>")
            .field("active", &"<object>")
            .field("profile", &"<object>")
            .field("uuid", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpFamily {
    V4,
    V6,
}

impl IpFamily {
    #[cfg(any(not(target_os = "macos"), test))]
    fn setting(self) -> &'static str {
        match self {
            Self::V4 => "ipv4",
            Self::V6 => "ipv6",
        }
    }

    fn max_prefix(self) -> u8 {
        match self {
            Self::V4 => 32,
            Self::V6 => 128,
        }
    }

    fn matches(self, address: IpAddr) -> bool {
        matches!(
            (self, address),
            (Self::V4, IpAddr::V4(_)) | (Self::V6, IpAddr::V6(_))
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::V4 => "IPv4",
            Self::V6 => "IPv6",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpMethod {
    Automatic,
    Dhcp,
    Manual,
    Disabled,
    LinkLocal,
    Unsupported(String),
}

impl IpMethod {
    pub fn editable(&self) -> bool {
        !matches!(self, Self::Unsupported(_))
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Automatic => "Automatic",
            Self::Dhcp => "DHCP Only",
            Self::Manual => "Manual",
            Self::Disabled => "Off",
            Self::LinkLocal => "Link-Local Only",
            Self::Unsupported(value) => value,
        }
    }

    #[cfg(any(not(target_os = "macos"), test))]
    fn from_dbus(value: &str) -> Self {
        match value {
            "auto" => Self::Automatic,
            "dhcp" => Self::Dhcp,
            "manual" => Self::Manual,
            "disabled" | "ignore" => Self::Disabled,
            "link-local" => Self::LinkLocal,
            value => Self::Unsupported(value.to_owned()),
        }
    }

    fn dbus_value(&self, family: IpFamily) -> Result<&'static str, NetworkValidationError> {
        match self {
            Self::Automatic => Ok("auto"),
            Self::Dhcp if family == IpFamily::V6 => Ok("dhcp"),
            Self::Manual => Ok("manual"),
            Self::Disabled => Ok("disabled"),
            Self::LinkLocal => Ok("link-local"),
            Self::Dhcp | Self::Unsupported(_) => Err(NetworkValidationError::UnsupportedMethod {
                family,
                method: self.label().to_owned(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpAddress {
    pub address: IpAddr,
    pub prefix: u8,
}

impl fmt::Display for IpAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.address, self.prefix)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpConfiguration {
    pub method: IpMethod,
    pub addresses: Vec<IpAddress>,
    pub gateway: Option<IpAddr>,
    pub dns: Vec<IpAddr>,
    pub ignore_auto_dns: bool,
}

impl IpConfiguration {
    pub fn parse(
        family: IpFamily,
        method: IpMethod,
        addresses: &str,
        gateway: &str,
        dns: &str,
        ignore_auto_dns: bool,
    ) -> Result<Self, NetworkValidationError> {
        method.dbus_value(family)?;
        let mut parsed_addresses = split_values(addresses)
            .map(|value| parse_cidr(family, value))
            .collect::<Result<Vec<_>, _>>()?;
        deduplicate(&mut parsed_addresses);
        if parsed_addresses.len() > 16 {
            return Err(NetworkValidationError::TooManyAddresses(family));
        }
        let gateway = parse_optional_address(family, gateway, "gateway")?;
        let mut dns = split_values(dns)
            .map(|value| parse_address(family, value, "DNS server"))
            .collect::<Result<Vec<_>, _>>()?;
        deduplicate(&mut dns);
        if dns.len() > 16 {
            return Err(NetworkValidationError::TooManyDnsServers(family));
        }

        match &method {
            IpMethod::Manual if parsed_addresses.is_empty() => {
                return Err(NetworkValidationError::ManualAddressRequired(family));
            }
            IpMethod::Disabled | IpMethod::LinkLocal
                if !parsed_addresses.is_empty()
                    || gateway.is_some()
                    || !dns.is_empty()
                    || ignore_auto_dns =>
            {
                return Err(NetworkValidationError::ValuesNotAllowed(family));
            }
            _ => {}
        }
        Ok(Self {
            method,
            addresses: parsed_addresses,
            gateway,
            dns,
            ignore_auto_dns,
        })
    }

    pub fn addresses_text(&self) -> String {
        self.addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn gateway_text(&self) -> String {
        self.gateway
            .map(|value| value.to_string())
            .unwrap_or_default()
    }

    pub fn dns_text(&self) -> String {
        self.dns
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyMethod {
    None,
    Automatic,
    Unsupported(i32),
}

impl ProxyMethod {
    pub fn editable(self) -> bool {
        !matches!(self, Self::Unsupported(_))
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Off",
            Self::Automatic => "Automatic",
            Self::Unsupported(_) => "Unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyConfiguration {
    pub method: ProxyMethod,
    pub pac_url: Option<String>,
    pub browser_only: bool,
    has_inline_script: bool,
}

impl ProxyConfiguration {
    pub fn new(
        method: ProxyMethod,
        pac_url: &str,
        browser_only: bool,
    ) -> Result<Self, NetworkValidationError> {
        if !method.editable() {
            return Err(NetworkValidationError::UnsupportedProxyMethod);
        }
        let pac_url = pac_url.trim();
        let pac_url = if pac_url.is_empty() {
            None
        } else {
            if pac_url.len() > 2_048 {
                return Err(NetworkValidationError::InvalidProxyUrl);
            }
            let url =
                url::Url::parse(pac_url).map_err(|_| NetworkValidationError::InvalidProxyUrl)?;
            if !matches!(url.scheme(), "http" | "https" | "file")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(NetworkValidationError::InvalidProxyUrl);
            }
            Some(url.to_string())
        };
        if method == ProxyMethod::None && pac_url.is_some() {
            return Err(NetworkValidationError::ProxyUrlWhileDisabled);
        }
        Ok(Self {
            method,
            pac_url,
            browser_only,
            has_inline_script: false,
        })
    }

    pub fn pac_url_text(&self) -> &str {
        self.pac_url.as_deref().unwrap_or("")
    }

    pub fn has_inline_script(&self) -> bool {
        self.has_inline_script
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkConfiguration {
    pub id: NetworkConnectionId,
    pub name: String,
    pub ipv4: IpConfiguration,
    pub ipv6: IpConfiguration,
    pub proxy: ProxyConfiguration,
    pub editable: bool,
    pub limitation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkEdit {
    pub id: NetworkConnectionId,
    pub ipv4: IpConfiguration,
    pub ipv6: IpConfiguration,
    pub proxy: ProxyConfiguration,
}

impl NetworkEdit {
    pub fn new(
        configuration: &NetworkConfiguration,
        ipv4: IpConfiguration,
        ipv6: IpConfiguration,
        proxy: ProxyConfiguration,
    ) -> Result<Self, NetworkValidationError> {
        if !configuration.editable {
            return Err(NetworkValidationError::ReadOnlyProfile);
        }
        validate_ip_configuration(IpFamily::V4, &ipv4)?;
        validate_ip_configuration(IpFamily::V6, &ipv6)?;
        validate_proxy_configuration(&proxy)?;
        Ok(Self {
            id: configuration.id.clone(),
            ipv4,
            ipv6,
            proxy,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkValidationError {
    InvalidAddress {
        family: IpFamily,
        field: &'static str,
    },
    InvalidPrefix(IpFamily),
    WrongAddressFamily {
        family: IpFamily,
        field: &'static str,
    },
    ManualAddressRequired(IpFamily),
    ValuesNotAllowed(IpFamily),
    TooManyAddresses(IpFamily),
    TooManyDnsServers(IpFamily),
    UnsupportedMethod {
        family: IpFamily,
        method: String,
    },
    UnsupportedProxyMethod,
    InvalidProxyUrl,
    ProxyUrlWhileDisabled,
    ReadOnlyProfile,
}

impl fmt::Display for NetworkValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress { family, field } => {
                write!(formatter, "enter a valid {} {field}", family.label())
            }
            Self::InvalidPrefix(family) => write!(
                formatter,
                "enter an {} address with a prefix from 0 to {}",
                family.label(),
                family.max_prefix()
            ),
            Self::WrongAddressFamily { family, field } => {
                write!(
                    formatter,
                    "the {field} must be an {} address",
                    family.label()
                )
            }
            Self::ManualAddressRequired(family) => {
                write!(
                    formatter,
                    "manual {} requires at least one address",
                    family.label()
                )
            }
            Self::ValuesNotAllowed(family) => write!(
                formatter,
                "{} addresses, gateway, and DNS preferences must be empty for this method",
                family.label()
            ),
            Self::TooManyAddresses(family) => {
                write!(
                    formatter,
                    "{} supports at most 16 addresses here",
                    family.label()
                )
            }
            Self::TooManyDnsServers(family) => write!(
                formatter,
                "{} supports at most 16 DNS servers here",
                family.label()
            ),
            Self::UnsupportedMethod { family, method } => {
                write!(
                    formatter,
                    "the {} method “{method}” is read-only",
                    family.label()
                )
            }
            Self::UnsupportedProxyMethod => formatter.write_str("this proxy method is read-only"),
            Self::InvalidProxyUrl => formatter.write_str(
                "enter an HTTP, HTTPS, or absolute file URL without credentials or a fragment",
            ),
            Self::ProxyUrlWhileDisabled => {
                formatter.write_str("turn on automatic proxy configuration before entering a URL")
            }
            Self::ReadOnlyProfile => formatter.write_str("this connection profile is read-only"),
        }
    }
}

impl std::error::Error for NetworkValidationError {}

fn validate_ip_configuration(
    family: IpFamily,
    configuration: &IpConfiguration,
) -> Result<(), NetworkValidationError> {
    configuration.method.dbus_value(family)?;
    if configuration
        .addresses
        .iter()
        .any(|address| !family.matches(address.address) || address.prefix > family.max_prefix())
    {
        return Err(NetworkValidationError::WrongAddressFamily {
            family,
            field: "address",
        });
    }
    if configuration
        .gateway
        .is_some_and(|value| !family.matches(value))
    {
        return Err(NetworkValidationError::WrongAddressFamily {
            family,
            field: "gateway",
        });
    }
    if configuration
        .dns
        .iter()
        .any(|value| !family.matches(*value))
    {
        return Err(NetworkValidationError::WrongAddressFamily {
            family,
            field: "DNS server",
        });
    }
    if configuration.method == IpMethod::Manual && configuration.addresses.is_empty() {
        return Err(NetworkValidationError::ManualAddressRequired(family));
    }
    if configuration.addresses.len() > 16 {
        return Err(NetworkValidationError::TooManyAddresses(family));
    }
    if configuration.dns.len() > 16 {
        return Err(NetworkValidationError::TooManyDnsServers(family));
    }
    if matches!(
        configuration.method,
        IpMethod::Disabled | IpMethod::LinkLocal
    ) && (!configuration.addresses.is_empty()
        || configuration.gateway.is_some()
        || !configuration.dns.is_empty()
        || configuration.ignore_auto_dns)
    {
        return Err(NetworkValidationError::ValuesNotAllowed(family));
    }
    Ok(())
}

fn validate_proxy_configuration(
    configuration: &ProxyConfiguration,
) -> Result<(), NetworkValidationError> {
    if !configuration.method.editable() || configuration.has_inline_script {
        return Err(NetworkValidationError::UnsupportedProxyMethod);
    }
    let checked = ProxyConfiguration::new(
        configuration.method,
        configuration.pac_url_text(),
        configuration.browser_only,
    )?;
    if checked.pac_url != configuration.pac_url {
        return Err(NetworkValidationError::InvalidProxyUrl);
    }
    Ok(())
}

fn split_values(value: &str) -> impl Iterator<Item = &str> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn parse_cidr(family: IpFamily, value: &str) -> Result<IpAddress, NetworkValidationError> {
    let (address, prefix) = value
        .rsplit_once('/')
        .ok_or(NetworkValidationError::InvalidPrefix(family))?;
    let address = parse_address(family, address.trim(), "address")?;
    let prefix = prefix
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|prefix| *prefix <= family.max_prefix())
        .ok_or(NetworkValidationError::InvalidPrefix(family))?;
    Ok(IpAddress { address, prefix })
}

fn parse_optional_address(
    family: IpFamily,
    value: &str,
    field: &'static str,
) -> Result<Option<IpAddr>, NetworkValidationError> {
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        parse_address(family, value, field).map(Some)
    }
}

fn parse_address(
    family: IpFamily,
    value: &str,
    field: &'static str,
) -> Result<IpAddr, NetworkValidationError> {
    let address = IpAddr::from_str(value)
        .map_err(|_| NetworkValidationError::InvalidAddress { family, field })?;
    if !family.matches(address) {
        return Err(NetworkValidationError::WrongAddressFamily { family, field });
    }
    Ok(address)
}

fn deduplicate<T: PartialEq>(values: &mut Vec<T>) {
    let mut index = 0;
    while index < values.len() {
        let mut later = index + 1;
        while later < values.len() {
            if values[index] == values[later] {
                values.remove(later);
            } else {
                later += 1;
            }
        }
        index += 1;
    }
}

#[cfg(any(not(target_os = "macos"), test))]
type SettingsMap = HashMap<String, HashMap<String, OwnedValue>>;

#[cfg(not(target_os = "macos"))]
const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
#[cfg(not(target_os = "macos"))]
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
#[cfg(not(target_os = "macos"))]
const SETTINGS_INTERFACE: &str = "org.freedesktop.NetworkManager.Settings";
#[cfg(not(target_os = "macos"))]
const PROFILE_INTERFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
#[cfg(not(target_os = "macos"))]
const DEVICE_INTERFACE: &str = "org.freedesktop.NetworkManager.Device";
#[cfg(not(target_os = "macos"))]
const ACTIVE_INTERFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
#[cfg(not(target_os = "macos"))]
const UPDATE_TO_DISK: u32 = 0x1;
#[cfg(not(target_os = "macos"))]
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(target_os = "macos"))]
const VERIFY_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_active_configuration(
    connection: &zbus::blocking::Connection,
    device_path: &OwnedObjectPath,
    active_path: &OwnedObjectPath,
) -> Result<Option<NetworkConfiguration>, Error> {
    if active_path.as_str() == "/" {
        return Ok(None);
    }
    let active = zbus::blocking::Proxy::new(
        connection,
        NETWORK_MANAGER,
        active_path.as_str(),
        ACTIVE_INTERFACE,
    )
    .map_err(|error| Error::new("open active network connection", error.to_string()))?;
    let profile_path = active
        .get_property::<OwnedObjectPath>("Connection")
        .map_err(|error| Error::new("identify active network profile", error.to_string()))?;
    let profile = profile_proxy(connection, &profile_path)?;
    let settings = get_settings(&profile)?;
    let connection_setting = settings
        .get("connection")
        .ok_or_else(|| Error::new("read network profile", "connection settings are missing"))?;
    let uuid = super::property_string(connection_setting, "uuid")
        .ok_or_else(|| Error::new("read network profile", "the profile UUID is missing"))?;
    let name = super::property_string(connection_setting, "id")
        .unwrap_or_else(|| "Network Connection".into());
    let ipv4 = parse_ip_setting(&settings, IpFamily::V4)?;
    let ipv6 = parse_ip_setting(&settings, IpFamily::V6)?;
    let proxy = parse_proxy_setting(&settings)?;
    let unsaved = profile
        .get_property::<bool>("Unsaved")
        .map_err(|error| Error::new("read network profile state", error.to_string()))?;
    let can_modify = settings_proxy(connection)?
        .get_property::<bool>("CanModify")
        .map_err(|error| Error::new("read network editing capability", error.to_string()))?;
    let limitations =
        profile_editing_limitations(&settings, &ipv4, &ipv6, &proxy, can_modify, unsaved);
    Ok(Some(NetworkConfiguration {
        id: NetworkConnectionId {
            device_path: device_path.to_string(),
            active_path: active_path.to_string(),
            profile_path: profile_path.to_string(),
            uuid,
        },
        name,
        ipv4,
        ipv6,
        proxy,
        editable: limitations.is_empty(),
        limitation: (!limitations.is_empty()).then(|| limitations.join(". ")),
    }))
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_ip_setting(settings: &SettingsMap, family: IpFamily) -> Result<IpConfiguration, Error> {
    let setting = settings.get(family.setting());
    let method = setting
        .and_then(|setting| super::property_string(setting, "method"))
        .map_or_else(
            || {
                if family == IpFamily::V4 {
                    IpMethod::Automatic
                } else {
                    IpMethod::Disabled
                }
            },
            |method| IpMethod::from_dbus(&method),
        );
    let address_records = setting
        .map(|setting| owned_property::<Vec<HashMap<String, OwnedValue>>>(setting, "address-data"))
        .transpose()?
        .flatten()
        .unwrap_or_default();
    let mut addresses = address_records
        .into_iter()
        .map(|record| {
            let value = super::property_string(&record, "address").ok_or_else(|| {
                Error::new("read network profile", "an address is missing its value")
            })?;
            let address = IpAddr::from_str(&value)
                .map_err(|error| Error::new("read network profile address", error.to_string()))?;
            let prefix = super::property::<u32>(&record, "prefix")
                .and_then(|prefix| u8::try_from(prefix).ok())
                .filter(|prefix| *prefix <= family.max_prefix())
                .ok_or_else(|| {
                    Error::new("read network profile", "an address has an invalid prefix")
                })?;
            if !family.matches(address) {
                return Err(Error::new(
                    "read network profile",
                    "an address belongs to the wrong IP family",
                ));
            }
            Ok(IpAddress { address, prefix })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    deduplicate(&mut addresses);
    let gateway = match setting.and_then(|setting| super::property_string(setting, "gateway")) {
        Some(value) if !value.is_empty() => {
            let address = IpAddr::from_str(&value)
                .map_err(|error| Error::new("read network profile gateway", error.to_string()))?;
            if !family.matches(address) {
                return Err(Error::new(
                    "read network profile",
                    "the gateway belongs to the wrong IP family",
                ));
            }
            Some(address)
        }
        _ => None,
    };
    let dns_values = setting
        .map(|setting| owned_property::<Vec<String>>(setting, "dns-data"))
        .transpose()?
        .flatten()
        .unwrap_or_default();
    let mut dns = dns_values
        .into_iter()
        .map(|value| {
            let address = IpAddr::from_str(&value)
                .map_err(|error| Error::new("read network profile DNS", error.to_string()))?;
            if !family.matches(address) {
                return Err(Error::new(
                    "read network profile",
                    "a DNS server belongs to the wrong IP family",
                ));
            }
            Ok(address)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    deduplicate(&mut dns);
    let ignore_auto_dns = setting
        .and_then(|setting| super::property::<bool>(setting, "ignore-auto-dns"))
        .unwrap_or(false);
    Ok(IpConfiguration {
        method,
        addresses,
        gateway,
        dns,
        ignore_auto_dns,
    })
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_proxy_setting(settings: &SettingsMap) -> Result<ProxyConfiguration, Error> {
    let setting = settings.get("proxy");
    let method = setting
        .and_then(|setting| super::property::<i32>(setting, "method"))
        .map_or(ProxyMethod::None, |method| match method {
            0 => ProxyMethod::None,
            1 => ProxyMethod::Automatic,
            method => ProxyMethod::Unsupported(method),
        });
    Ok(ProxyConfiguration {
        method,
        pac_url: setting
            .and_then(|setting| super::property_string(setting, "pac-url"))
            .filter(|value| !value.is_empty()),
        browser_only: setting
            .and_then(|setting| super::property::<bool>(setting, "browser-only"))
            .unwrap_or(false),
        has_inline_script: setting
            .and_then(|setting| super::property_string(setting, "pac-script"))
            .is_some(),
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_update(edit: &NetworkEdit) -> Result<NetworkSnapshot, Error> {
    linux_update(edit)
}

#[cfg(target_os = "macos")]
pub(super) fn system_update(_edit: &NetworkEdit) -> Result<NetworkSnapshot, Error> {
    Err(Error::new(
        "update network connection",
        "the macOS development adapter is read-only",
    ))
}

#[cfg(not(target_os = "macos"))]
fn linux_update(edit: &NetworkEdit) -> Result<NetworkSnapshot, Error> {
    validate_ip_configuration(IpFamily::V4, &edit.ipv4)
        .map_err(|error| Error::new("validate IPv4 settings", error.to_string()))?;
    validate_ip_configuration(IpFamily::V6, &edit.ipv6)
        .map_err(|error| Error::new("validate IPv6 settings", error.to_string()))?;
    validate_proxy_configuration(&edit.proxy)
        .map_err(|error| Error::new("validate proxy settings", error.to_string()))?;

    let connection = super::system_connection("connect to NetworkManager")?;
    let device_path = object_path(&edit.id.device_path, "network device")?;
    let active_path = object_path(&edit.id.active_path, "active network connection")?;
    let profile_path = object_path(&edit.id.profile_path, "network profile")?;
    verify_identity(
        &connection,
        &device_path,
        &active_path,
        &profile_path,
        &edit.id.uuid,
    )?;
    let profile = profile_proxy(&connection, &profile_path)?;
    if profile
        .get_property::<bool>("Unsaved")
        .map_err(|error| Error::new("read network profile state", error.to_string()))?
    {
        return Err(Error::new(
            "update network connection",
            "the profile has unsaved external changes",
        ));
    }
    if !settings_proxy(&connection)?
        .get_property::<bool>("CanModify")
        .map_err(|error| Error::new("read network editing capability", error.to_string()))?
    {
        return Err(Error::new(
            "update network connection",
            "NetworkManager does not allow profile changes",
        ));
    }
    let (original, settings_version) = stable_profile_settings(&connection, &profile)?;
    let original_ipv4 = parse_ip_setting(&original, IpFamily::V4)?;
    let original_ipv6 = parse_ip_setting(&original, IpFamily::V6)?;
    let original_proxy = parse_proxy_setting(&original)?;
    let limitations = profile_editing_limitations(
        &original,
        &original_ipv4,
        &original_ipv6,
        &original_proxy,
        true,
        false,
    );
    if !limitations.is_empty() {
        return Err(Error::new(
            "update network connection",
            limitations.join(". "),
        ));
    }
    let mut candidate = clone_settings(&original)?;
    apply_ip_setting(&mut candidate, IpFamily::V4, &original_ipv4, &edit.ipv4)?;
    apply_ip_setting(&mut candidate, IpFamily::V6, &original_ipv6, &edit.ipv6)?;
    apply_proxy_setting(&mut candidate, &original_proxy, &edit.proxy)?;

    let expected_candidate = clone_settings(&candidate)?;
    let ip_changed = edit.ipv4 != original_ipv4 || edit.ipv6 != original_ipv6;
    let device = device_proxy(&connection, &device_path)?;
    if ip_changed {
        let mut applied_candidate = clone_settings(&original)?;
        apply_ip_setting(
            &mut applied_candidate,
            IpFamily::V4,
            &original_ipv4,
            &edit.ipv4,
        )?;
        apply_ip_setting(
            &mut applied_candidate,
            IpFamily::V6,
            &original_ipv6,
            &edit.ipv6,
        )?;
        let (_applied, applied_version) = applied_connection(&device)?;
        if let Err(error) = reapply(&device, applied_candidate, applied_version) {
            let recovery = recover_failed_update(
                &connection,
                &profile,
                &profile_path,
                &device_path,
                &active_path,
                &expected_candidate,
                original,
            );
            return Err(Error::new(
                "apply network settings",
                format!("{error}; {recovery}"),
            ));
        }
    }

    let result = (|| {
        if ip_changed {
            wait_for_applied_edit(&device, edit)?;
        }
        persist_profile_candidate(
            &connection,
            &profile,
            &original,
            &expected_candidate,
            settings_version,
        )?;
        verify_identity(
            &connection,
            &device_path,
            &active_path,
            &profile_path,
            &edit.id.uuid,
        )?;
        if ip_changed {
            wait_for_applied_edit(&device, edit)?;
        }
        let snapshot = super::linux_network_snapshot()?;
        let configuration = snapshot
            .devices
            .iter()
            .filter_map(|device| device.configuration.as_ref())
            .find(|configuration| configuration.id == edit.id)
            .ok_or_else(|| {
                Error::new(
                    "verify network settings",
                    "the edited connection is no longer active",
                )
            })?;
        if !configuration_matches_edit(configuration, edit) {
            return Err(Error::new(
                "verify network settings",
                "NetworkManager did not retain the requested configuration",
            ));
        }
        Ok(snapshot)
    })();
    match result {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => {
            let recovery = recover_failed_update(
                &connection,
                &profile,
                &profile_path,
                &device_path,
                &active_path,
                &expected_candidate,
                original,
            );
            Err(Error::new(
                "update network connection",
                format!("{error}; {recovery}"),
            ))
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn recover_failed_update(
    connection: &zbus::blocking::Connection,
    profile: &zbus::blocking::Proxy<'_>,
    profile_path: &OwnedObjectPath,
    device_path: &OwnedObjectPath,
    active_path: &OwnedObjectPath,
    staged: &SettingsMap,
    original: SettingsMap,
) -> String {
    match stable_profile_settings(connection, profile) {
        Ok((current, version)) if current == *staged => {
            match rollback(
                connection,
                profile_path,
                device_path,
                active_path,
                original,
                version,
            ) {
                Ok(()) => "the previous configuration was restored".into(),
                Err(error) => {
                    format!("restoring the previous configuration also failed: {error}")
                }
            }
        }
        Ok((current, version)) if current == original => {
            match restore_applied_settings(
                connection,
                profile_path,
                device_path,
                active_path,
                &original,
                version,
            ) {
                Ok(()) => {
                    "the saved profile was unchanged and its active settings were restored".into()
                }
                Err(error) => {
                    format!("the saved profile was unchanged, but restoring its active settings failed: {error}")
                }
            }
        }
        Ok((current, version)) => {
            match restore_applied_settings(
                connection,
                profile_path,
                device_path,
                active_path,
                &current,
                version,
            ) {
                Ok(()) => {
                    "the profile changed concurrently; the newer profile was left untouched and reapplied"
                        .into()
                }
                Err(error) => format!(
                    "the profile changed concurrently and was left untouched, but its active settings could not be reapplied: {error}"
                ),
            }
        }
        Err(error) => format!(
            "rmac could not prove it still owned the staged profile and left it untouched: {error}"
        ),
    }
}

#[cfg(not(target_os = "macos"))]
fn verify_identity(
    connection: &zbus::blocking::Connection,
    device_path: &OwnedObjectPath,
    active_path: &OwnedObjectPath,
    profile_path: &OwnedObjectPath,
    uuid: &str,
) -> Result<(), Error> {
    let device = device_proxy(connection, device_path)?;
    let current_active = device
        .get_property::<OwnedObjectPath>("ActiveConnection")
        .map_err(|error| Error::new("identify active network connection", error.to_string()))?;
    if &current_active != active_path {
        return Err(Error::new(
            "update network connection",
            "the selected connection is no longer active on this device",
        ));
    }
    let active = zbus::blocking::Proxy::new(
        connection,
        NETWORK_MANAGER,
        active_path.as_str(),
        ACTIVE_INTERFACE,
    )
    .map_err(|error| Error::new("open active network connection", error.to_string()))?;
    let current_profile = active
        .get_property::<OwnedObjectPath>("Connection")
        .map_err(|error| Error::new("identify active network profile", error.to_string()))?;
    if &current_profile != profile_path {
        return Err(Error::new(
            "update network connection",
            "the selected profile is no longer active",
        ));
    }
    let settings = get_settings(&profile_proxy(connection, profile_path)?)?;
    let current_uuid = settings
        .get("connection")
        .and_then(|setting| super::property_string(setting, "uuid"));
    if current_uuid.as_deref() != Some(uuid) {
        return Err(Error::new(
            "update network connection",
            "the selected profile identity changed",
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn rollback(
    connection: &zbus::blocking::Connection,
    profile_path: &OwnedObjectPath,
    device_path: &OwnedObjectPath,
    active_path: &OwnedObjectPath,
    original: SettingsMap,
    version: u64,
) -> Result<(), Error> {
    let expected_settings = clone_settings(&original)?;
    let profile = profile_proxy(connection, profile_path)?;
    update_profile(&profile, original, UPDATE_TO_DISK, version)
        .map_err(|error| Error::new("restore network profile", error.to_string()))?;
    if profile
        .get_property::<bool>("Unsaved")
        .map_err(|error| Error::new("verify restored network profile", error.to_string()))?
        || get_settings(&profile)? != expected_settings
    {
        return Err(Error::new(
            "verify restored network profile",
            "NetworkManager did not restore the previous saved profile",
        ));
    }
    restore_applied_settings(
        connection,
        profile_path,
        device_path,
        active_path,
        &expected_settings,
        settings_version(connection)?,
    )
}

#[cfg(not(target_os = "macos"))]
fn restore_applied_settings(
    connection: &zbus::blocking::Connection,
    profile_path: &OwnedObjectPath,
    device_path: &OwnedObjectPath,
    active_path: &OwnedObjectPath,
    expected: &SettingsMap,
    profile_version: u64,
) -> Result<(), Error> {
    let expected_ipv4 = parse_ip_setting(expected, IpFamily::V4)?;
    let expected_ipv6 = parse_ip_setting(expected, IpFamily::V6)?;
    let before = settings_version(connection)?;
    let current = get_settings(&profile_proxy(connection, profile_path)?)?;
    let after = settings_version(connection)?;
    if before != profile_version || after != profile_version || current != *expected {
        return Err(Error::new(
            "restore active network settings",
            "the saved profile changed before it could be reapplied",
        ));
    }
    let device = device_proxy(connection, device_path)?;
    let current_active = device
        .get_property::<OwnedObjectPath>("ActiveConnection")
        .map_err(|error| Error::new("identify active network connection", error.to_string()))?;
    if &current_active != active_path {
        return Ok(());
    }
    let active = zbus::blocking::Proxy::new(
        connection,
        NETWORK_MANAGER,
        active_path.as_str(),
        ACTIVE_INTERFACE,
    )
    .map_err(|error| Error::new("open active network connection", error.to_string()))?;
    let current_profile = active
        .get_property::<OwnedObjectPath>("Connection")
        .map_err(|error| Error::new("identify active network profile", error.to_string()))?;
    if &current_profile != profile_path {
        return Ok(());
    }
    let (_, applied_version) = applied_connection(&device)?;
    reapply(&device, SettingsMap::new(), applied_version)
        .map_err(|error| Error::new("restore active network settings", error.to_string()))?;
    wait_for_applied_configuration(
        &device,
        &expected_ipv4,
        &expected_ipv6,
        "NetworkManager did not restore the previous active settings before the timeout",
    )
}

#[cfg(not(target_os = "macos"))]
fn wait_for_applied_edit(
    device: &zbus::blocking::Proxy<'_>,
    edit: &NetworkEdit,
) -> Result<(), Error> {
    wait_for_applied_configuration(
        device,
        &edit.ipv4,
        &edit.ipv6,
        "NetworkManager did not apply the requested settings before the timeout",
    )
}

#[cfg(not(target_os = "macos"))]
fn wait_for_applied_configuration(
    device: &zbus::blocking::Proxy<'_>,
    expected_ipv4: &IpConfiguration,
    expected_ipv6: &IpConfiguration,
    timeout_message: &'static str,
) -> Result<(), Error> {
    let deadline = Instant::now() + VERIFY_TIMEOUT;
    loop {
        let (settings, _) = applied_connection(device)?;
        let ipv4 = parse_ip_setting(&settings, IpFamily::V4)?;
        let ipv6 = parse_ip_setting(&settings, IpFamily::V6)?;
        if ipv4 == *expected_ipv4 && ipv6 == *expected_ipv6 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Error::new(
                "verify active network settings",
                timeout_message,
            ));
        }
        std::thread::sleep(VERIFY_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
fn configuration_matches_edit(configuration: &NetworkConfiguration, edit: &NetworkEdit) -> bool {
    configuration.ipv4 == edit.ipv4
        && configuration.ipv6 == edit.ipv6
        && proxy_matches(&configuration.proxy, &edit.proxy)
}

#[cfg(any(not(target_os = "macos"), test))]
fn proxy_matches(actual: &ProxyConfiguration, expected: &ProxyConfiguration) -> bool {
    actual.method == expected.method
        && actual.pac_url == expected.pac_url
        && actual.browser_only == expected.browser_only
}

#[cfg(any(not(target_os = "macos"), test))]
fn apply_ip_setting(
    settings: &mut SettingsMap,
    family: IpFamily,
    current: &IpConfiguration,
    configuration: &IpConfiguration,
) -> Result<(), Error> {
    validate_ip_configuration(family, configuration)
        .map_err(|error| Error::new("validate network settings", error.to_string()))?;
    if current == configuration {
        return Ok(());
    }
    let mut existing_address_data = if current.addresses != configuration.addresses {
        settings
            .get(family.setting())
            .map(|setting| {
                owned_property::<Vec<HashMap<String, OwnedValue>>>(setting, "address-data")
            })
            .transpose()?
            .flatten()
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let setting = settings.entry(family.setting().into()).or_default();
    if current.method != configuration.method {
        setting.insert(
            "method".into(),
            OwnedValue::from(Str::from(
                configuration
                    .method
                    .dbus_value(family)
                    .map_err(|error| Error::new("validate network settings", error.to_string()))?
                    .to_string(),
            )),
        );
    }
    if current.addresses != configuration.addresses {
        setting.remove("addresses");
        let address_data = configuration
            .addresses
            .iter()
            .map(|address| {
                existing_address_data
                    .iter()
                    .position(|record| address_record_matches(record, address))
                    .map(|index| existing_address_data.remove(index))
                    .unwrap_or_else(|| {
                        HashMap::from([
                            (
                                "address".to_string(),
                                OwnedValue::from(Str::from(address.address.to_string())),
                            ),
                            (
                                "prefix".to_string(),
                                OwnedValue::from(u32::from(address.prefix)),
                            ),
                        ])
                    })
            })
            .collect::<Vec<_>>();
        setting.insert("address-data".into(), owned_value(address_data)?);
    }
    if current.gateway != configuration.gateway {
        if let Some(gateway) = configuration.gateway {
            setting.insert(
                "gateway".into(),
                OwnedValue::from(Str::from(gateway.to_string())),
            );
        } else {
            setting.remove("gateway");
        }
    }
    if current.dns != configuration.dns {
        setting.remove("dns");
        setting.insert(
            "dns-data".into(),
            owned_value(
                configuration
                    .dns
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )?,
        );
    }
    if current.ignore_auto_dns != configuration.ignore_auto_dns {
        setting.insert(
            "ignore-auto-dns".into(),
            OwnedValue::from(configuration.ignore_auto_dns),
        );
    }
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
fn address_record_matches(record: &HashMap<String, OwnedValue>, address: &IpAddress) -> bool {
    let expected = address.address.to_string();
    super::property_string(record, "address").as_deref() == Some(expected.as_str())
        && super::property::<u32>(record, "prefix") == Some(u32::from(address.prefix))
}

#[cfg(any(not(target_os = "macos"), test))]
fn apply_proxy_setting(
    settings: &mut SettingsMap,
    current: &ProxyConfiguration,
    configuration: &ProxyConfiguration,
) -> Result<(), Error> {
    if current == configuration {
        return Ok(());
    }
    let setting = settings.entry("proxy".into()).or_default();
    let method = match configuration.method {
        ProxyMethod::None => 0,
        ProxyMethod::Automatic => 1,
        ProxyMethod::Unsupported(_) => {
            return Err(Error::new(
                "validate proxy settings",
                "the proxy method is read-only",
            ));
        }
    };
    if current.method != configuration.method {
        setting.insert("method".into(), OwnedValue::from(method));
    }
    if current.browser_only != configuration.browser_only {
        setting.insert(
            "browser-only".into(),
            OwnedValue::from(configuration.browser_only),
        );
    }
    if current.pac_url != configuration.pac_url {
        if let Some(url) = &configuration.pac_url {
            setting.insert("pac-url".into(), OwnedValue::from(Str::from(url.clone())));
        } else {
            setting.remove("pac-url");
        }
    }
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
fn clone_settings(settings: &SettingsMap) -> Result<SettingsMap, Error> {
    settings
        .iter()
        .map(|(setting, properties)| {
            let properties = properties
                .iter()
                .map(|(key, value)| {
                    value
                        .try_clone()
                        .map(|value| (key.clone(), value))
                        .map_err(|error| Error::new("copy network settings", error.to_string()))
                })
                .collect::<Result<HashMap<_, _>, _>>()?;
            Ok((setting.clone(), properties))
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn stable_profile_settings(
    connection: &zbus::blocking::Connection,
    profile: &zbus::blocking::Proxy<'_>,
) -> Result<(SettingsMap, u64), Error> {
    for _ in 0..3 {
        let before = settings_version(connection)?;
        let settings = get_settings(profile)?;
        let after = settings_version(connection)?;
        if before == after {
            return Ok((settings, after));
        }
    }
    Err(Error::new(
        "read network profile",
        "network profiles kept changing; wait a moment and try again",
    ))
}

#[cfg(not(target_os = "macos"))]
fn persist_profile_candidate(
    connection: &zbus::blocking::Connection,
    profile: &zbus::blocking::Proxy<'_>,
    original: &SettingsMap,
    expected: &SettingsMap,
    original_version: u64,
) -> Result<(), Error> {
    let before = settings_version(connection)?;
    let current = get_settings(profile)?;
    let after = settings_version(connection)?;
    if before != original_version || after != original_version || current != *original {
        return Err(Error::new(
            "save network settings",
            "the connection profile changed during editing",
        ));
    }
    update_profile(
        profile,
        clone_settings(expected)?,
        UPDATE_TO_DISK,
        original_version,
    )
    .map_err(|error| Error::new("save network settings", error.to_string()))?;
    if profile
        .get_property::<bool>("Unsaved")
        .map_err(|error| Error::new("verify saved network settings", error.to_string()))?
    {
        return Err(Error::new(
            "verify saved network settings",
            "NetworkManager still reports unsaved changes",
        ));
    }
    if get_settings(profile)? != *expected {
        return Err(Error::new(
            "verify saved network settings",
            "the connection profile changed while it was being saved",
        ));
    }
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
fn owned_value<T>(value: T) -> Result<OwnedValue, Error>
where
    T: Into<Value<'static>> + zbus::zvariant::DynamicType,
{
    OwnedValue::try_from(Value::new(value))
        .map_err(|error| Error::new("encode network settings", error.to_string()))
}

#[cfg(any(not(target_os = "macos"), test))]
fn owned_property<T>(
    properties: &HashMap<String, OwnedValue>,
    key: &str,
) -> Result<Option<T>, Error>
where
    T: TryFrom<OwnedValue>,
    T::Error: fmt::Display,
{
    let Some(value) = properties.get(key) else {
        return Ok(None);
    };
    let value = value
        .try_clone()
        .map_err(|error| Error::new("copy network profile value", error.to_string()))?;
    T::try_from(value)
        .map(Some)
        .map_err(|error| Error::new("decode network profile value", error.to_string()))
}

#[cfg(any(not(target_os = "macos"), test))]
fn legacy_ip_limitations(settings: &SettingsMap) -> Vec<String> {
    [IpFamily::V4, IpFamily::V6]
        .into_iter()
        .filter_map(|family| {
            let setting = settings.get(family.setting())?;
            let legacy_addresses = setting.contains_key("addresses");
            let legacy_dns = setting.contains_key("dns");
            (legacy_addresses || legacy_dns).then(|| {
                format!(
                    "{} uses a legacy NetworkManager format and is read-only here",
                    family.label()
                )
            })
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn profile_editing_limitations(
    settings: &SettingsMap,
    ipv4: &IpConfiguration,
    ipv6: &IpConfiguration,
    proxy: &ProxyConfiguration,
    can_modify: bool,
    unsaved: bool,
) -> Vec<String> {
    let mut limitations = Vec::new();
    if !can_modify {
        limitations.push("NetworkManager does not allow profile changes".to_string());
    }
    if unsaved {
        limitations.push("Save or discard external unsaved profile changes first".to_string());
    }
    if !ipv4.method.editable() {
        limitations.push(format!(
            "IPv4 method “{}” is not editable",
            ipv4.method.label()
        ));
    } else if let Err(error) = validate_ip_configuration(IpFamily::V4, ipv4) {
        limitations.push(format!(
            "The current IPv4 values are read-only here: {error}"
        ));
    }
    if !ipv6.method.editable() {
        limitations.push(format!(
            "IPv6 method “{}” is not editable",
            ipv6.method.label()
        ));
    } else if let Err(error) = validate_ip_configuration(IpFamily::V6, ipv6) {
        limitations.push(format!(
            "The current IPv6 values are read-only here: {error}"
        ));
    }
    if !proxy.method.editable() {
        limitations.push("The current proxy method is not editable".to_string());
    }
    if proxy.has_inline_script {
        limitations.push("Inline proxy scripts must be edited with another tool".to_string());
    }
    if proxy.method == ProxyMethod::None && (proxy.pac_url.is_some() || proxy.browser_only) {
        limitations.push(
            "This disabled proxy has dormant values that must be reviewed with another tool"
                .to_string(),
        );
    }
    limitations.extend(legacy_ip_limitations(settings));
    limitations
}

#[cfg(not(target_os = "macos"))]
fn settings_version(connection: &zbus::blocking::Connection) -> Result<u64, Error> {
    settings_proxy(connection)?
        .get_property::<u64>("VersionId")
        .map_err(|error| {
            Error::new(
                "read NetworkManager settings version",
                format!("safe concurrent editing requires NetworkManager 1.44 or newer: {error}"),
            )
        })
}

#[cfg(not(target_os = "macos"))]
fn settings_proxy(
    connection: &zbus::blocking::Connection,
) -> Result<zbus::blocking::Proxy<'_>, Error> {
    zbus::blocking::Proxy::new(
        connection,
        NETWORK_MANAGER,
        SETTINGS_PATH,
        SETTINGS_INTERFACE,
    )
    .map_err(|error| Error::new("open NetworkManager settings", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn update_profile(
    profile: &zbus::blocking::Proxy<'_>,
    settings: SettingsMap,
    flags: u32,
    version: u64,
) -> zbus::Result<()> {
    let args = HashMap::from([("version-id".to_string(), OwnedValue::from(version))]);
    profile
        .call::<_, _, HashMap<String, OwnedValue>>("Update2", &(settings, flags, args))
        .map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn reapply(
    device: &zbus::blocking::Proxy<'_>,
    settings: SettingsMap,
    version: u64,
) -> zbus::Result<()> {
    device.call::<_, _, ()>("Reapply", &(settings, version, 0_u32))
}

#[cfg(not(target_os = "macos"))]
fn applied_connection(device: &zbus::blocking::Proxy<'_>) -> Result<(SettingsMap, u64), Error> {
    device
        .call::<_, _, (SettingsMap, u64)>("GetAppliedConnection", &(0_u32,))
        .map_err(|error| Error::new("read applied network settings", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn get_settings(profile: &zbus::blocking::Proxy<'_>) -> Result<SettingsMap, Error> {
    profile
        .call::<_, _, SettingsMap>("GetSettings", &())
        .map_err(|error| Error::new("read network profile", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn profile_proxy<'a>(
    connection: &'a zbus::blocking::Connection,
    path: &'a OwnedObjectPath,
) -> Result<zbus::blocking::Proxy<'a>, Error> {
    zbus::blocking::Proxy::new(
        connection,
        NETWORK_MANAGER,
        path.as_str(),
        PROFILE_INTERFACE,
    )
    .map_err(|error| Error::new("open network profile", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn device_proxy<'a>(
    connection: &'a zbus::blocking::Connection,
    path: &'a OwnedObjectPath,
) -> Result<zbus::blocking::Proxy<'a>, Error> {
    zbus::blocking::Proxy::new(connection, NETWORK_MANAGER, path.as_str(), DEVICE_INTERFACE)
        .map_err(|error| Error::new("open network device", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn object_path(value: &str, kind: &'static str) -> Result<OwnedObjectPath, Error> {
    OwnedObjectPath::try_from(value).map_err(|error| {
        Error::new(
            "validate network identity",
            format!("invalid {kind}: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(value: &str) -> OwnedValue {
        OwnedValue::from(Str::from(value.to_owned()))
    }

    #[test]
    fn ip_inputs_validate_family_prefix_and_manual_requirements() {
        let ipv4 = IpConfiguration::parse(
            IpFamily::V4,
            IpMethod::Manual,
            "192.0.2.20/24, 192.0.2.21/24",
            "192.0.2.1",
            "1.1.1.1, 9.9.9.9",
            true,
        )
        .unwrap();
        assert_eq!(ipv4.addresses.len(), 2);
        assert_eq!(ipv4.gateway_text(), "192.0.2.1");
        assert!(ipv4.ignore_auto_dns);

        assert!(matches!(
            IpConfiguration::parse(IpFamily::V4, IpMethod::Manual, "", "", "", false),
            Err(NetworkValidationError::ManualAddressRequired(IpFamily::V4))
        ));
        assert!(matches!(
            IpConfiguration::parse(
                IpFamily::V4,
                IpMethod::Automatic,
                "2001:db8::1/64",
                "",
                "",
                false,
            ),
            Err(NetworkValidationError::WrongAddressFamily { .. })
        ));
        assert!(matches!(
            IpConfiguration::parse(
                IpFamily::V6,
                IpMethod::Manual,
                "2001:db8::1/129",
                "",
                "",
                false,
            ),
            Err(NetworkValidationError::InvalidPrefix(IpFamily::V6))
        ));
    }

    #[test]
    fn duplicate_addresses_and_dns_are_removed_without_reordering() {
        let configuration = IpConfiguration::parse(
            IpFamily::V6,
            IpMethod::Automatic,
            "2001:db8::5/64,2001:db8::5/64",
            "",
            "2001:4860:4860::8888,2001:4860:4860::8888",
            false,
        )
        .unwrap();
        assert_eq!(configuration.addresses.len(), 1);
        assert_eq!(configuration.dns.len(), 1);
    }

    #[test]
    fn disabled_and_link_local_methods_reject_network_values() {
        assert!(matches!(
            IpConfiguration::parse(IpFamily::V4, IpMethod::Disabled, "", "", "1.1.1.1", false,),
            Err(NetworkValidationError::ValuesNotAllowed(IpFamily::V4))
        ));
        assert!(matches!(
            IpConfiguration::parse(IpFamily::V6, IpMethod::LinkLocal, "", "", "", true,),
            Err(NetworkValidationError::ValuesNotAllowed(IpFamily::V6))
        ));
    }

    #[test]
    fn network_edit_revalidates_public_configuration_fields() {
        let configuration = NetworkConfiguration {
            id: NetworkConnectionId {
                device_path: "/org/freedesktop/NetworkManager/Devices/7".into(),
                active_path: "/org/freedesktop/NetworkManager/ActiveConnection/9".into(),
                profile_path: "/org/freedesktop/NetworkManager/Settings/3".into(),
                uuid: "00000000-0000-0000-0000-000000000000".into(),
            },
            name: "Wired".into(),
            ipv4: IpConfiguration::parse(IpFamily::V4, IpMethod::Automatic, "", "", "", false)
                .unwrap(),
            ipv6: IpConfiguration::parse(IpFamily::V6, IpMethod::Automatic, "", "", "", false)
                .unwrap(),
            proxy: ProxyConfiguration::new(ProxyMethod::None, "", false).unwrap(),
            editable: true,
            limitation: None,
        };
        let invalid_ipv4 = IpConfiguration {
            method: IpMethod::Disabled,
            addresses: Vec::new(),
            gateway: None,
            dns: Vec::new(),
            ignore_auto_dns: true,
        };
        assert!(matches!(
            NetworkEdit::new(
                &configuration,
                invalid_ipv4,
                configuration.ipv6.clone(),
                configuration.proxy.clone(),
            ),
            Err(NetworkValidationError::ValuesNotAllowed(IpFamily::V4))
        ));
    }

    #[test]
    fn proxy_urls_are_bounded_and_do_not_accept_credentials() {
        let proxy = ProxyConfiguration::new(
            ProxyMethod::Automatic,
            "https://proxy.example/proxy.pac",
            false,
        )
        .unwrap();
        assert_eq!(proxy.pac_url_text(), "https://proxy.example/proxy.pac");
        assert!(ProxyConfiguration::new(
            ProxyMethod::Automatic,
            "https://user:secret@proxy.example/proxy.pac",
            false,
        )
        .is_err());
        assert!(ProxyConfiguration::new(
            ProxyMethod::None,
            "https://proxy.example/proxy.pac",
            false,
        )
        .is_err());
    }

    #[test]
    fn connection_identity_debug_does_not_expose_object_paths() {
        let id = NetworkConnectionId {
            device_path: "/org/freedesktop/NetworkManager/Devices/7".into(),
            active_path: "/org/freedesktop/NetworkManager/ActiveConnection/9".into(),
            profile_path: "/org/freedesktop/NetworkManager/Settings/3".into(),
            uuid: "00000000-0000-0000-0000-000000000000".into(),
        };
        let debug = format!("{id:?}");
        assert!(!debug.contains("Devices/7"));
        assert!(!debug.contains("Settings/3"));
        assert!(!debug.contains("00000000"));
    }

    #[test]
    fn settings_map_edits_preserve_unrelated_profile_values() {
        let mut settings = SettingsMap::from([
            (
                "connection".into(),
                HashMap::from([
                    ("id".into(), string("Wired")),
                    (
                        "uuid".into(),
                        string("00000000-0000-0000-0000-000000000000"),
                    ),
                    ("custom-policy".into(), string("keep-me")),
                ]),
            ),
            (
                "ipv4".into(),
                HashMap::from([
                    ("method".into(), string("auto")),
                    ("route-metric".into(), OwnedValue::from(37_u32)),
                ]),
            ),
            (
                "ipv6".into(),
                HashMap::from([("method".into(), string("auto"))]),
            ),
        ]);
        let original = clone_settings(&settings).unwrap();
        let current_ipv4 = parse_ip_setting(&settings, IpFamily::V4).unwrap();
        let current_proxy = parse_proxy_setting(&settings).unwrap();
        let ipv4 = IpConfiguration::parse(
            IpFamily::V4,
            IpMethod::Manual,
            "192.0.2.20/24",
            "192.0.2.1",
            "1.1.1.1, 9.9.9.9",
            true,
        )
        .unwrap();
        let proxy = ProxyConfiguration::new(ProxyMethod::Automatic, "https://proxy.test/pac", true)
            .unwrap();

        apply_ip_setting(&mut settings, IpFamily::V4, &current_ipv4, &ipv4).unwrap();
        apply_proxy_setting(&mut settings, &current_proxy, &proxy).unwrap();

        assert_eq!(settings["connection"], original["connection"]);
        assert_eq!(
            super::super::property::<u32>(&settings["ipv4"], "route-metric"),
            Some(37)
        );
        assert_eq!(parse_ip_setting(&settings, IpFamily::V4).unwrap(), ipv4);
        assert!(proxy_matches(
            &parse_proxy_setting(&settings).unwrap(),
            &proxy
        ));
    }

    #[test]
    fn deprecated_ip_arrays_make_the_profile_read_only() {
        let mut settings = SettingsMap::from([(
            "ipv4".into(),
            HashMap::from([("method".into(), string("auto"))]),
        )]);
        settings.get_mut("ipv4").unwrap().insert(
            "addresses".into(),
            owned_value(vec![vec![0_u32, 24_u32, 0_u32]]).unwrap(),
        );
        assert_eq!(legacy_ip_limitations(&settings).len(), 1);
    }

    #[test]
    fn dns_only_edits_preserve_complete_address_records() {
        let address_record: HashMap<String, OwnedValue> = HashMap::from([
            ("address".into(), string("192.0.2.20")),
            ("prefix".into(), OwnedValue::from(24_u32)),
            ("label".into(), string("provider-owned-attribute")),
        ]);
        let mut settings = SettingsMap::from([(
            "ipv4".into(),
            HashMap::from([
                ("method".into(), string("auto")),
                (
                    "address-data".into(),
                    owned_value(vec![address_record]).unwrap(),
                ),
            ]),
        )]);
        let original = clone_settings(&settings).unwrap();
        let current = parse_ip_setting(&settings, IpFamily::V4).unwrap();
        let mut edited = current.clone();
        edited.dns = vec!["1.1.1.1".parse().unwrap()];

        apply_ip_setting(&mut settings, IpFamily::V4, &current, &edited).unwrap();

        assert_eq!(
            settings["ipv4"]["address-data"],
            original["ipv4"]["address-data"]
        );
        assert_eq!(parse_ip_setting(&settings, IpFamily::V4).unwrap(), edited);
    }
}
