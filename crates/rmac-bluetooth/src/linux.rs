use std::time::{Duration, Instant};

use crate::macos::sort_devices;
use crate::pairing_agent;
use crate::{BluetoothService, Device, Error, PairingSession, Snapshot, SystemBluetoothService};

const REMOVE_VERIFY_TIMEOUT: Duration = Duration::from_secs(4);
const REMOVE_VERIFY_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(not(target_os = "macos"))]
impl BluetoothService for SystemBluetoothService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        linux_snapshot()
    }

    fn set_powered(&self, powered: bool) -> Result<(), Error> {
        set_adapter_property("Powered", powered, "change Bluetooth power")
    }

    fn set_discoverable(&self, discoverable: bool) -> Result<(), Error> {
        set_adapter_property(
            "Discoverable",
            discoverable,
            "change Bluetooth discoverability",
        )
    }

    fn start_discovery(&self) -> Result<(), Error> {
        call_adapter("StartDiscovery", "start Bluetooth discovery")
    }

    fn stop_discovery(&self) -> Result<(), Error> {
        call_adapter("StopDiscovery", "stop Bluetooth discovery")
    }

    fn set_connected(&self, device_id: &str, connected: bool) -> Result<(), Error> {
        let connection = system_connection()?;
        let (device, _, _) = device_context(&connection, device_id)?;
        let proxy =
            zbus::blocking::Proxy::new(&connection, "org.bluez", device, "org.bluez.Device1")
                .map_err(|error| Error::new("open Bluetooth device", error.to_string()))?;
        let method = if connected { "Connect" } else { "Disconnect" };
        proxy
            .call::<_, _, ()>(method, &())
            .map_err(|error| Error::new("change Bluetooth connection", error.to_string()))
    }

    fn pair(&self, device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
        linux_pair(device_id, session)
    }

    fn cancel_pairing(&self, device_id: &str) -> Result<(), Error> {
        let connection = system_connection()?;
        let (device, _, _) = device_context(&connection, device_id)?;
        zbus::blocking::Proxy::new(&connection, "org.bluez", device, "org.bluez.Device1")
            .map_err(|error| Error::new("open Bluetooth device", error.to_string()))?
            .call::<_, _, ()>("CancelPairing", &())
            .map_err(|error| Error::new("cancel Bluetooth pairing", error.to_string()))
    }

    fn remove_device(&self, device_id: &str) -> Result<Snapshot, Error> {
        linux_remove_device(device_id)
    }
}

#[cfg(not(target_os = "macos"))]
fn linux_snapshot() -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    linux_snapshot_with_connection(&connection)
}

#[cfg(not(target_os = "macos"))]
fn linux_snapshot_with_connection(
    connection: &zbus::blocking::Connection,
) -> Result<Snapshot, Error> {
    let objects = managed_objects(connection)?;
    let Some((adapter_path, adapter)) = find_interface(&objects, "org.bluez.Adapter1") else {
        return Ok(Snapshot::default());
    };
    let powered = property::<bool>(adapter, "Powered").unwrap_or(false);
    let discoverable = property::<bool>(adapter, "Discoverable").unwrap_or(false);
    let discovering = property::<bool>(adapter, "Discovering").unwrap_or(false);
    let adapter_name = property_string(adapter, "Alias")
        .or_else(|| property_string(adapter, "Name"))
        .or_else(|| Some(adapter_path.to_string()));
    let mut devices = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let properties = interfaces.iter().find_map(|(interface, properties)| {
                (interface.as_str() == "org.bluez.Device1").then_some(properties)
            })?;
            let paired = property::<bool>(properties, "Paired").unwrap_or(false);
            let trusted = property::<bool>(properties, "Trusted").unwrap_or(false);
            let connected = property::<bool>(properties, "Connected").unwrap_or(false);
            let name = property_string(properties, "Alias")
                .or_else(|| property_string(properties, "Name"))
                .or_else(|| property_string(properties, "Address"))?;
            Some(Device {
                id: path.to_string(),
                name,
                address: property_string(properties, "Address").unwrap_or_default(),
                kind: property_string(properties, "Icon").unwrap_or_default(),
                paired,
                trusted,
                connected,
            })
        })
        .collect::<Vec<_>>();
    sort_devices(&mut devices);
    Ok(Snapshot {
        available: true,
        powered,
        discoverable,
        discovering,
        adapter_name,
        devices,
    })
}

#[cfg(not(target_os = "macos"))]
fn linux_pair(device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
    let operation = "pair Bluetooth device";
    let result = (|| {
        let device = zbus::zvariant::OwnedObjectPath::try_from(device_id)
            .map_err(|error| Error::new(operation, format!("invalid device identity: {error}")))?;
        let agent =
            pairing_agent::RegisteredPairingAgent::register(device.clone(), session.clone())
                .map_err(|error| {
                    Error::new("register Bluetooth pairing agent", error.to_string())
                })?;
        let connection = agent.connection();
        let (device, _, paired) = device_context(connection, device_id)?;
        if paired {
            return Err(Error::new(
                operation,
                "the selected device is already paired",
            ));
        }
        if session.outcome().canceled {
            return Err(Error::pairing(operation, "pairing was canceled", session));
        }

        let proxy = zbus::blocking::Proxy::new(
            connection,
            "org.bluez",
            device.clone(),
            "org.bluez.Device1",
        )
        .map_err(|error| Error::new("open Bluetooth device", error.to_string()))?;
        if let Err(error) = proxy.call::<_, _, ()>("Pair", &()) {
            return Err(Error::pairing(operation, error.to_string(), session));
        }
        if let Err(error) = proxy.set_property("Trusted", true) {
            let snapshot = linux_snapshot_with_connection(connection).ok();
            let state = snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .devices
                        .iter()
                        .find(|candidate| candidate.id == device_id)
                })
                .map(|device| format!("paired={}, trusted={}", device.paired, device.trusted))
                .unwrap_or_else(|| "device state unavailable".into());
            return Err(Error::new(
                "trust paired Bluetooth device",
                format!("{error}; authoritative state: {state}"),
            ));
        }

        let snapshot = linux_snapshot_with_connection(connection)?;
        let Some(device) = snapshot
            .devices
            .iter()
            .find(|candidate| candidate.id == device_id)
        else {
            return Err(Error::new(
                operation,
                "the paired device disappeared before state could be verified",
            ));
        };
        if !device.paired || !device.trusted {
            return Err(Error::new(
                operation,
                "BlueZ did not confirm both paired and trusted state",
            ));
        }
        Ok(snapshot)
    })();
    session.finish();
    result
}

#[cfg(not(target_os = "macos"))]
fn linux_remove_device(device_id: &str) -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    let (device, adapter, _) = device_context(&connection, device_id)?;
    zbus::blocking::Proxy::new(&connection, "org.bluez", adapter, "org.bluez.Adapter1")
        .map_err(|error| Error::new("open Bluetooth adapter", error.to_string()))?
        .call::<_, _, ()>("RemoveDevice", &(device,))
        .map_err(|error| Error::new("forget Bluetooth device", error.to_string()))?;
    let deadline = Instant::now() + REMOVE_VERIFY_TIMEOUT;
    loop {
        let snapshot = linux_snapshot_with_connection(&connection)?;
        if snapshot
            .devices
            .iter()
            .all(|candidate| candidate.id != device_id)
        {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(Error::new(
                "forget Bluetooth device",
                "BlueZ still reports the device after removal timed out",
            ));
        }
        std::thread::sleep(REMOVE_VERIFY_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
type ManagedObjects = zbus::fdo::ManagedObjects;

#[cfg(not(target_os = "macos"))]
fn system_connection() -> Result<zbus::blocking::Connection, Error> {
    rmac_dbus::system_blocking().map_err(|error| Error::new("connect to BlueZ", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn managed_objects(connection: &zbus::blocking::Connection) -> Result<ManagedObjects, Error> {
    zbus::blocking::Proxy::new(
        connection,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .map_err(|error| Error::new("open BlueZ object manager", error.to_string()))?
    .call("GetManagedObjects", &())
    .map_err(|error| Error::new("read Bluetooth objects", error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn find_interface<'a>(
    objects: &'a ManagedObjects,
    target: &str,
) -> Option<(
    &'a zbus::zvariant::OwnedObjectPath,
    &'a std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
)> {
    objects.iter().find_map(|(path, interfaces)| {
        interfaces.iter().find_map(|(interface, properties)| {
            (interface.as_str() == target).then_some((path, properties))
        })
    })
}

#[cfg(not(target_os = "macos"))]
fn adapter_path(connection: &zbus::blocking::Connection) -> Result<String, Error> {
    let objects = managed_objects(connection)?;
    find_interface(&objects, "org.bluez.Adapter1")
        .map(|(path, _)| path.to_string())
        .ok_or_else(|| Error::new("find Bluetooth adapter", "no Bluetooth adapter found"))
}

#[cfg(not(target_os = "macos"))]
fn device_context(
    connection: &zbus::blocking::Connection,
    device_id: &str,
) -> Result<
    (
        zbus::zvariant::OwnedObjectPath,
        zbus::zvariant::OwnedObjectPath,
        bool,
    ),
    Error,
> {
    use zbus::zvariant::OwnedObjectPath;

    let device = OwnedObjectPath::try_from(device_id).map_err(|error| {
        Error::new(
            "find Bluetooth device",
            format!("invalid identity: {error}"),
        )
    })?;
    let objects = managed_objects(connection)?;
    let interfaces = objects.get(&device).ok_or_else(|| {
        Error::new(
            "find Bluetooth device",
            "the selected device is no longer available",
        )
    })?;
    let properties = interfaces
        .iter()
        .find_map(|(interface, properties)| {
            (interface.as_str() == "org.bluez.Device1").then_some(properties)
        })
        .ok_or_else(|| {
            Error::new(
                "find Bluetooth device",
                "the selected object is not a Bluetooth device",
            )
        })?;
    let adapter = properties
        .get("Adapter")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| OwnedObjectPath::try_from(value).ok())
        .ok_or_else(|| {
            Error::new(
                "find Bluetooth device",
                "BlueZ did not identify the device adapter",
            )
        })?;
    let adapter_valid = objects.get(&adapter).is_some_and(|interfaces| {
        interfaces
            .keys()
            .any(|interface| interface.as_str() == "org.bluez.Adapter1")
    });
    if !adapter_valid {
        return Err(Error::new(
            "find Bluetooth device",
            "the device adapter is no longer available",
        ));
    }
    Ok((
        device,
        adapter,
        property::<bool>(properties, "Paired").unwrap_or(false),
    ))
}

#[cfg(not(target_os = "macos"))]
fn set_adapter_property(property: &str, value: bool, operation: &'static str) -> Result<(), Error> {
    let connection = system_connection()?;
    let path = adapter_path(&connection)?;
    zbus::blocking::Proxy::new(&connection, "org.bluez", path, "org.bluez.Adapter1")
        .map_err(|error| Error::new("open Bluetooth adapter", error.to_string()))?
        .set_property(property, value)
        .map_err(|error| Error::new(operation, error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn call_adapter(method: &str, operation: &'static str) -> Result<(), Error> {
    let connection = system_connection()?;
    let path = adapter_path(&connection)?;
    zbus::blocking::Proxy::new(&connection, "org.bluez", path, "org.bluez.Adapter1")
        .map_err(|error| Error::new("open Bluetooth adapter", error.to_string()))?
        .call::<_, _, ()>(method, &())
        .map_err(|error| Error::new(operation, error.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn property<T>(
    properties: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<T>
where
    for<'a> T: TryFrom<&'a zbus::zvariant::OwnedValue>,
{
    properties
        .get(key)
        .and_then(|value| T::try_from(value).ok())
}

#[cfg(not(target_os = "macos"))]
fn property_string(
    properties: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    key: &str,
) -> Option<String> {
    properties
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}
