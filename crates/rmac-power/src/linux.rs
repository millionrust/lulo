//! Linux UPower and power-profiles-daemon backend.

use super::*;

#[cfg(any(not(target_os = "macos"), test))]
pub(super) const UPOWER_SERVICE: &str = "org.freedesktop.UPower";
#[cfg(not(target_os = "macos"))]
pub(super) const WATCH_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
pub(super) const HISTORY_TIMESPAN_SECONDS: u32 = 24 * 60 * 60;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) const HISTORY_POINT_LIMIT: usize = 96;
#[cfg(not(target_os = "macos"))]
pub(super) const THRESHOLD_VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(not(target_os = "macos"))]
pub(super) const THRESHOLD_VERIFY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);

#[cfg(not(target_os = "macos"))]
pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
    let connection = zbus::blocking::Connection::system()
        .map_err(|error| Error::new("connect to the power service", error.to_string()))?;
    system_snapshot_with_connection(&connection)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_snapshot_with_connection(
    connection: &zbus::blocking::Connection,
) -> Result<Snapshot, Error> {
    Ok(Snapshot {
        battery: linux_battery(connection)?,
        profiles: linux_profiles(connection),
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_battery(
    connection: &zbus::blocking::Connection,
) -> Result<Option<Battery>, Error> {
    use zbus::zvariant::OwnedObjectPath;

    let upower = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .map_err(|error| Error::new("open UPower", error.to_string()))?;
    let on_battery = upower
        .get_property::<bool>("OnBattery")
        .map_err(|error| Error::new("read the active power source", error.to_string()))?;
    let path = upower
        .call::<_, _, OwnedObjectPath>("GetDisplayDevice", &())
        .map_err(|error| Error::new("find the display battery", error.to_string()))?;
    let device = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.UPower",
        path.as_str(),
        "org.freedesktop.UPower.Device",
    )
    .map_err(|error| Error::new("open the display battery", error.to_string()))?;
    let present = device
        .get_property::<bool>("IsPresent")
        .map_err(|error| Error::new("read battery presence", error.to_string()))?;
    if !present {
        return Ok(None);
    }
    let percentage = device
        .get_property::<f64>("Percentage")
        .map_err(|error| Error::new("read battery charge", error.to_string()))?;
    let state = device
        .get_property::<u32>("State")
        .map(battery_state_from_upower)
        .unwrap_or_default();
    let seconds_remaining = match state {
        BatteryState::Charging | BatteryState::PendingCharge => {
            optional_property::<i64>(&device, "TimeToFull")
        }
        _ => optional_property::<i64>(&device, "TimeToEmpty"),
    }
    .and_then(|seconds| (seconds > 0).then_some(seconds as u64));
    let mut capacity = optional_property::<f64>(&device, "Capacity")
        .filter(|value| *value > 0.0)
        .map(percent);
    let mut charge_cycles = optional_property::<i32>(&device, "ChargeCycles")
        .and_then(|cycles| (cycles >= 0).then_some(cycles as u32));
    let energy_rate_watts =
        optional_property::<f64>(&device, "EnergyRate").filter(|rate| *rate > 0.0);
    let mut model = optional_property::<String>(&device, "Model").filter(|model| !model.is_empty());
    let physical = physical_batteries(connection, &upower).unwrap_or_default();
    if let Some(details) = physical.first() {
        capacity = details.capacity.or(capacity);
        charge_cycles = details.charge_cycles.or(charge_cycles);
        model = details.model.clone().or(model);
    }
    let service_owner = upower_service_owner(connection).ok();
    let charge_threshold = charge_threshold_from_batteries(&physical, service_owner.as_deref());
    let history = battery_history(connection, &device, &physical);
    Ok(Some(Battery {
        percentage: percent(percentage),
        state,
        on_battery,
        seconds_remaining,
        capacity,
        charge_cycles,
        energy_rate_watts,
        model,
        charge_threshold,
        history,
    }))
}

#[cfg(not(target_os = "macos"))]
pub(super) struct PhysicalBattery {
    object_path: String,
    native_path: String,
    serial: String,
    capacity: Option<u8>,
    charge_cycles: Option<u32>,
    model: Option<String>,
    threshold_supported: bool,
    threshold_enabled: bool,
    threshold_start: Option<u8>,
    threshold_end: Option<u8>,
    threshold_firmware_managed: bool,
}

#[cfg(not(target_os = "macos"))]
pub(super) fn physical_batteries(
    connection: &zbus::blocking::Connection,
    upower: &zbus::blocking::Proxy<'_>,
) -> Option<Vec<PhysicalBattery>> {
    let paths = upower
        .call::<_, _, Vec<zbus::zvariant::OwnedObjectPath>>("EnumerateDevices", &())
        .ok()?;
    let mut batteries = Vec::new();
    for path in paths {
        let Ok(device) = zbus::blocking::Proxy::new(
            connection,
            "org.freedesktop.UPower",
            path.as_str(),
            "org.freedesktop.UPower.Device",
        ) else {
            continue;
        };
        if optional_property::<u32>(&device, "Type") != Some(2)
            || optional_property::<bool>(&device, "PowerSupply") != Some(true)
            || optional_property::<bool>(&device, "IsPresent") != Some(true)
        {
            continue;
        }
        let threshold_settings =
            optional_property::<u32>(&device, "ChargeThresholdSettingsSupported")
                .unwrap_or_default();
        batteries.push(PhysicalBattery {
            object_path: path.to_string(),
            native_path: optional_property::<String>(&device, "NativePath").unwrap_or_default(),
            serial: optional_property::<String>(&device, "Serial").unwrap_or_default(),
            capacity: optional_property::<f64>(&device, "Capacity")
                .filter(|value| *value > 0.0)
                .map(percent),
            charge_cycles: optional_property::<i32>(&device, "ChargeCycles")
                .and_then(|cycles| (cycles >= 0).then_some(cycles as u32)),
            model: optional_property::<String>(&device, "Model").filter(|model| !model.is_empty()),
            threshold_supported: optional_property::<bool>(&device, "ChargeThresholdSupported")
                .unwrap_or(false),
            threshold_enabled: optional_property::<bool>(&device, "ChargeThresholdEnabled")
                .unwrap_or(false),
            threshold_start: threshold_percent(optional_property::<u32>(
                &device,
                "ChargeStartThreshold",
            )),
            threshold_end: threshold_percent(optional_property::<u32>(
                &device,
                "ChargeEndThreshold",
            )),
            threshold_firmware_managed: threshold_settings & 4 != 0,
        });
    }
    Some(batteries)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn threshold_percent(value: Option<u32>) -> Option<u8> {
    value.filter(|value| *value <= 100).map(|value| value as u8)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn charge_threshold_from_batteries(
    batteries: &[PhysicalBattery],
    service_owner: Option<&str>,
) -> ChargeThreshold {
    if batteries.len() > 1 {
        return ChargeThreshold {
            availability: ChargeThresholdAvailability::MultipleBatteries,
            ..ChargeThreshold::default()
        };
    }
    let Some(battery) = batteries.first() else {
        return ChargeThreshold::default();
    };
    let Some(service_owner) = service_owner else {
        return ChargeThreshold::default();
    };
    if !battery.threshold_supported {
        return ChargeThreshold::default();
    }
    ChargeThreshold {
        availability: ChargeThresholdAvailability::Available,
        enabled: battery.threshold_enabled,
        start_percent: battery.threshold_start,
        end_percent: battery.threshold_end,
        firmware_managed: battery.threshold_firmware_managed,
        identity: Some(ChargeThresholdIdentity {
            service_owner: service_owner.to_owned(),
            object_path: battery.object_path.clone(),
            native_path: battery.native_path.clone(),
            serial: battery.serial.clone(),
        }),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn battery_history(
    connection: &zbus::blocking::Connection,
    display_device: &zbus::blocking::Proxy<'_>,
    physical: &[PhysicalBattery],
) -> BatteryHistory {
    if let Some(history) = history_from_device(display_device) {
        return history;
    }
    if physical.len() != 1 {
        return BatteryHistory::default();
    }
    let Ok(device) = zbus::blocking::Proxy::new(
        connection,
        UPOWER_SERVICE,
        physical[0].object_path.as_str(),
        "org.freedesktop.UPower.Device",
    ) else {
        return BatteryHistory::default();
    };
    history_from_device(&device).unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
pub(super) fn history_from_device(device: &zbus::blocking::Proxy<'_>) -> Option<BatteryHistory> {
    if optional_property::<bool>(device, "HasHistory") != Some(true) {
        return None;
    }
    let raw = match device.call::<_, _, Vec<(u32, f64, u32)>>(
        "GetHistory",
        &(
            "charge",
            HISTORY_TIMESPAN_SECONDS,
            HISTORY_POINT_LIMIT as u32,
        ),
    ) {
        Ok(raw) => raw,
        Err(_) => {
            return Some(BatteryHistory {
                availability: BatteryHistoryAvailability::TemporarilyUnavailable,
                points: Vec::new(),
            });
        }
    };
    Some(BatteryHistory {
        availability: BatteryHistoryAvailability::Available,
        points: normalize_history(raw),
    })
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn normalize_history(raw: Vec<(u32, f64, u32)>) -> Vec<BatteryHistoryPoint> {
    let mut points = raw
        .into_iter()
        .filter(|(timestamp, value, _)| {
            *timestamp > 0 && value.is_finite() && (0.0..=100.0).contains(value)
        })
        .map(|(timestamp, value, state)| BatteryHistoryPoint {
            timestamp: u64::from(timestamp),
            percentage: percent(value),
            state: battery_state_from_upower(state),
        })
        .collect::<Vec<_>>();
    points.sort_by_key(|point| point.timestamp);
    points.dedup_by_key(|point| point.timestamp);
    if points.len() > HISTORY_POINT_LIMIT {
        points.drain(..points.len() - HISTORY_POINT_LIMIT);
    }
    points
}

#[cfg(not(target_os = "macos"))]
pub(super) fn upower_service_owner(
    connection: &zbus::blocking::Connection,
) -> Result<String, Error> {
    let dbus = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .map_err(|error| Error::new("identify UPower", error.to_string()))?;
    dbus.call::<_, _, String>("GetNameOwner", &(UPOWER_SERVICE,))
        .map_err(|error| Error::new("identify UPower", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn optional_property<T>(proxy: &zbus::blocking::Proxy<'_>, name: &str) -> Option<T>
where
    T: TryFrom<zbus::zvariant::OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    proxy.get_property(name).ok()
}

#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy)]
pub(super) struct ProfileEndpoint {
    destination: &'static str,
    path: &'static str,
    interface: &'static str,
}

#[cfg(not(target_os = "macos"))]
pub(super) const PROFILE_ENDPOINTS: [ProfileEndpoint; 2] = [
    ProfileEndpoint {
        destination: "org.freedesktop.UPower.PowerProfiles",
        path: "/org/freedesktop/UPower/PowerProfiles",
        interface: "org.freedesktop.UPower.PowerProfiles",
    },
    ProfileEndpoint {
        destination: "net.hadess.PowerProfiles",
        path: "/net/hadess/PowerProfiles",
        interface: "net.hadess.PowerProfiles",
    },
];

#[cfg(not(target_os = "macos"))]
pub(super) fn linux_profiles(connection: &zbus::blocking::Connection) -> Profiles {
    PROFILE_ENDPOINTS
        .iter()
        .find_map(|endpoint| profiles_at_endpoint(connection, *endpoint).ok())
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
pub(super) fn profiles_at_endpoint(
    connection: &zbus::blocking::Connection,
    endpoint: ProfileEndpoint,
) -> Result<Profiles, zbus::Error> {
    use std::collections::HashMap;
    use zbus::zvariant::OwnedValue;

    let proxy = zbus::blocking::Proxy::new(
        connection,
        endpoint.destination,
        endpoint.path,
        endpoint.interface,
    )?;
    let active = proxy
        .get_property::<String>("ActiveProfile")
        .ok()
        .and_then(|profile| parse_profile(&profile));
    let raw = proxy.get_property::<Vec<HashMap<String, OwnedValue>>>("Profiles")?;
    let mut supported = raw
        .iter()
        .filter_map(|profile| property_string(profile, "Profile"))
        .filter_map(|profile| parse_profile(&profile))
        .collect::<Vec<_>>();
    normalize_profiles(&mut supported);
    let performance_degraded = proxy
        .get_property::<String>("PerformanceDegraded")
        .ok()
        .filter(|reason| !reason.is_empty());
    Ok(Profiles {
        available: true,
        active,
        supported,
        performance_degraded,
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn property_string(
    properties: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<String> {
    properties
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_set_profile(profile: PowerProfile) -> Result<(), Error> {
    let connection = zbus::blocking::Connection::system()
        .map_err(|error| Error::new("connect to the power profile service", error.to_string()))?;
    let mut failures = Vec::new();
    for endpoint in PROFILE_ENDPOINTS {
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            endpoint.destination,
            endpoint.path,
            endpoint.interface,
        );
        match proxy {
            Ok(proxy) => match proxy.set_property("ActiveProfile", profile.id()) {
                Ok(()) => return Ok(()),
                Err(error) => failures.push(error.to_string()),
            },
            Err(error) => failures.push(error.to_string()),
        }
    }
    Err(Error::new("change the power profile", failures.join("; ")))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_set_charge_threshold(
    threshold: &ChargeThreshold,
    enabled: bool,
) -> Result<Snapshot, Error> {
    let identity = threshold.identity.as_ref().ok_or_else(|| {
        Error::new(
            "change optimized charging",
            "UPower did not advertise a writable charge threshold for this battery",
        )
    })?;
    if !threshold.can_change() {
        return Err(Error::new(
            "change optimized charging",
            "the captured battery capability is no longer writable",
        ));
    }

    let connection = zbus::blocking::Connection::system()
        .map_err(|error| Error::new("connect to UPower", error.to_string()))?;
    let current = revalidate_threshold_battery(&connection, identity)?;
    if current.threshold_enabled != enabled {
        let device = zbus::blocking::Proxy::new(
            &connection,
            identity.service_owner.as_str(),
            identity.object_path.as_str(),
            "org.freedesktop.UPower.Device",
        )
        .map_err(|error| Error::new("open the charge-threshold device", error.to_string()))?;
        device
            .call::<_, _, ()>("EnableChargeThreshold", &(enabled,))
            .map_err(|error| Error::new("change optimized charging", error.to_string()))?;
    }

    let deadline = std::time::Instant::now() + THRESHOLD_VERIFY_TIMEOUT;
    loop {
        let current = revalidate_threshold_battery(&connection, identity)?;
        if current.threshold_enabled == enabled {
            let snapshot = system_snapshot_with_connection(&connection)?;
            let verified = snapshot.battery.as_ref().is_some_and(|battery| {
                battery.charge_threshold.can_change()
                    && battery.charge_threshold.enabled == enabled
                    && battery.charge_threshold.identity.as_ref() == Some(identity)
            });
            if verified {
                return Ok(snapshot);
            }
            return Err(Error::new(
                "verify optimized charging",
                "UPower returned a different battery after the change",
            ));
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                "verify optimized charging",
                "UPower did not confirm the requested state within three seconds",
            ));
        }
        std::thread::sleep(THRESHOLD_VERIFY_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn revalidate_threshold_battery(
    connection: &zbus::blocking::Connection,
    identity: &ChargeThresholdIdentity,
) -> Result<PhysicalBattery, Error> {
    let owner = upower_service_owner(connection)?;
    if owner != identity.service_owner {
        return Err(Error::new(
            "change optimized charging",
            "UPower restarted; refresh Battery before trying again",
        ));
    }
    let upower = zbus::blocking::Proxy::new(
        connection,
        UPOWER_SERVICE,
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .map_err(|error| Error::new("open UPower", error.to_string()))?;
    let mut batteries = physical_batteries(connection, &upower).ok_or_else(|| {
        Error::new(
            "inspect the charge-threshold device",
            "UPower did not return its physical battery inventory",
        )
    })?;
    if upower_service_owner(connection)? != identity.service_owner {
        return Err(Error::new(
            "change optimized charging",
            "UPower restarted during validation; refresh Battery before trying again",
        ));
    }
    if batteries.len() != 1 {
        return Err(Error::new(
            "change optimized charging",
            "the physical battery inventory changed; refresh Battery before trying again",
        ));
    }
    let battery = batteries.remove(0);
    if battery.object_path != identity.object_path
        || battery.native_path != identity.native_path
        || battery.serial != identity.serial
    {
        return Err(Error::new(
            "change optimized charging",
            "the physical battery changed; refresh Battery before trying again",
        ));
    }
    if !battery.threshold_supported {
        return Err(Error::new(
            "change optimized charging",
            "UPower no longer advertises writable charge thresholds for this battery",
        ));
    }
    Ok(battery)
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let mut unavailable_reported = false;
    loop {
        match watch_once(&sender, &mut unavailable_reported).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => publish_unavailable(&sender, &mut unavailable_reported).await?,
        }
        async_io::Timer::after(WATCH_RECONNECT_DELAY).await;
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn watch_once(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system()
        .await
        .map_err(|error| Error::new("connect power event stream", error.to_string()))?;
    let upower_rule = service_signal_rule(UPOWER_SERVICE, "build UPower signal filter")?;
    let modern_profiles_rule = service_signal_rule(
        PROFILE_ENDPOINTS[0].destination,
        "build power-profile signal filter",
    )?;
    let legacy_profiles_rule = service_signal_rule(
        PROFILE_ENDPOINTS[1].destination,
        "build legacy power-profile signal filter",
    )?;
    // Only the power services' own owner changes: the system bus announces
    // every client that connects, and waking for each of them kept this
    // watcher busy on a quiet desktop.
    let owner_rule = |upower_namespace: bool| -> Result<MatchRule<'static>, Error> {
        let builder = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender("org.freedesktop.DBus")
            .and_then(|builder| builder.path("/org/freedesktop/DBus"))
            .and_then(|builder| builder.interface("org.freedesktop.DBus"))
            .and_then(|builder| builder.member("NameOwnerChanged"));
        let builder = if upower_namespace {
            // org.freedesktop.UPower and org.freedesktop.UPower.PowerProfiles.
            builder.and_then(|builder| builder.arg0ns(UPOWER_SERVICE))
        } else {
            builder.and_then(|builder| builder.add_arg(PROFILE_ENDPOINTS[1].destination))
        };
        builder
            .map(|builder| builder.build())
            .map_err(|error| Error::new("build power owner filter", error.to_string()))
    };
    let mut upower = MessageStream::for_match_rule(upower_rule, &connection, Some(64))
        .await
        .map_err(|error| Error::new("subscribe to UPower changes", error.to_string()))?
        .fuse();
    let mut modern_profiles =
        MessageStream::for_match_rule(modern_profiles_rule, &connection, Some(16))
            .await
            .map_err(|error| Error::new("subscribe to power-profile changes", error.to_string()))?
            .fuse();
    let mut legacy_profiles =
        MessageStream::for_match_rule(legacy_profiles_rule, &connection, Some(16))
            .await
            .map_err(|error| {
                Error::new(
                    "subscribe to legacy power-profile changes",
                    error.to_string(),
                )
            })?
            .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule(true)?, &connection, Some(8))
        .await
        .map_err(|error| Error::new("subscribe to power service restarts", error.to_string()))?
        .fuse();
    let mut legacy_owners = MessageStream::for_match_rule(owner_rule(false)?, &connection, Some(8))
        .await
        .map_err(|error| Error::new("subscribe to power service restarts", error.to_string()))?
        .fuse();

    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|error| Error::new("inspect UPower service", error.to_string()))?;
    let service = zbus::names::BusName::try_from(UPOWER_SERVICE)
        .map_err(|error| Error::new("inspect UPower service", error.to_string()))?;
    let mut upower_available = dbus
        .name_has_owner(service)
        .await
        .map_err(|error| Error::new("inspect UPower service", error.to_string()))?;
    if upower_available {
        publish_changed(sender, unavailable_reported).await?;
    } else {
        publish_unavailable(sender, unavailable_reported).await?;
    }

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let event = futures_util::select! {
            message = upower.next() => {
                read_signal(message, "read UPower change")?
                    .then_some(PowerOwnerEvent::Upower(true))
            },
            message = modern_profiles.next() => {
                read_signal(message, "read power-profile change")?
                    .then_some(PowerOwnerEvent::Profiles)
            },
            message = legacy_profiles.next() => {
                read_signal(message, "read legacy power-profile change")?
                    .then_some(PowerOwnerEvent::Profiles)
            },
            message = owners.next() => read_owner_event(message)?,
            message = legacy_owners.next() => read_owner_event(message)?,
            _ = closed => return Ok(()),
        };
        match event {
            Some(PowerOwnerEvent::Upower(false)) => {
                upower_available = false;
                publish_unavailable(sender, unavailable_reported).await?;
            }
            Some(PowerOwnerEvent::Upower(true)) => {
                upower_available = true;
                publish_changed(sender, unavailable_reported).await?;
            }
            Some(PowerOwnerEvent::Profiles) if upower_available => {
                publish_changed(sender, unavailable_reported).await?;
            }
            _ => {}
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn service_signal_rule(
    service: &'static str,
    operation: &'static str,
) -> Result<zbus::MatchRule<'static>, Error> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(service)
        .map_err(|error| Error::new(operation, error.to_string()))?
        .build();
    Ok(rule)
}

/// Reads one message from a service's signal stream and returns whether the
/// service itself sent it.
///
/// zbus cannot match a well-known sender on the client side, so each
/// sender-only stream also receives every signal the connection's other
/// rules let in, including the bus's own `NameOwnerChanged` for every client
/// that connects or disconnects. Treating those as power changes made the
/// desktop re-read UPower on a fresh connection whose `NameOwnerChanged`
/// triggered the next read: a loop that kept the desktop busy at idle.
#[cfg(not(target_os = "macos"))]
pub(super) fn read_signal(
    message: Option<Result<zbus::Message, zbus::Error>>,
    operation: &'static str,
) -> Result<bool, Error> {
    match message {
        Some(Ok(message)) => {
            let header = message.header();
            Ok(sent_by_service(
                header.sender().map(|sender| sender.as_str()),
            ))
        }
        Some(Err(error)) => Err(Error::new(operation, error.to_string())),
        None => Err(Error::new(operation, "the signal stream ended")),
    }
}

/// Whether a signal came from a service rather than the message bus itself.
#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn sent_by_service(sender: Option<&str>) -> bool {
    matches!(sender, Some(sender) if sender != "org.freedesktop.DBus")
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PowerOwnerEvent {
    Upower(bool),
    Profiles,
}

#[cfg(not(target_os = "macos"))]
pub(super) fn read_owner_event(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<PowerOwnerEvent>, Error> {
    let message = message
        .ok_or_else(|| Error::new("read power service owner", "the signal stream ended"))?
        .map_err(|error| Error::new("read power service owner", error.to_string()))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|error| Error::new("read power service owner", error.to_string()))?;
    Ok(power_owner_event(&name, &new_owner))
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn power_owner_event(name: &str, new_owner: &str) -> Option<PowerOwnerEvent> {
    if name == UPOWER_SERVICE {
        Some(PowerOwnerEvent::Upower(!new_owner.is_empty()))
    } else if name == "org.freedesktop.UPower.PowerProfiles" || name == "net.hadess.PowerProfiles" {
        Some(PowerOwnerEvent::Profiles)
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn publish_changed(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if *unavailable_reported {
        sender
            .send(WatchEvent::Changed)
            .await
            .map_err(|_| Error::new("publish power recovery", "the event consumer closed"))?;
    } else {
        let _ = sender.try_send(WatchEvent::Changed);
    }
    *unavailable_reported = false;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn publish_unavailable(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if !*unavailable_reported {
        sender
            .send(WatchEvent::Unavailable)
            .await
            .map_err(|_| Error::new("publish power outage", "the event consumer closed"))?;
        *unavailable_reported = true;
    }
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn battery_state_from_upower(state: u32) -> BatteryState {
    match state {
        1 => BatteryState::Charging,
        2 => BatteryState::Discharging,
        3 => BatteryState::Empty,
        4 => BatteryState::FullyCharged,
        5 => BatteryState::PendingCharge,
        6 => BatteryState::PendingDischarge,
        _ => BatteryState::Unknown,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_profile(profile: &str) -> Option<PowerProfile> {
    match profile {
        "power-saver" => Some(PowerProfile::PowerSaver),
        "balanced" => Some(PowerProfile::Balanced),
        "performance" => Some(PowerProfile::Performance),
        _ => None,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn normalize_profiles(profiles: &mut Vec<PowerProfile>) {
    profiles.sort_by_key(|profile| match profile {
        PowerProfile::PowerSaver => 0,
        PowerProfile::Balanced => 1,
        PowerProfile::Performance => 2,
    });
    profiles.dedup();
}
