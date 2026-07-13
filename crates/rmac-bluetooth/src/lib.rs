//! Cross-platform Bluetooth adapter and known-device service.

use std::fmt;
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(not(target_os = "macos"))]
use std::time::{Duration, Instant};

mod pairing_agent;

pub use pairing_agent::{
    PairingEvent, PairingInputError, PairingPasskey, PairingPinCode, PairingPrompt,
    PairingPromptId, PairingPromptKind, PairingSession,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub address: String,
    pub kind: String,
    pub paired: bool,
    pub trusted: bool,
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
    kind: ErrorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorKind {
    General,
    Canceled,
    Rejected,
    TimedOut,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
            kind: ErrorKind::General,
        }
    }

    #[cfg(any(not(target_os = "macos"), test))]
    fn pairing(
        operation: &'static str,
        detail: impl Into<String>,
        session: &PairingSession,
    ) -> Self {
        let detail = detail.into();
        let outcome = session.outcome();
        let kind = if outcome.timed_out || detail.contains("AuthenticationTimeout") {
            ErrorKind::TimedOut
        } else if outcome.rejected || detail.contains("AuthenticationRejected") {
            ErrorKind::Rejected
        } else if outcome.canceled || detail.contains("AuthenticationCanceled") {
            ErrorKind::Canceled
        } else {
            ErrorKind::General
        };
        let detail = match kind {
            ErrorKind::TimedOut => "pairing confirmation timed out".to_string(),
            ErrorKind::Rejected => "pairing confirmation was rejected".to_string(),
            ErrorKind::Canceled => "pairing was canceled".to_string(),
            ErrorKind::General => detail,
        };
        Self {
            operation,
            detail,
            kind,
        }
    }

    pub fn is_canceled(&self) -> bool {
        self.kind == ErrorKind::Canceled
    }

    pub fn is_rejected(&self) -> bool {
        self.kind == ErrorKind::Rejected
    }

    pub fn is_timed_out(&self) -> bool {
        self.kind == ErrorKind::TimedOut
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
    fn pair(&self, device_id: &str, session: &PairingSession) -> Result<Snapshot, Error>;
    fn cancel_pairing(&self, device_id: &str) -> Result<(), Error>;
    fn remove_device(&self, device_id: &str) -> Result<Snapshot, Error>;
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

pub fn pair(device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
    SystemBluetoothService.pair(device_id, session)
}

pub fn cancel_pairing(device_id: &str) -> Result<(), Error> {
    SystemBluetoothService.cancel_pairing(device_id)
}

pub fn remove_device(device_id: &str) -> Result<Snapshot, Error> {
    SystemBluetoothService.remove_device(device_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    system_watch(sender).await
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

#[cfg(not(target_os = "macos"))]
const BLUEZ_SERVICE: &str = "org.bluez";
#[cfg(not(target_os = "macos"))]
const WATCH_RECONNECT_DELAY: Duration = Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
const WATCH_QUIET_PERIOD: Duration = Duration::from_millis(75);
#[cfg(not(target_os = "macos"))]
const REMOVE_VERIFY_TIMEOUT: Duration = Duration::from_secs(4);
#[cfg(not(target_os = "macos"))]
const REMOVE_VERIFY_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(not(target_os = "macos"))]
async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
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

#[cfg(target_os = "macos")]
async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    sender
        .send(WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch Bluetooth changes", "the event consumer closed"))
}

#[cfg(not(target_os = "macos"))]
async fn watch_once(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    use futures_util::{FutureExt as _, StreamExt as _};
    use zbus::{message::Type, MatchRule, MessageStream};

    let connection = zbus::Connection::system()
        .await
        .map_err(|error| Error::new("connect Bluetooth event stream", error.to_string()))?;
    let bluez_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(BLUEZ_SERVICE)
        .map_err(|error| Error::new("build Bluetooth signal filter", error.to_string()))?
        .build();
    let owner_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .path("/org/freedesktop/DBus")
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .interface("org.freedesktop.DBus")
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .member("NameOwnerChanged")
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .add_arg(BLUEZ_SERVICE)
        .map_err(|error| Error::new("build Bluetooth owner filter", error.to_string()))?
        .build();
    let mut bluez = MessageStream::for_match_rule(bluez_rule, &connection, Some(64))
        .await
        .map_err(|error| Error::new("subscribe to Bluetooth changes", error.to_string()))?
        .fuse();
    let mut owners = MessageStream::for_match_rule(owner_rule, &connection, Some(8))
        .await
        .map_err(|error| Error::new("subscribe to BlueZ restarts", error.to_string()))?
        .fuse();

    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|error| Error::new("inspect BlueZ service", error.to_string()))?;
    let service = zbus::names::BusName::try_from(BLUEZ_SERVICE)
        .map_err(|error| Error::new("inspect BlueZ service", error.to_string()))?;
    let mut available = dbus
        .name_has_owner(service)
        .await
        .map_err(|error| Error::new("inspect BlueZ service", error.to_string()))?;
    if available {
        publish_changed(sender, unavailable_reported).await?;
    } else {
        publish_unavailable(sender, unavailable_reported).await?;
    }

    loop {
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(closed);
        let signal = futures_util::select! {
            message = bluez.next() => {
                read_signal(message)?;
                Some(true)
            },
            message = owners.next() => read_bluez_owner(message)?,
            _ = closed => return Ok(()),
        };
        let Some(signal_available) = signal else {
            continue;
        };
        if !signal_available {
            available = false;
            publish_unavailable(sender, unavailable_reported).await?;
            continue;
        }
        if !available {
            available = true;
        }
        let mut refresh_pending = true;

        loop {
            let quiet = futures_util::FutureExt::fuse(async_io::Timer::after(WATCH_QUIET_PERIOD));
            let closed = sender.closed().fuse();
            futures_util::pin_mut!(quiet, closed);
            let signal = futures_util::select! {
                message = bluez.next() => {
                    read_signal(message)?;
                    Some(true)
                },
                message = owners.next() => read_bluez_owner(message)?,
                _ = quiet => break,
                _ = closed => return Ok(()),
            };
            if let Some(signal_available) = signal {
                available = signal_available;
                refresh_pending = signal_available;
                if !signal_available {
                    publish_unavailable(sender, unavailable_reported).await?;
                }
            }
        }
        if available && refresh_pending {
            publish_changed(sender, unavailable_reported).await?;
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn read_signal(message: Option<Result<zbus::Message, zbus::Error>>) -> Result<(), Error> {
    match message {
        Some(Ok(_)) => Ok(()),
        Some(Err(error)) => Err(Error::new("read Bluetooth change", error.to_string())),
        None => Err(Error::new(
            "read Bluetooth change",
            "the signal stream ended",
        )),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_bluez_owner(
    message: Option<Result<zbus::Message, zbus::Error>>,
) -> Result<Option<bool>, Error> {
    let message = message
        .ok_or_else(|| Error::new("read BlueZ owner", "the signal stream ended"))?
        .map_err(|error| Error::new("read BlueZ owner", error.to_string()))?;
    let (name, _old_owner, new_owner): (String, String, String) = message
        .body()
        .deserialize()
        .map_err(|error| Error::new("read BlueZ owner", error.to_string()))?;
    Ok(bluez_owner_availability(&name, &new_owner))
}

#[cfg(any(not(target_os = "macos"), test))]
fn bluez_owner_availability(name: &str, new_owner: &str) -> Option<bool> {
    (name == "org.bluez").then_some(!new_owner.is_empty())
}

#[cfg(not(target_os = "macos"))]
async fn publish_changed(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if *unavailable_reported {
        sender
            .send(WatchEvent::Changed)
            .await
            .map_err(|_| Error::new("publish Bluetooth change", "the event consumer closed"))?;
    } else {
        let _ = sender.try_send(WatchEvent::Changed);
    }
    *unavailable_reported = false;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn publish_unavailable(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if !*unavailable_reported {
        sender
            .send(WatchEvent::Unavailable)
            .await
            .map_err(|_| Error::new("publish Bluetooth outage", "the event consumer closed"))?;
        *unavailable_reported = true;
    }
    Ok(())
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

    fn pair(&self, _device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
        session.finish();
        Err(Error::new(
            "pair Bluetooth device",
            "the macOS development adapter is read-only",
        ))
    }

    fn cancel_pairing(&self, _device_id: &str) -> Result<(), Error> {
        Err(Error::new(
            "cancel Bluetooth pairing",
            "the macOS development adapter is read-only",
        ))
    }

    fn remove_device(&self, _device_id: &str) -> Result<Snapshot, Error> {
        Err(Error::new(
            "forget Bluetooth device",
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
                    trusted: true,
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
    fn bluez_owner_changes_only_match_the_bluez_service() {
        assert_eq!(bluez_owner_availability("org.bluez", ":1.42"), Some(true));
        assert_eq!(bluez_owner_availability("org.bluez", ""), Some(false));
        assert_eq!(bluez_owner_availability("org.example.Other", ""), None);
    }

    #[test]
    fn pairing_errors_keep_user_outcomes_typed() {
        let (session, _events) = PairingSession::new();
        session.cancel();
        let error = Error::pairing("pair Bluetooth device", "D-Bus failure", &session);
        assert!(error.is_canceled());
        assert!(!error.is_rejected());
        assert!(!error.is_timed_out());
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
                trusted: false,
                connected: false,
            },
            Device {
                id: "paired".into(),
                name: "Paired".into(),
                address: String::new(),
                kind: String::new(),
                paired: true,
                trusted: true,
                connected: false,
            },
            Device {
                id: "connected".into(),
                name: "Connected".into(),
                address: String::new(),
                kind: String::new(),
                paired: true,
                trusted: true,
                connected: true,
            },
        ];

        sort_devices(&mut devices);

        assert_eq!(devices[0].id, "connected");
        assert_eq!(devices[1].id, "paired");
        assert_eq!(devices[2].id, "nearby");
    }
}
