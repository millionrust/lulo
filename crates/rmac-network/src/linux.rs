//! NetworkManager Wi-Fi, wired-network, and VPN backend.

use super::*;

#[cfg(not(target_os = "macos"))]
pub(super) const NETWORK_MANAGER_SERVICE: &str = "org.freedesktop.NetworkManager";
#[cfg(not(target_os = "macos"))]
pub(super) const WIFI_WATCH_RECONNECT_DELAY: Duration = Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
pub(super) const WIFI_WATCH_QUIET_PERIOD: Duration = Duration::from_millis(75);

#[cfg(not(target_os = "macos"))]
pub(super) async fn system_watch_wifi(
    sender: async_channel::Sender<WifiWatchEvent>,
) -> Result<(), Error> {
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
#[cfg(not(target_os = "macos"))]
pub(super) async fn watch_wifi_once(
    sender: &async_channel::Sender<WifiWatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = rmac_dbus::system()
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
pub(super) fn read_wifi_signal(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<(), Error> {
    match message {
        Some(Ok(_)) => Ok(()),
        Some(Err(error)) => Err(Error::new("read Wi-Fi change", error.to_string())),
        None => Err(Error::new("read Wi-Fi change", "the signal stream ended")),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn read_network_manager_owner(
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
pub(super) fn network_manager_owner_availability(name: &str, new_owner: &str) -> Option<bool> {
    (name == "org.freedesktop.NetworkManager").then_some(!new_owner.is_empty())
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn publish_wifi_changed(
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
pub(super) async fn publish_wifi_unavailable(
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

    fn connect_enterprise(
        &self,
        network: &WifiNetworkId,
        credentials: WifiEnterpriseCredentials,
        cancellation: &WifiCancellation,
    ) -> Result<WifiSnapshot, Error> {
        linux_connect_enterprise_wifi(network, credentials, cancellation)
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_snapshot() -> Result<WifiSnapshot, Error> {
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
pub(super) struct WifiAccessPointRecord {
    id: WifiNetworkId,
    path: zbus::zvariant::OwnedObjectPath,
    strength: u8,
    connected: bool,
}

#[cfg(not(target_os = "macos"))]
pub(super) struct WifiProfileRecord {
    id: WifiNetworkId,
    connection_path: zbus::zvariant::OwnedObjectPath,
    timestamp: u64,
}

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_wifi_access_points(
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
pub(super) fn linux_wifi_profiles(
    connection: &zbus::blocking::Connection,
) -> Result<Vec<WifiProfileRecord>, Error> {
    use zbus::zvariant::OwnedValue;

    let paths = wifi_connection_paths(connection)?;
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
pub(super) fn wifi_profile_identity(
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
pub(super) fn exact_created_wifi_profile(
    settings: &HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>,
    profile_uuid: &str,
    network: &WifiNetworkId,
) -> bool {
    wifi_profile_identity(settings).is_some_and(|(identity, _)| identity == *network)
        && settings
            .get("connection")
            .and_then(|connection| property_string(connection, "uuid"))
            .as_deref()
            == Some(profile_uuid)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn wifi_security_from_access_point(flags: u32, wpa: u32, rsn: u32) -> WifiSecurity {
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
pub(super) fn wifi_security_from_profile(
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
pub(super) fn linux_connect_wifi(network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
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
pub(super) fn linux_connect_wifi_with_password(
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
    let profile_uuid = uuid::Uuid::new_v4().to_string();
    let template = secret_agent::personal_connection_template(network, &profile_uuid)
        .map_err(|error| Error::new("prepare protected Wi-Fi", error.to_string()))?;
    let agent = secret_agent::RegisteredSecretAgent::register_personal(
        network.clone(),
        profile_uuid.clone(),
        password,
    )
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

    if linux_wifi_profiles(connection)?
        .into_iter()
        .any(|profile| network.matches_profile(&profile.id))
    {
        return Err(Error::new(
            "connect protected Wi-Fi",
            "the network became known; select it again to use the saved profile",
        ));
    }
    let (profile_path, active_path) = manager
        .call::<_, _, (OwnedObjectPath, OwnedObjectPath)>(
            "AddAndActivateConnection",
            &(template, device.clone(), access_point.path),
        )
        .map_err(|error| Error::new("connect protected Wi-Fi", error.to_string()))?;
    wait_for_new_wifi_activation(
        connection,
        &profile_path,
        &profile_uuid,
        &active_path,
        &device,
        network,
        Some(cancellation),
    )
}

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_connect_enterprise_wifi(
    network: &WifiNetworkId,
    credentials: WifiEnterpriseCredentials,
    cancellation: &WifiCancellation,
) -> Result<WifiSnapshot, Error> {
    use zbus::zvariant::OwnedObjectPath;

    if network.security != WifiSecurity::Enterprise {
        return Err(Error::new(
            "connect enterprise Wi-Fi",
            "the selected network does not use enterprise security",
        ));
    }
    if cancellation.is_cancelled() {
        return Err(Error::cancelled("connect enterprise Wi-Fi"));
    }
    let profile_uuid = uuid::Uuid::new_v4().to_string();
    let template =
        secret_agent::enterprise_connection_template(network, &profile_uuid, &credentials)
            .map_err(|error| Error::new("prepare enterprise Wi-Fi", error.to_string()))?;
    let agent = secret_agent::RegisteredSecretAgent::register_enterprise(
        network.clone(),
        profile_uuid.clone(),
        credentials,
    )
    .map_err(|error| Error::new("register Wi-Fi secret agent", error.to_string()))?;
    let connection = agent.connection();
    let manager = manager_proxy(connection)?;
    let enabled = manager
        .get_property::<bool>("WirelessEnabled")
        .map_err(|error| Error::new("read Wi-Fi power", error.to_string()))?;
    if !enabled {
        return Err(Error::new(
            "connect enterprise Wi-Fi",
            "Wi-Fi is turned off",
        ));
    }
    let device = wifi_device_path(connection)?
        .ok_or_else(|| Error::new("connect enterprise Wi-Fi", "no Wi-Fi adapter found"))?;
    let access_point = linux_wifi_access_points(connection, &device)?
        .into_iter()
        .filter(|access_point| access_point.id == *network)
        .max_by_key(|access_point| (access_point.connected, access_point.strength))
        .ok_or_else(|| {
            Error::new(
                "connect enterprise Wi-Fi",
                "the network is no longer in range",
            )
        })?;
    if access_point.connected {
        return linux_snapshot();
    }
    if cancellation.is_cancelled() {
        return Err(Error::cancelled("connect enterprise Wi-Fi"));
    }
    if linux_wifi_profiles(connection)?
        .into_iter()
        .any(|profile| network.matches_profile(&profile.id))
    {
        return Err(Error::new(
            "connect enterprise Wi-Fi",
            "the network became known; select it again to use the saved profile",
        ));
    }
    let (profile_path, active_path) = manager
        .call::<_, _, (OwnedObjectPath, OwnedObjectPath)>(
            "AddAndActivateConnection",
            &(template, device.clone(), access_point.path),
        )
        .map_err(|error| Error::new("connect enterprise Wi-Fi", error.to_string()))?;
    wait_for_new_wifi_activation(
        connection,
        &profile_path,
        &profile_uuid,
        &active_path,
        &device,
        network,
        Some(cancellation),
    )
}

#[cfg(not(target_os = "macos"))]
pub(super) fn wait_for_new_wifi_activation(
    connection: &zbus::blocking::Connection,
    profile_path: &zbus::zvariant::OwnedObjectPath,
    profile_uuid: &str,
    active_path: &zbus::zvariant::OwnedObjectPath,
    device: &zbus::zvariant::OwnedObjectPath,
    network: &WifiNetworkId,
    cancellation: Option<&WifiCancellation>,
) -> Result<WifiSnapshot, Error> {
    match wait_for_wifi_activation(connection, active_path, device, network, cancellation) {
        Ok(snapshot) => Ok(snapshot),
        Err(mut failure) => {
            if let Ok(manager) = manager_proxy(connection) {
                let _ = manager.call::<_, _, ()>("DeactivateConnection", &(active_path.clone(),));
            }
            if delete_exact_created_wifi_profile(connection, profile_path, profile_uuid, network)
                .is_err()
            {
                failure
                    .detail
                    .push_str("; the new saved profile could not be removed safely");
            }
            Err(failure)
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn delete_exact_created_wifi_profile(
    connection: &zbus::blocking::Connection,
    profile_path: &zbus::zvariant::OwnedObjectPath,
    profile_uuid: &str,
    network: &WifiNetworkId,
) -> Result<(), Error> {
    if !wifi_connection_paths(connection)?
        .iter()
        .any(|candidate| candidate == profile_path)
    {
        return Ok(());
    }
    let proxy = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        profile_path.as_str(),
        "org.freedesktop.NetworkManager.Settings.Connection",
    )
    .map_err(|error| Error::new("open failed Wi-Fi profile", error.to_string()))?;
    let settings = proxy
        .call::<_, _, HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>>(
            "GetSettings",
            &(),
        )
        .map_err(|error| Error::new("read failed Wi-Fi profile", error.to_string()))?;
    if !exact_created_wifi_profile(&settings, profile_uuid, network) {
        return Err(Error::new(
            "remove failed Wi-Fi profile",
            "the created profile identity changed",
        ));
    }
    proxy
        .call::<_, _, ()>("Delete", &())
        .map_err(|error| Error::new("remove failed Wi-Fi profile", error.to_string()))?;
    for _ in 0..20 {
        if !wifi_connection_paths(connection)?
            .iter()
            .any(|candidate| candidate == profile_path)
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(Error::new(
        "remove failed Wi-Fi profile",
        "NetworkManager did not confirm profile removal",
    ))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn wifi_connection_paths(
    connection: &zbus::blocking::Connection,
) -> Result<Vec<zbus::zvariant::OwnedObjectPath>, Error> {
    let settings = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .map_err(|error| Error::new("open saved Wi-Fi connections", error.to_string()))?;
    settings
        .call::<_, _, Vec<zbus::zvariant::OwnedObjectPath>>("ListConnections", &())
        .map_err(|error| Error::new("list saved Wi-Fi connections", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_forget_wifi(network: &WifiNetworkId) -> Result<WifiSnapshot, Error> {
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
pub(super) fn device_active_connection(
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
pub(super) fn wait_for_wifi_activation(
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
pub(super) fn wifi_activation_failure(
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
        Some(8..=11)
            if matches!(
                network.security,
                WifiSecurity::Personal(_) | WifiSecurity::Enterprise
            ) =>
        {
            "the credentials may be incorrect or network authentication failed"
        }
        Some(53) => "the network is no longer in range",
        _ => "NetworkManager rejected the connection",
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_network_snapshot() -> Result<NetworkSnapshot, Error> {
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
pub(super) struct VpnRecord {
    pub(super) profile: VpnProfile,
    connection_path: zbus::zvariant::OwnedObjectPath,
    pub(super) active_path: Option<zbus::zvariant::OwnedObjectPath>,
}

#[cfg(not(target_os = "macos"))]
pub(super) const VPN_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(not(target_os = "macos"))]
pub(super) const VPN_DEACTIVATION_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(target_os = "macos"))]
pub(super) const VPN_STATE_INTERVAL: Duration = Duration::from_millis(250);

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_vpn_snapshot() -> Result<VpnSnapshot, Error> {
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
pub(super) fn linux_set_vpn_enabled(
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
pub(super) fn wait_for_vpn_activation(
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
pub(super) fn stop_exact_vpn_activation(
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
pub(super) fn wait_for_vpn_deactivation(
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
pub(super) fn exact_active_vpn(
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
pub(super) fn linux_vpn_records(
    connection: &zbus::blocking::Connection,
) -> Result<Vec<VpnRecord>, Error> {
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
pub(super) fn active_connection_name(
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
pub(super) fn read_ip_configuration(
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
pub(super) fn system_connection(
    operation: &'static str,
) -> Result<zbus::blocking::Connection, Error> {
    rmac_dbus::system_blocking().map_err(|error| Error::new(operation, error.to_string()))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn manager_proxy(
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
pub(super) fn wifi_device_path(
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
pub(super) fn property<T>(
    properties: &HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<T>
where
    for<'a> T: TryFrom<&'a zbus::zvariant::OwnedValue>,
{
    properties
        .get(key)
        .and_then(|value| T::try_from(value).ok())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn property_string(
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
pub(super) fn property_bytes(
    properties: &HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<Vec<u8>> {
    properties
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| Vec::<u8>::try_from(value).ok())
}
