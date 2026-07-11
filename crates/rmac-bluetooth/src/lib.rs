//! Cross-platform Bluetooth adapter and known-device service.

use std::fmt;
#[cfg(target_os = "macos")]
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub address: String,
    pub kind: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub available: bool,
    pub powered: bool,
    pub discoverable: bool,
    pub discovering: bool,
    pub adapter_name: Option<String>,
    pub devices: Vec<Device>,
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

pub trait BluetoothService {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_powered(&self, powered: bool) -> Result<(), Error>;
    fn set_discoverable(&self, discoverable: bool) -> Result<(), Error>;
    fn start_discovery(&self) -> Result<(), Error>;
    fn stop_discovery(&self) -> Result<(), Error>;
    fn set_connected(&self, device_id: &str, connected: bool) -> Result<(), Error>;
}

pub struct SystemBluetoothService;

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemBluetoothService.snapshot()
}

pub fn set_powered(powered: bool) -> Result<(), Error> {
    SystemBluetoothService.set_powered(powered)
}

pub fn set_discoverable(discoverable: bool) -> Result<(), Error> {
    SystemBluetoothService.set_discoverable(discoverable)
}

pub fn start_discovery() -> Result<(), Error> {
    SystemBluetoothService.start_discovery()
}

pub fn stop_discovery() -> Result<(), Error> {
    SystemBluetoothService.stop_discovery()
}

pub fn set_connected(device_id: &str, connected: bool) -> Result<(), Error> {
    SystemBluetoothService.set_connected(device_id, connected)
}

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
        let proxy =
            zbus::blocking::Proxy::new(&connection, "org.bluez", device_id, "org.bluez.Device1")
                .map_err(|error| Error::new("open Bluetooth device", error.to_string()))?;
        let method = if connected { "Connect" } else { "Disconnect" };
        proxy
            .call::<_, _, ()>(method, &())
            .map_err(|error| Error::new("change Bluetooth connection", error.to_string()))
    }
}

#[cfg(not(target_os = "macos"))]
fn linux_snapshot() -> Result<Snapshot, Error> {
    let connection = system_connection()?;
    let objects = managed_objects(&connection)?;
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
type ManagedObjects = zbus::fdo::ManagedObjects;

#[cfg(not(target_os = "macos"))]
fn system_connection() -> Result<zbus::blocking::Connection, Error> {
    zbus::blocking::Connection::system()
        .map_err(|error| Error::new("connect to BlueZ", error.to_string()))
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

#[cfg(target_os = "macos")]
impl BluetoothService for SystemBluetoothService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        let output = Command::new("system_profiler")
            .arg("SPBluetoothDataType")
            .output()
            .map_err(|error| Error::new("start Bluetooth profiler", error.to_string()))?;
        if !output.status.success() {
            return Err(Error::new(
                "read Bluetooth state",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        Ok(parse_macos_snapshot(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    fn set_powered(&self, _powered: bool) -> Result<(), Error> {
        Err(Error::new(
            "change Bluetooth power",
            "the macOS development adapter is read-only",
        ))
    }

    fn set_discoverable(&self, _discoverable: bool) -> Result<(), Error> {
        Err(Error::new(
            "change Bluetooth discoverability",
            "the macOS development adapter is read-only",
        ))
    }

    fn start_discovery(&self) -> Result<(), Error> {
        Ok(())
    }

    fn stop_discovery(&self) -> Result<(), Error> {
        Ok(())
    }

    fn set_connected(&self, _device_id: &str, _connected: bool) -> Result<(), Error> {
        Err(Error::new(
            "change Bluetooth connection",
            "the macOS development adapter is read-only",
        ))
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_snapshot(output: &str) -> Snapshot {
    let powered = output
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("State: On"));
    let mut devices = Vec::new();
    let mut connected_group = false;
    let mut current: Option<Device> = None;
    let flush = |current: &mut Option<Device>, devices: &mut Vec<Device>| {
        if let Some(device) = current.take() {
            devices.push(device);
        }
    };
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed == "Connected:" {
            flush(&mut current, &mut devices);
            connected_group = true;
        } else if trimmed == "Not Connected:" {
            flush(&mut current, &mut devices);
            connected_group = false;
        } else if trimmed.ends_with(':') && !trimmed.contains(": ") {
            flush(&mut current, &mut devices);
            let name = trimmed.trim_end_matches(':').trim();
            if !matches!(
                name,
                "" | "Bluetooth" | "Bluetooth Controller" | "Controller"
            ) {
                current = Some(Device {
                    id: name.to_string(),
                    name: name.to_string(),
                    address: String::new(),
                    kind: String::new(),
                    paired: true,
                    connected: connected_group,
                });
            }
        } else if let Some(device) = current.as_mut() {
            if let Some(value) = trimmed.strip_prefix("Address:") {
                device.address = value.trim().to_string();
            } else if let Some(value) = trimmed.strip_prefix("Minor Type:") {
                device.kind = value.trim().to_string();
            }
        }
    }
    flush(&mut current, &mut devices);
    sort_devices(&mut devices);
    Snapshot {
        available: output.contains("Bluetooth Controller:"),
        powered,
        adapter_name: Some("Bluetooth".into()),
        devices,
        ..Snapshot::default()
    }
}

fn sort_devices(devices: &mut [Device]) {
    devices.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.paired.cmp(&left.paired))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_fixture_parses_and_sorts_known_devices() {
        let snapshot = parse_macos_snapshot(
            "Bluetooth Controller:\n  State: On\n  Connected:\n    Keyboard:\n      Address: AA-BB\n      Minor Type: Keyboard\n  Not Connected:\n    Headphones:\n      Address: CC-DD\n      Minor Type: Headphones\n",
        );

        assert!(snapshot.available);
        assert!(snapshot.powered);
        assert_eq!(snapshot.devices.len(), 2);
        assert_eq!(snapshot.devices[0].name, "Keyboard");
        assert!(snapshot.devices[0].connected);
        assert_eq!(snapshot.devices[1].kind, "Headphones");
    }

    #[test]
    fn errors_preserve_operation_context() {
        let error = Error::new("read Bluetooth state", "service unavailable");
        assert_eq!(
            error.to_string(),
            "could not read Bluetooth state: service unavailable"
        );
    }

    #[test]
    fn device_order_prefers_connected_then_paired() {
        let mut devices = vec![
            Device {
                id: "nearby".into(),
                name: "Nearby".into(),
                address: String::new(),
                kind: String::new(),
                paired: false,
                connected: false,
            },
            Device {
                id: "paired".into(),
                name: "Paired".into(),
                address: String::new(),
                kind: String::new(),
                paired: true,
                connected: false,
            },
            Device {
                id: "connected".into(),
                name: "Connected".into(),
                address: String::new(),
                kind: String::new(),
                paired: true,
                connected: true,
            },
        ];

        sort_devices(&mut devices);

        assert_eq!(devices[0].id, "connected");
        assert_eq!(devices[1].id, "paired");
        assert_eq!(devices[2].id, "nearby");
    }
}
