use std::fmt;

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashMap;

#[cfg(any(not(target_os = "macos"), test))]
type SettingsMap = HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>;

#[cfg(all(target_os = "macos", not(test)))]
type SettingsMap = ();

#[derive(Clone)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct VpnSecretClearPreviewId {
    profile: super::VpnProfileId,
    settings: SettingsMap,
}

impl fmt::Debug for VpnSecretClearPreviewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VpnSecretClearPreviewId")
            .field("profile", &"<redacted>")
            .field("settings", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct VpnSecretClearPreview {
    pub id: VpnSecretClearPreviewId,
    pub name: String,
    pub service: String,
    pub currently_connected: bool,
}

pub(super) fn prepare(id: &super::VpnProfileId) -> Result<VpnSecretClearPreview, super::Error> {
    #[cfg(not(target_os = "macos"))]
    {
        linux_prepare(id)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = id;
        Err(super::Error::new(
            "prepare saved VPN authentication removal",
            "saved VPN authentication is managed in the supported Linux session",
        ))
    }
}

pub(super) fn clear(preview: &VpnSecretClearPreviewId) -> Result<super::VpnSnapshot, super::Error> {
    #[cfg(not(target_os = "macos"))]
    {
        linux_clear(preview)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = preview;
        Err(super::Error::new(
            "forget saved VPN authentication",
            "saved VPN authentication is managed in the supported Linux session",
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn linux_prepare(id: &super::VpnProfileId) -> Result<VpnSecretClearPreview, super::Error> {
    let connection = super::system_connection("connect to NetworkManager for VPN authentication")?;
    let record = exact_record(&connection, id)?;
    let (settings, unsaved) = stable_profile_settings(&connection, id)?;
    if unsaved {
        return Err(super::Error::new(
            "prepare saved VPN authentication removal",
            "the profile is temporary and still owned by another editor",
        ));
    }
    validate_plugin_profile(id, &settings)?;
    let connection_setting = settings.get("connection").ok_or_else(|| {
        super::Error::new(
            "prepare saved VPN authentication removal",
            "connection settings are missing",
        )
    })?;
    let name = super::property_string(connection_setting, "id")
        .filter(|name| !name.is_empty() && name.chars().count() <= 256)
        .unwrap_or_else(|| "VPN Connection".to_string());
    let service_type = settings
        .get("vpn")
        .and_then(|vpn| super::property_string(vpn, "service-type"));
    Ok(VpnSecretClearPreview {
        id: VpnSecretClearPreviewId {
            profile: id.clone(),
            settings,
        },
        name,
        service: super::vpn_service_label("vpn", service_type.as_deref()),
        currently_connected: record.active_path.is_some(),
    })
}

#[cfg(not(target_os = "macos"))]
fn linux_clear(preview: &VpnSecretClearPreviewId) -> Result<super::VpnSnapshot, super::Error> {
    let connection = super::system_connection("connect to NetworkManager for VPN authentication")?;
    exact_record(&connection, &preview.profile)?;
    let profile = require_unchanged_plugin_profile(&connection, preview)?;
    profile
        .call::<_, _, ()>("ClearSecrets", &())
        .map_err(|error| super::Error::new("forget saved VPN authentication", error.to_string()))?;

    let (settings, unsaved) = stable_profile_settings(&connection, &preview.profile)?;
    if unsaved || settings != preview.settings {
        return Err(super::Error::new(
            "verify saved VPN authentication removal",
            "the profile changed while NetworkManager was clearing authentication; refresh before making another change",
        ));
    }
    validate_plugin_profile(&preview.profile, &settings)?;
    exact_record(&connection, &preview.profile)?;
    let snapshot = super::linux_vpn_snapshot()?;
    if !snapshot
        .profiles
        .iter()
        .any(|profile| profile.id == preview.profile)
    {
        return Err(super::Error::new(
            "verify saved VPN authentication removal",
            "the profile disappeared after NetworkManager cleared authentication",
        ));
    }
    Ok(snapshot)
}

#[cfg(not(target_os = "macos"))]
fn exact_record(
    connection: &zbus::blocking::Connection,
    id: &super::VpnProfileId,
) -> Result<super::VpnRecord, super::Error> {
    super::linux_vpn_records(connection)?
        .into_iter()
        .find(|record| record.profile.id == *id)
        .ok_or_else(|| super::Error::new("find VPN profile", "the profile no longer exists"))
}

#[cfg(not(target_os = "macos"))]
fn require_unchanged_plugin_profile<'a>(
    connection: &'a zbus::blocking::Connection,
    preview: &'a VpnSecretClearPreviewId,
) -> Result<zbus::blocking::Proxy<'a>, super::Error> {
    let (settings, unsaved) = stable_profile_settings(connection, &preview.profile)?;
    if unsaved || settings != preview.settings {
        return Err(super::Error::new(
            "revalidate saved VPN authentication removal",
            "the profile changed after confirmation and its authentication was left untouched",
        ));
    }
    validate_plugin_profile(&preview.profile, &settings)?;
    profile_proxy(connection, &preview.profile.object_path)
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

#[cfg(any(not(target_os = "macos"), test))]
fn validate_plugin_profile(
    id: &super::VpnProfileId,
    settings: &SettingsMap,
) -> Result<(), super::Error> {
    let connection = settings.get("connection").ok_or_else(|| {
        super::Error::new("validate VPN profile", "connection settings are missing")
    })?;
    if super::property_string(connection, "uuid").as_deref() != Some(id.uuid.as_str()) {
        return Err(super::Error::new(
            "validate VPN profile",
            "the profile UUID changed",
        ));
    }
    if super::property_string(connection, "type").as_deref() != Some("vpn")
        || !settings.contains_key("vpn")
    {
        return Err(super::Error::new(
            "forget saved VPN authentication",
            "only plugin VPN authentication can be cleared safely; native WireGuard private keys are never removed here",
        ));
    }
    Ok(())
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

    fn profile_id() -> super::super::VpnProfileId {
        super::super::VpnProfileId {
            object_path: "/org/freedesktop/NetworkManager/Settings/42".to_string(),
            uuid: "12345678-1234-1234-1234-123456789abc".to_string(),
        }
    }

    fn settings(connection_type: &str) -> SettingsMap {
        let mut settings = HashMap::from([(
            "connection".to_string(),
            HashMap::from([
                ("id".to_string(), value("Work VPN")),
                ("uuid".to_string(), value(&profile_id().uuid)),
                ("type".to_string(), value(connection_type)),
            ]),
        )]);
        if connection_type == "vpn" {
            settings.insert(
                "vpn".to_string(),
                HashMap::from([(
                    "service-type".to_string(),
                    value("org.freedesktop.NetworkManager.openvpn"),
                )]),
            );
        }
        settings
    }

    #[test]
    fn clear_preview_identity_redacts_profile_and_settings() {
        let preview = VpnSecretClearPreviewId {
            profile: profile_id(),
            settings: settings("vpn"),
        };
        let debug = format!("{preview:?}");
        assert_eq!(
            debug,
            "VpnSecretClearPreviewId { profile: \"<redacted>\", settings: \"<redacted>\" }"
        );
        assert!(!debug.contains("Settings/42"));
        assert!(!debug.contains("12345678"));
    }

    #[test]
    fn clear_accepts_plugin_vpn_but_rejects_native_wireguard() {
        validate_plugin_profile(&profile_id(), &settings("vpn")).expect("plugin VPN");
        let error = validate_plugin_profile(&profile_id(), &settings("wireguard"))
            .expect_err("native private key must remain untouched");
        assert!(error.to_string().contains("WireGuard private keys"));
    }

    #[test]
    fn clear_rejects_a_replaced_profile_uuid() {
        let mut settings = settings("vpn");
        settings.get_mut("connection").expect("connection").insert(
            "uuid".to_string(),
            value("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        );
        let error = validate_plugin_profile(&profile_id(), &settings)
            .expect_err("replacement identity must fail");
        assert!(error.to_string().contains("UUID changed"));
    }
}
