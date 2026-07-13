use std::fmt;

#[cfg(not(target_os = "macos"))]
type SettingsMap = std::collections::HashMap<
    String,
    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
>;

#[cfg(target_os = "macos")]
type SettingsMap = ();

#[derive(Clone)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub struct VpnDeletePreviewId {
    profile: super::VpnProfileId,
    settings: SettingsMap,
}

impl fmt::Debug for VpnDeletePreviewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VpnDeletePreviewId")
            .field("profile", &"<redacted>")
            .field("settings", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct VpnDeletePreview {
    pub id: VpnDeletePreviewId,
    pub name: String,
    pub service: String,
    pub will_disconnect: bool,
}

pub(super) fn prepare(id: &super::VpnProfileId) -> Result<VpnDeletePreview, super::Error> {
    #[cfg(not(target_os = "macos"))]
    {
        linux_prepare(id)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = id;
        Err(super::Error::new(
            "prepare VPN deletion",
            "profile deletion is available in the supported Linux session",
        ))
    }
}

pub(super) fn delete(preview: &VpnDeletePreviewId) -> Result<super::VpnSnapshot, super::Error> {
    #[cfg(not(target_os = "macos"))]
    {
        linux_delete(preview)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = preview;
        Err(super::Error::new(
            "delete VPN profile",
            "profile deletion is available in the supported Linux session",
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn linux_prepare(id: &super::VpnProfileId) -> Result<VpnDeletePreview, super::Error> {
    let connection = super::system_connection("connect to NetworkManager for VPN deletion")?;
    let record = exact_record(&connection, id)?;
    let (settings, unsaved) = stable_profile_settings(&connection, id)?;
    if unsaved {
        return Err(super::Error::new(
            "prepare VPN deletion",
            "the profile is temporary and is still owned by another editor",
        ));
    }
    validate_profile_type(id, &settings)?;
    let connection_setting = settings.get("connection").ok_or_else(|| {
        super::Error::new("prepare VPN deletion", "connection settings are missing")
    })?;
    let connection_type = super::property_string(connection_setting, "type").unwrap_or_default();
    let service_type = settings
        .get("vpn")
        .and_then(|vpn| super::property_string(vpn, "service-type"));
    let name = super::property_string(connection_setting, "id")
        .filter(|name| !name.is_empty() && name.len() <= 256)
        .unwrap_or_else(|| "VPN Connection".to_string());
    let service = super::vpn_service_label(&connection_type, service_type.as_deref());
    Ok(VpnDeletePreview {
        id: VpnDeletePreviewId {
            profile: id.clone(),
            settings,
        },
        name,
        service,
        will_disconnect: record.active_path.is_some(),
    })
}

#[cfg(not(target_os = "macos"))]
fn linux_delete(preview: &VpnDeletePreviewId) -> Result<super::VpnSnapshot, super::Error> {
    let connection = super::system_connection("connect to NetworkManager for VPN deletion")?;
    let mut record = exact_record(&connection, &preview.profile)?;
    require_unchanged_persistent_profile(&connection, preview)?;
    if let Some(active_path) = record.active_path.take() {
        if !super::exact_active_vpn(&connection, &preview.profile, &active_path)? {
            return Err(super::Error::new(
                "delete VPN profile",
                "the active connection identity changed before disconnection",
            ));
        }
        super::manager_proxy(&connection)?
            .call::<_, _, ()>("DeactivateConnection", &(active_path.clone(),))
            .map_err(|error| {
                super::Error::new("disconnect VPN before deletion", error.to_string())
            })?;
        super::wait_for_vpn_deactivation(&connection, &preview.profile, &active_path).map_err(
            |error| super::Error::new("disconnect VPN before deletion", error.to_string()),
        )?;
    }

    let record = exact_record(&connection, &preview.profile)?;
    if record.active_path.is_some() {
        return Err(super::Error::new(
            "delete VPN profile",
            "the profile became active again and was left installed",
        ));
    }
    let profile = require_unchanged_persistent_profile(&connection, preview)?;
    profile
        .call::<_, _, ()>("Delete", &())
        .map_err(|error| super::Error::new("delete VPN profile", error.to_string()))?;

    let deadline = std::time::Instant::now() + super::VPN_DEACTIVATION_TIMEOUT;
    loop {
        if !connection_paths(&connection)?
            .iter()
            .any(|path| path.as_str() == preview.profile.object_path)
        {
            let snapshot = super::linux_vpn_snapshot()?;
            if !snapshot
                .profiles
                .iter()
                .any(|profile| profile.id == preview.profile)
            {
                return Ok(snapshot);
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(super::Error::new(
                "delete VPN profile",
                "NetworkManager did not confirm removal within 10 seconds",
            ));
        }
        std::thread::sleep(super::VPN_STATE_INTERVAL);
    }
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
fn require_unchanged_persistent_profile<'a>(
    connection: &'a zbus::blocking::Connection,
    preview: &'a VpnDeletePreviewId,
) -> Result<zbus::blocking::Proxy<'a>, super::Error> {
    let (settings, unsaved) = stable_profile_settings(connection, &preview.profile)?;
    if unsaved || settings != preview.settings {
        return Err(super::Error::new(
            "revalidate VPN deletion",
            "the profile changed after confirmation and was left installed",
        ));
    }
    validate_profile_type(&preview.profile, &settings)?;
    profile_proxy(connection, &preview.profile.object_path)
}

#[cfg(not(target_os = "macos"))]
fn stable_profile_settings(
    connection: &zbus::blocking::Connection,
    id: &super::VpnProfileId,
) -> Result<(SettingsMap, bool), super::Error> {
    const MAX_ATTEMPTS: usize = 3;
    let settings_authority = settings_proxy(connection)?;
    for _ in 0..MAX_ATTEMPTS {
        let before = settings_authority
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
        let after = settings_authority
            .get_property::<u64>("VersionId")
            .map_err(|error| {
                super::Error::new("version VPN profile inventory", error.to_string())
            })?;
        if before == after {
            let uuid = settings
                .get("connection")
                .and_then(|setting| super::property_string(setting, "uuid"));
            if uuid.as_deref() != Some(id.uuid.as_str()) {
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
) -> Result<(), super::Error> {
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
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn connection_paths(
    connection: &zbus::blocking::Connection,
) -> Result<Vec<zbus::zvariant::OwnedObjectPath>, super::Error> {
    settings_proxy(connection)?
        .call::<_, _, Vec<zbus::zvariant::OwnedObjectPath>>("ListConnections", &())
        .map_err(|error| super::Error::new("list VPN profiles", error.to_string()))
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

    #[test]
    fn deletion_preview_identity_redacts_profile_and_settings() {
        let preview = VpnDeletePreviewId {
            profile: super::super::VpnProfileId {
                object_path: "/org/freedesktop/NetworkManager/Settings/42".to_string(),
                uuid: "12345678-1234-1234-1234-123456789abc".to_string(),
            },
            settings: Default::default(),
        };
        let debug = format!("{preview:?}");
        assert_eq!(
            debug,
            "VpnDeletePreviewId { profile: \"<redacted>\", settings: \"<redacted>\" }"
        );
        assert!(!debug.contains("Settings/42"));
        assert!(!debug.contains("12345678"));
    }
}
