use std::fmt;

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashMap;

#[cfg(any(not(target_os = "macos"), test))]
type SettingsMap = HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>;

#[cfg(all(target_os = "macos", not(test)))]
type SettingsMap = ();

#[derive(Clone)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct VpnProfileEditId {
    profile: super::VpnProfileId,
    settings: SettingsMap,
}

impl fmt::Debug for VpnProfileEditId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VpnProfileEditId")
            .field("profile", &"<redacted>")
            .field("settings", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct VpnProfileConfiguration {
    id: VpnProfileEditId,
    pub name: String,
    pub service: String,
    pub username: String,
    pub persistent: bool,
    pub timeout: u32,
    pub supports_vpn_options: bool,
}

impl fmt::Debug for VpnProfileConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VpnProfileConfiguration")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("service", &self.service)
            .field("username", &"<redacted>")
            .field("persistent", &self.persistent)
            .field("timeout", &self.timeout)
            .field("supports_vpn_options", &self.supports_vpn_options)
            .finish()
    }
}

#[derive(Clone)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct VpnProfileEdit {
    id: VpnProfileEditId,
    name: String,
    username: String,
    persistent: bool,
    timeout: u32,
    supports_vpn_options: bool,
}

impl fmt::Debug for VpnProfileEdit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VpnProfileEdit")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("username", &"<redacted>")
            .field("persistent", &self.persistent)
            .field("timeout", &self.timeout)
            .field("supports_vpn_options", &self.supports_vpn_options)
            .finish()
    }
}

impl VpnProfileEdit {
    pub fn new(
        configuration: &VpnProfileConfiguration,
        name: &str,
        username: &str,
        persistent: bool,
        timeout: u32,
    ) -> Result<Self, VpnEditValidationError> {
        let name = validate_name(name)?;
        let username = validate_username(username)?;
        if !configuration.supports_vpn_options
            && (username != configuration.username
                || persistent != configuration.persistent
                || timeout != configuration.timeout)
        {
            return Err(VpnEditValidationError::UnsupportedVpnOptions);
        }
        Ok(Self {
            id: configuration.id.clone(),
            name,
            username,
            persistent,
            timeout,
            supports_vpn_options: configuration.supports_vpn_options,
        })
    }

    pub fn is_unchanged(&self, configuration: &VpnProfileConfiguration) -> bool {
        self.name == configuration.name
            && self.username == configuration.username
            && self.persistent == configuration.persistent
            && self.timeout == configuration.timeout
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VpnEditValidationError {
    InvalidName,
    InvalidUsername,
    UnsupportedVpnOptions,
}

impl fmt::Display for VpnEditValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidName => {
                "enter a connection name from 1 to 256 characters without control characters"
            }
            Self::InvalidUsername => {
                "enter a username no longer than 256 characters and without control characters"
            }
            Self::UnsupportedVpnOptions => {
                "this profile type supports renaming only in System Settings"
            }
        })
    }
}

impl std::error::Error for VpnEditValidationError {}

pub(super) fn configuration(
    id: &super::VpnProfileId,
) -> Result<VpnProfileConfiguration, super::Error> {
    #[cfg(not(target_os = "macos"))]
    {
        linux_configuration(id)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = id;
        Err(super::Error::new(
            "open VPN profile editor",
            "profile editing is available in the supported Linux session",
        ))
    }
}

pub(super) fn update(edit: &VpnProfileEdit) -> Result<super::VpnSnapshot, super::Error> {
    #[cfg(not(target_os = "macos"))]
    {
        linux_update(edit)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = edit;
        Err(super::Error::new(
            "save VPN profile",
            "profile editing is available in the supported Linux session",
        ))
    }
}

fn validate_name(value: &str) -> Result<String, VpnEditValidationError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 256 || value.chars().any(char::is_control) {
        return Err(VpnEditValidationError::InvalidName);
    }
    Ok(value.to_owned())
}

fn validate_username(value: &str) -> Result<String, VpnEditValidationError> {
    let value = value.trim();
    if value.chars().count() > 256 || value.chars().any(char::is_control) {
        return Err(VpnEditValidationError::InvalidUsername);
    }
    Ok(value.to_owned())
}

#[cfg(not(target_os = "macos"))]
fn linux_configuration(id: &super::VpnProfileId) -> Result<VpnProfileConfiguration, super::Error> {
    let connection = super::system_connection("connect to NetworkManager for VPN editing")?;
    require_exact_record(&connection, id)?;
    let (settings, unsaved) = stable_profile_settings(&connection, id)?;
    if unsaved {
        return Err(super::Error::new(
            "open VPN profile editor",
            "the profile is temporary and still owned by another editor",
        ));
    }
    let connection_type = validate_profile_type(id, &settings)?;
    configuration_from_settings(id.clone(), settings, &connection_type)
}

#[cfg(not(target_os = "macos"))]
fn configuration_from_settings(
    profile: super::VpnProfileId,
    settings: SettingsMap,
    connection_type: &str,
) -> Result<VpnProfileConfiguration, super::Error> {
    let connection = settings.get("connection").ok_or_else(|| {
        super::Error::new("read VPN profile editor", "connection settings are missing")
    })?;
    let name = super::property_string(connection, "id").ok_or_else(|| {
        super::Error::new("read VPN profile editor", "the connection name is missing")
    })?;
    validate_name(&name)
        .map_err(|error| super::Error::new("read VPN profile editor", error.to_string()))?;
    let vpn = settings.get("vpn");
    let supports_vpn_options = connection_type == "vpn" && vpn.is_some();
    let username = vpn
        .and_then(|properties| super::property_string(properties, "user-name"))
        .unwrap_or_default();
    let persistent = vpn
        .and_then(|properties| super::property::<bool>(properties, "persistent"))
        .unwrap_or(false);
    let timeout = vpn
        .and_then(|properties| super::property::<u32>(properties, "timeout"))
        .unwrap_or(0);
    let service_type =
        vpn.and_then(|properties| super::property_string(properties, "service-type"));
    Ok(VpnProfileConfiguration {
        id: VpnProfileEditId { profile, settings },
        name,
        service: super::vpn_service_label(connection_type, service_type.as_deref()),
        username,
        persistent,
        timeout,
        supports_vpn_options,
    })
}

#[cfg(not(target_os = "macos"))]
fn linux_update(edit: &VpnProfileEdit) -> Result<super::VpnSnapshot, super::Error> {
    let connection = super::system_connection("connect to NetworkManager for VPN editing")?;
    require_exact_record(&connection, &edit.id.profile)?;
    let (before, unsaved) = stable_profile_settings(&connection, &edit.id.profile)?;
    if unsaved || before != edit.id.settings {
        return Err(super::Error::new(
            "revalidate VPN profile edit",
            "the profile changed outside System Settings; reopen Details to edit current values",
        ));
    }
    let connection_type = validate_profile_type(&edit.id.profile, &before)?;
    let supports_vpn_options = connection_type == "vpn" && before.contains_key("vpn");
    if supports_vpn_options != edit.supports_vpn_options {
        return Err(super::Error::new(
            "revalidate VPN profile edit",
            "the profile capabilities changed outside System Settings",
        ));
    }
    let expected = EditableValues::from_edit(edit);
    let before_unrelated = settings_without_editable_values(before.clone(), supports_vpn_options);

    let command_result = run_modify_command(edit);
    let (after, after_unsaved) =
        stable_profile_settings(&connection, &edit.id.profile).map_err(|error| {
            super::Error::new(
                "verify VPN profile edit",
                format!("NetworkManager could not confirm the saved profile: {error}"),
            )
        })?;
    let after_type = validate_profile_type(&edit.id.profile, &after)?;
    if after_unsaved || after_type != connection_type {
        return Err(super::Error::new(
            "verify VPN profile edit",
            "the profile authority changed while the edit was being saved",
        ));
    }
    let actual = EditableValues::from_settings(&after, supports_vpn_options)?;
    if actual != expected {
        return Err(command_result.err().unwrap_or_else(|| {
            super::Error::new(
                "verify VPN profile edit",
                "NetworkManager did not retain every requested value; refresh before retrying",
            )
        }));
    }
    if settings_without_editable_values(after, supports_vpn_options) != before_unrelated {
        return Err(super::Error::new(
            "verify VPN profile edit",
            "other profile fields changed concurrently; the requested values may be saved, so refresh before editing again",
        ));
    }

    let snapshot = super::linux_vpn_snapshot()?;
    let saved = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == edit.id.profile)
        .ok_or_else(|| {
            super::Error::new(
                "verify VPN profile edit",
                "the edited profile disappeared from NetworkManager",
            )
        })?;
    if saved.name != edit.name {
        return Err(super::Error::new(
            "verify VPN profile edit",
            "the refreshed profile name did not match the saved value",
        ));
    }
    Ok(snapshot)
}

#[cfg(not(target_os = "macos"))]
fn run_modify_command(edit: &VpnProfileEdit) -> Result<(), super::Error> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
    let mut command = Command::new("nmcli");
    command.args([
        "--wait",
        "15",
        "connection",
        "modify",
        "uuid",
        edit.id.profile.uuid.as_str(),
        "connection.id",
        edit.name.as_str(),
    ]);
    if edit.supports_vpn_options {
        let timeout = edit.timeout.to_string();
        command.args([
            "vpn.user-name",
            edit.username.as_str(),
            "vpn.persistent",
            if edit.persistent { "yes" } else { "no" },
            "vpn.timeout",
            timeout.as_str(),
        ]);
    }
    let mut child = command
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            super::Error::new(
                "save VPN profile",
                "NetworkManager's profile editor could not be started",
            )
        })?;
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => {
                return Err(super::Error::new(
                    "save VPN profile",
                    "NetworkManager rejected the profile edit or authorization was denied",
                ));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(super::Error::new(
                    "save VPN profile",
                    "NetworkManager did not finish the profile edit within 20 seconds",
                ));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(super::Error::new(
                    "save VPN profile",
                    "NetworkManager's profile editor could not be monitored",
                ));
            }
        }
    }
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Debug, PartialEq, Eq)]
struct EditableValues {
    name: String,
    username: String,
    persistent: bool,
    timeout: u32,
}

#[cfg(any(not(target_os = "macos"), test))]
impl EditableValues {
    fn from_edit(edit: &VpnProfileEdit) -> Self {
        Self {
            name: edit.name.clone(),
            username: edit.username.clone(),
            persistent: edit.persistent,
            timeout: edit.timeout,
        }
    }

    fn from_settings(
        settings: &SettingsMap,
        supports_vpn_options: bool,
    ) -> Result<Self, super::Error> {
        let connection = settings.get("connection").ok_or_else(|| {
            super::Error::new("verify VPN profile edit", "connection settings are missing")
        })?;
        let name = super::property_string(connection, "id").ok_or_else(|| {
            super::Error::new("verify VPN profile edit", "the connection name is missing")
        })?;
        let vpn = settings.get("vpn");
        Ok(Self {
            name,
            username: supports_vpn_options
                .then(|| vpn.and_then(|values| super::property_string(values, "user-name")))
                .flatten()
                .unwrap_or_default(),
            persistent: supports_vpn_options
                && vpn
                    .and_then(|values| super::property::<bool>(values, "persistent"))
                    .unwrap_or(false),
            timeout: if supports_vpn_options {
                vpn.and_then(|values| super::property::<u32>(values, "timeout"))
                    .unwrap_or(0)
            } else {
                0
            },
        })
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn settings_without_editable_values(
    mut settings: SettingsMap,
    supports_vpn_options: bool,
) -> SettingsMap {
    if let Some(connection) = settings.get_mut("connection") {
        connection.remove("id");
    }
    if supports_vpn_options {
        if let Some(vpn) = settings.get_mut("vpn") {
            vpn.remove("user-name");
            vpn.remove("persistent");
            vpn.remove("timeout");
        }
    }
    settings
}

#[cfg(not(target_os = "macos"))]
fn require_exact_record(
    connection: &zbus::blocking::Connection,
    id: &super::VpnProfileId,
) -> Result<(), super::Error> {
    super::linux_vpn_records(connection)?
        .into_iter()
        .any(|record| record.profile.id == *id)
        .then_some(())
        .ok_or_else(|| super::Error::new("find VPN profile", "the profile no longer exists"))
}

#[cfg(not(target_os = "macos"))]
fn stable_profile_settings(
    connection: &zbus::blocking::Connection,
    id: &super::VpnProfileId,
) -> Result<(SettingsMap, bool), super::Error> {
    const MAX_ATTEMPTS: usize = 3;
    let authority = settings_proxy(connection)?;
    for _ in 0..MAX_ATTEMPTS {
        let before = authority
            .get_property::<u64>("VersionId")
            .map_err(|error| {
                super::Error::new("version VPN profile inventory", error.to_string())
            })?;
        let profile = profile_proxy(connection, &id.object_path)?;
        let settings = profile
            .call::<_, _, SettingsMap>("GetSettings", &())
            .map_err(|error| super::Error::new("read VPN profile", error.to_string()))?;
        let unsaved = profile
            .get_property::<bool>("Unsaved")
            .map_err(|error| super::Error::new("read VPN profile state", error.to_string()))?;
        let after = authority
            .get_property::<u64>("VersionId")
            .map_err(|error| {
                super::Error::new("version VPN profile inventory", error.to_string())
            })?;
        if before == after {
            if settings
                .get("connection")
                .and_then(|values| super::property_string(values, "uuid"))
                .as_deref()
                != Some(id.uuid.as_str())
            {
                return Err(super::Error::new(
                    "read VPN profile",
                    "the profile UUID no longer matches its object",
                ));
            }
            return Ok((settings, unsaved));
        }
    }
    Err(super::Error::new(
        "read VPN profile",
        "the profile inventory kept changing; try again after other network edits finish",
    ))
}

#[cfg(not(target_os = "macos"))]
fn validate_profile_type(
    id: &super::VpnProfileId,
    settings: &SettingsMap,
) -> Result<String, super::Error> {
    let connection = settings.get("connection").ok_or_else(|| {
        super::Error::new("validate VPN profile", "connection settings are missing")
    })?;
    let connection_type = super::property_string(connection, "type").unwrap_or_default();
    if !super::is_vpn_connection_type(&connection_type) {
        return Err(super::Error::new(
            "validate VPN profile",
            "the selected object is no longer a VPN profile",
        ));
    }
    if super::property_string(connection, "uuid").as_deref() != Some(id.uuid.as_str()) {
        return Err(super::Error::new(
            "validate VPN profile",
            "the profile UUID changed",
        ));
    }
    Ok(connection_type)
}

#[cfg(not(target_os = "macos"))]
fn settings_proxy(
    connection: &zbus::blocking::Connection,
) -> Result<zbus::blocking::Proxy<'_>, super::Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .map_err(|error| super::Error::new("open VPN profile inventory", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn profile_proxy<'a>(
    connection: &'a zbus::blocking::Connection,
    path: &'a str,
) -> Result<zbus::blocking::Proxy<'a>, super::Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        path,
        "org.freedesktop.NetworkManager.Settings.Connection",
    )
    .map_err(|error| super::Error::new("open VPN profile", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{OwnedValue, Str};

    fn value(value: &str) -> OwnedValue {
        OwnedValue::from(Str::from(value))
    }

    fn settings() -> SettingsMap {
        HashMap::from([
            (
                "connection".to_string(),
                HashMap::from([
                    ("id".to_string(), value("Work VPN")),
                    (
                        "uuid".to_string(),
                        value("12345678-1234-1234-1234-123456789abc"),
                    ),
                    ("type".to_string(), value("vpn")),
                    ("autoconnect".to_string(), OwnedValue::from(false)),
                ]),
            ),
            (
                "vpn".to_string(),
                HashMap::from([
                    (
                        "service-type".to_string(),
                        value("org.freedesktop.NetworkManager.openvpn"),
                    ),
                    ("user-name".to_string(), value("jacob")),
                    ("persistent".to_string(), OwnedValue::from(true)),
                    ("timeout".to_string(), OwnedValue::from(30_u32)),
                    ("remote".to_string(), value("vpn.example.test")),
                ]),
            ),
        ])
    }

    fn configuration() -> VpnProfileConfiguration {
        VpnProfileConfiguration {
            id: VpnProfileEditId {
                profile: super::super::VpnProfileId {
                    object_path: "/org/freedesktop/NetworkManager/Settings/42".to_string(),
                    uuid: "12345678-1234-1234-1234-123456789abc".to_string(),
                },
                settings: settings(),
            },
            name: "Work VPN".to_string(),
            service: "OpenVPN".to_string(),
            username: "jacob".to_string(),
            persistent: true,
            timeout: 30,
            supports_vpn_options: true,
        }
    }

    #[test]
    fn edit_identity_redacts_profile_and_settings() {
        let debug = format!("{:?}", configuration().id);
        assert_eq!(
            debug,
            "VpnProfileEditId { profile: \"<redacted>\", settings: \"<redacted>\" }"
        );
        assert!(!debug.contains("Settings/42"));
        assert!(!debug.contains("12345678"));
    }

    #[test]
    fn edit_debug_redacts_account_name() {
        let configuration = configuration();
        let edit = VpnProfileEdit::new(&configuration, "Work VPN", "private-user", true, 30)
            .expect("valid edit");
        let configuration_debug = format!("{configuration:?}");
        let edit_debug = format!("{edit:?}");
        assert!(!configuration_debug.contains("jacob"));
        assert!(!edit_debug.contains("private-user"));
        assert!(configuration_debug.contains("<redacted>"));
        assert!(edit_debug.contains("<redacted>"));
    }

    #[test]
    fn edit_validation_normalizes_non_secret_fields() {
        let edit = VpnProfileEdit::new(
            &configuration(),
            "  Personal VPN  ",
            " user@example.test ",
            false,
            0,
        )
        .expect("valid edit");
        assert_eq!(edit.name, "Personal VPN");
        assert_eq!(edit.username, "user@example.test");
        assert!(!edit.persistent);
        assert_eq!(edit.timeout, 0);
    }

    #[test]
    fn editable_projection_reads_only_the_supported_typed_fields() {
        let configuration = configuration();
        let edit = VpnProfileEdit::new(
            &configuration,
            &configuration.name,
            &configuration.username,
            configuration.persistent,
            configuration.timeout,
        )
        .expect("valid edit");
        assert_eq!(
            EditableValues::from_settings(&settings(), true).expect("typed fields"),
            EditableValues::from_edit(&edit)
        );
    }

    #[test]
    fn edit_validation_rejects_empty_or_controlled_values() {
        assert_eq!(
            VpnProfileEdit::new(&configuration(), " \n ", "jacob", true, 30)
                .expect_err("empty name"),
            VpnEditValidationError::InvalidName
        );
        assert_eq!(
            VpnProfileEdit::new(&configuration(), "Work", "bad\nuser", true, 30)
                .expect_err("controlled username"),
            VpnEditValidationError::InvalidUsername
        );
    }

    #[test]
    fn unrelated_projection_retains_plugin_data_and_autoconnect() {
        let stripped = settings_without_editable_values(settings(), true);
        let connection = stripped.get("connection").expect("connection");
        let vpn = stripped.get("vpn").expect("vpn");
        assert!(!connection.contains_key("id"));
        assert!(!vpn.contains_key("user-name"));
        assert!(!vpn.contains_key("persistent"));
        assert!(!vpn.contains_key("timeout"));
        assert_eq!(
            super::super::property::<bool>(connection, "autoconnect"),
            Some(false)
        );
        assert_eq!(
            super::super::property_string(vpn, "remote").as_deref(),
            Some("vpn.example.test")
        );
        assert_eq!(
            super::super::property_string(vpn, "service-type").as_deref(),
            Some("org.freedesktop.NetworkManager.openvpn")
        );
    }

    #[test]
    fn wireguard_configuration_rejects_vpn_only_changes() {
        let mut configuration = configuration();
        configuration.supports_vpn_options = false;
        configuration.username.clear();
        configuration.persistent = false;
        configuration.timeout = 0;
        assert_eq!(
            VpnProfileEdit::new(&configuration, "WireGuard", "someone", false, 0)
                .expect_err("VPN-only field"),
            VpnEditValidationError::UnsupportedVpnOptions
        );
        VpnProfileEdit::new(&configuration, "WireGuard", "", false, 0)
            .expect("renaming remains supported");
    }
}
