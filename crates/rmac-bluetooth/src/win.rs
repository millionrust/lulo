//! Windows Bluetooth through WinRT: the default adapter and its radio
//! (`Windows.Devices.Bluetooth`, `Windows.Devices.Radios`), paired and
//! nearby devices as association endpoints (`Windows.Devices.Enumeration`),
//! and change notifications from device watchers and radio/connection
//! events. Connecting and disconnecting go through Core Audio's Bluetooth
//! kernel-streaming one-shot properties, as Windows' own Sound settings do,
//! so only audio devices can be connected by an app. Battery levels and
//! container ids come from the devices' Plug and Play nodes. Every call
//! here runs on a background thread; nothing polls.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use windows::core::{IInspectable, IUnknown, GUID, HSTRING};
use windows::Devices::Bluetooth::{
    BluetoothAdapter, BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice,
};
use windows::Devices::Enumeration::{
    DeviceInformation, DeviceInformationCustomPairing, DeviceInformationUpdate, DevicePairingKinds,
    DevicePairingRequestedEventArgs, DevicePairingResultStatus, DeviceUnpairingResultStatus,
    DeviceWatcher, DeviceWatcherStatus,
};
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::Foundation::TypedEventHandler;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW,
    CM_Locate_DevNodeW, CM_GETIDLIST_FILTER_ENUMERATOR, CM_LOCATE_DEVNODE_FLAGS,
    CM_LOCATE_DEVNODE_NORMAL, CM_LOCATE_DEVNODE_PHANTOM, CR_BUFFER_SMALL, CR_SUCCESS,
};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_ContainerId;
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_ContainerId, DEVPROPTYPE, DEVPROP_TYPE_BYTE, DEVPROP_TYPE_GUID,
};
use windows::Win32::Foundation::DEVPROPKEY;
use windows::Win32::Media::Audio::{
    eAll, IDeviceTopology, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE,
    DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::Media::KernelStreaming::{
    IKsControl, KSPROPSETID_BtAudio, KSIDENTIFIER, KSIDENTIFIER_0, KSIDENTIFIER_0_0,
    KSPROPERTY_ONESHOT_DISCONNECT, KSPROPERTY_ONESHOT_RECONNECT, KSPROPERTY_TYPE_GET,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToGUID;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use zeroize::Zeroize as _;

use crate::macos::sort_devices;
use crate::pairing_agent::{ExpectedResponse, PairingResponse};
use crate::{
    BluetoothService, Device, Error, PairingEvent, PairingPromptKind, PairingSession, Snapshot,
    SystemBluetoothService, WatchEvent,
};

impl BluetoothService for SystemBluetoothService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        windows_snapshot()
    }

    fn set_powered(&self, powered: bool) -> Result<(), Error> {
        windows_set_powered(powered)
    }

    fn set_discoverable(&self, _discoverable: bool) -> Result<(), Error> {
        // Windows makes the PC discoverable only while its own Bluetooth
        // settings page is open; there is no API for other apps.
        Err(Error::new(
            "change Bluetooth discoverability",
            "Windows does not let apps make this PC discoverable",
        ))
    }

    fn start_discovery(&self) -> Result<(), Error> {
        windows_start_discovery()
    }

    fn stop_discovery(&self) -> Result<(), Error> {
        windows_stop_discovery();
        Ok(())
    }

    fn set_connected(&self, device_id: &str, connected: bool) -> Result<(), Error> {
        windows_set_connected(device_id, connected)
    }

    fn pair(&self, device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
        windows_pair(device_id, session)
    }

    fn cancel_pairing(&self, device_id: &str) -> Result<(), Error> {
        let active = lock(&PAIRING);
        if let Some(active) = active
            .as_ref()
            .filter(|active| active.device_id == device_id)
        {
            (active.cancel)();
        }
        Ok(())
    }

    fn remove_device(&self, device_id: &str) -> Result<Snapshot, Error> {
        windows_remove_device(device_id)
    }
}

/// WinRT needs an apartment on the calling thread; background pool threads
/// may be fresh. Repeated calls on one thread are harmless.
fn ensure_apartment() {
    // SAFETY: a plain WinRT bootstrap call with no pointers or borrows.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A WinRT call that returned a null object fails with an "empty" error
/// whose code is S_OK; its system message ("completed successfully") would
/// mislead, so that case gets a plain explanation.
fn winrt_error(operation: &'static str, error: windows::core::Error) -> Error {
    if error.code().is_ok() {
        Error::new(operation, "Windows returned no object")
    } else {
        Error::new(operation, error.message())
    }
}

fn map_winrt(operation: &'static str) -> impl FnOnce(windows::core::Error) -> Error {
    move |error| winrt_error(operation, error)
}

/// Splits a null WinRT result (absent hardware) from a real failure.
fn optional<T>(result: windows::core::Result<T>) -> windows::core::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.code().is_ok() => Ok(None),
        Err(error) => Err(error),
    }
}

fn default_adapter() -> windows::core::Result<Option<BluetoothAdapter>> {
    optional(BluetoothAdapter::GetDefaultAsync().and_then(|operation| operation.get()))
}

fn adapter_radio(adapter: &BluetoothAdapter) -> windows::core::Result<Option<Radio>> {
    optional(
        adapter
            .GetRadioAsync()
            .and_then(|operation| operation.get()),
    )
}

/// The Bluetooth radio: the default adapter's, or else the first Bluetooth
/// radio Windows lists, so a radio whose adapter object Windows withholds
/// (some drivers do while the radio is off) still shows, switched off.
fn bluetooth_radio(adapter: Option<&BluetoothAdapter>) -> windows::core::Result<Option<Radio>> {
    if let Some(radio) = adapter.map(adapter_radio).transpose()?.flatten() {
        return Ok(Some(radio));
    }
    let radios = Radio::GetRadiosAsync()?.get()?;
    for index in 0..radios.Size()? {
        let radio = radios.GetAt(index)?;
        if radio.Kind()? == RadioKind::Bluetooth {
            return Ok(Some(radio));
        }
    }
    Ok(None)
}

fn radio_name(radio: Option<&Radio>) -> Option<String> {
    radio
        .and_then(|radio| radio.Name().ok())
        .map(|name| name.to_string_lossy())
        .filter(|name| !name.is_empty())
}

fn adapter_name(adapter: &BluetoothAdapter, radio: Option<&Radio>) -> Option<String> {
    let device_name = adapter
        .DeviceId()
        .and_then(|id| DeviceInformation::CreateFromIdAsync(&id))
        .and_then(|operation| operation.get())
        .and_then(|information| information.Name())
        .map(|name| name.to_string_lossy())
        .ok()
        .filter(|name| !name.is_empty());
    device_name.or_else(|| radio_name(radio))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    Classic,
    LowEnergy,
}

/// Association-endpoint selectors for classic and LE devices in one
/// pairing state.
fn selectors(paired: bool) -> windows::core::Result<[(HSTRING, Transport); 2]> {
    Ok([
        (
            BluetoothDevice::GetDeviceSelectorFromPairingState(paired)?,
            Transport::Classic,
        ),
        (
            BluetoothLEDevice::GetDeviceSelectorFromPairingState(paired)?,
            Transport::LowEnergy,
        ),
    ])
}

/// Association-endpoint ids name the protocol first:
/// `Bluetooth#Bluetooth<radio>-<device>` or `BluetoothLE#BluetoothLE...`.
fn transport_from_id(id: &str) -> Transport {
    if id.starts_with("BluetoothLE#") {
        Transport::LowEnergy
    } else {
        Transport::Classic
    }
}

#[derive(Clone, Debug, Default)]
struct Details {
    address: String,
    kind: String,
    connected: bool,
}

/// Address, device kind and connection state from the typed device object;
/// falls back to the address embedded in the id when Windows cannot open it.
fn device_details(id: &HSTRING, transport: Transport) -> Details {
    let opened = match transport {
        Transport::Classic => classic_details(id),
        Transport::LowEnergy => low_energy_details(id),
    };
    opened.unwrap_or_else(|_| Details {
        address: address_from_id(&id.to_string_lossy()).unwrap_or_default(),
        ..Details::default()
    })
}

fn classic_details(id: &HSTRING) -> windows::core::Result<Details> {
    let device = BluetoothDevice::FromIdAsync(id)?.get()?;
    let kind = device
        .ClassOfDevice()
        .and_then(|class| class.RawValue())
        .ok()
        .and_then(class_icon)
        .unwrap_or_default();
    Ok(Details {
        address: format_address(device.BluetoothAddress()?),
        kind: kind.to_string(),
        connected: device.ConnectionStatus()? == BluetoothConnectionStatus::Connected,
    })
}

fn low_energy_details(id: &HSTRING) -> windows::core::Result<Details> {
    let device = BluetoothLEDevice::FromIdAsync(id)?.get()?;
    let kind = device
        .Appearance()
        .and_then(|appearance| appearance.RawValue())
        .ok()
        .and_then(appearance_icon)
        .unwrap_or_default();
    Ok(Details {
        address: format_address(device.BluetoothAddress()?),
        kind: kind.to_string(),
        connected: device.ConnectionStatus()? == BluetoothConnectionStatus::Connected,
    })
}

fn windows_snapshot() -> Result<Snapshot, Error> {
    ensure_apartment();
    let operation = "read Bluetooth state";
    // A switched-off radio is still present: its adapter, radio and paired
    // devices stay listed, with `powered: false`.
    let adapter = default_adapter().map_err(map_winrt(operation))?;
    let radio = bluetooth_radio(adapter.as_ref()).map_err(map_winrt(operation))?;
    if adapter.is_none() && radio.is_none() {
        return Ok(Snapshot::default());
    }
    let powered = match &radio {
        Some(radio) => radio.State().map_err(map_winrt(operation))? == RadioState::On,
        None => false,
    };
    let adapter_name = match &adapter {
        Some(adapter) => adapter_name(adapter, radio.as_ref()),
        None => radio_name(radio.as_ref()),
    };

    let mut devices = paired_devices()?;
    let paired_ids = devices
        .iter()
        .map(|device| device.id.clone())
        .collect::<HashSet<_>>();
    devices.extend(nearby_devices(&paired_ids));
    sort_devices(&mut devices);
    Ok(Snapshot {
        available: true,
        powered,
        discoverable: false,
        discovering: !lock(&DISCOVERY).watchers.is_empty(),
        adapter_name,
        devices,
    })
}

fn paired_devices() -> Result<Vec<Device>, Error> {
    let operation = "list paired Bluetooth devices";
    let nodes = bluetooth_instance_ids();
    let mut devices = Vec::new();
    let mut containers = Vec::new();
    for (selector, transport) in selectors(true).map_err(map_winrt(operation))? {
        let found = DeviceInformation::FindAllAsyncAqsFilter(&selector)
            .and_then(|operation| operation.get())
            .map_err(map_winrt(operation))?;
        for index in 0..found.Size().map_err(map_winrt(operation))? {
            let Ok(information) = found.GetAt(index) else {
                continue;
            };
            let Ok(id) = information.Id() else {
                continue;
            };
            let details = device_details(&id, transport);
            let name = information
                .Name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            let name = if name.is_empty() {
                details.address.clone()
            } else {
                name
            };
            let device_nodes = nodes_for_address(&nodes, &details.address);
            containers.push(container_id(&device_nodes));
            // A stored level outlives the connection; only a connected
            // device's level is current.
            let battery = details
                .connected
                .then(|| battery_level(&device_nodes))
                .flatten();
            // Windows has no separate trust flag: a paired device is
            // allowed to reconnect.
            devices.push(Device {
                id: id.to_string_lossy(),
                name,
                address: details.address,
                kind: details.kind,
                paired: true,
                trusted: true,
                connected: details.connected,
                can_connect: false,
                battery,
            });
        }
    }
    let wanted = containers.iter().flatten().copied().collect::<HashSet<_>>();
    let connectable = bluetooth_audio_controls(&wanted)
        .into_iter()
        .map(|(container, _)| container)
        .collect::<HashSet<_>>();
    for (device, container) in devices.iter_mut().zip(containers) {
        device.can_connect = container.is_some_and(|container| connectable.contains(&container));
    }
    Ok(devices)
}

/// Devices the discovery watchers have seen, minus paired ones and the
/// nameless LE advertisers Windows' own list hides too. Kind and address
/// are read once per device and kept with the discovery entry.
fn nearby_devices(paired_ids: &HashSet<String>) -> Vec<Device> {
    let pending = lock(&DISCOVERY)
        .nearby
        .iter()
        .filter(|(_, nearby)| nearby.details.is_none())
        .map(|(id, nearby)| (id.clone(), nearby.transport))
        .collect::<Vec<_>>();
    let resolved = pending
        .into_iter()
        .map(|(id, transport)| {
            let details = device_details(&HSTRING::from(id.as_str()), transport);
            (id, details)
        })
        .collect::<Vec<_>>();

    let mut state = lock(&DISCOVERY);
    for (id, details) in resolved {
        if let Some(nearby) = state.nearby.get_mut(&id) {
            nearby.details = Some(details);
        }
    }
    state
        .nearby
        .iter()
        .filter(|(id, _)| !paired_ids.contains(*id))
        .filter_map(|(id, nearby)| {
            let paired = nearby
                .information
                .Pairing()
                .and_then(|pairing| pairing.IsPaired())
                .unwrap_or(false);
            if paired {
                return None;
            }
            let name = nearby.information.Name().ok()?.to_string_lossy();
            if name.is_empty() {
                return None;
            }
            let details = nearby.details.clone().unwrap_or_default();
            Some(Device {
                id: id.clone(),
                name,
                address: details.address,
                kind: details.kind,
                paired: false,
                trusted: false,
                connected: false,
                can_connect: false,
                battery: None,
            })
        })
        .collect()
}

fn windows_set_powered(powered: bool) -> Result<(), Error> {
    ensure_apartment();
    let operation = "change Bluetooth power";
    let adapter = default_adapter().map_err(map_winrt(operation))?;
    let radio = bluetooth_radio(adapter.as_ref())
        .map_err(map_winrt(operation))?
        .ok_or_else(|| {
            if adapter.is_some() {
                Error::new(operation, "Windows did not expose the Bluetooth radio")
            } else {
                Error::new(operation, "no Bluetooth adapter found")
            }
        })?;
    let access = Radio::RequestAccessAsync()
        .and_then(|request| request.get())
        .map_err(map_winrt(operation))?;
    if let Some(detail) = radio_access_denial(access) {
        return Err(Error::new(operation, detail));
    }
    let state = if powered {
        RadioState::On
    } else {
        RadioState::Off
    };
    let status = radio
        .SetStateAsync(state)
        .and_then(|change| change.get())
        .map_err(map_winrt(operation))?;
    if status == RadioAccessStatus::Allowed {
        Ok(())
    } else {
        Err(Error::new(
            operation,
            radio_access_denial(status).unwrap_or("Windows did not change the radio"),
        ))
    }
}

fn radio_access_denial(status: RadioAccessStatus) -> Option<&'static str> {
    if status == RadioAccessStatus::DeniedByUser {
        Some("radio control is turned off for apps in Windows privacy settings")
    } else if status == RadioAccessStatus::DeniedBySystem {
        Some("Windows does not allow changing the radio now (airplane mode or policy)")
    } else {
        None
    }
}

// Discovery: watchers over unpaired classic and LE endpoints, run between
// `start_discovery` and `stop_discovery`. What they find stays listed after
// they stop, until the next discovery starts.

struct Nearby {
    information: DeviceInformation,
    transport: Transport,
    details: Option<Details>,
}

#[derive(Default)]
struct DiscoveryState {
    watchers: Vec<Watcher>,
    nearby: HashMap<String, Nearby>,
}

static DISCOVERY: LazyLock<Mutex<DiscoveryState>> = LazyLock::new(Default::default);

/// Live `watch` streams; discovery events reach them through this list.
static SUBSCRIBERS: Mutex<Vec<async_channel::Sender<WatchEvent>>> = Mutex::new(Vec::new());

fn notify_subscribers() {
    let mut subscribers = lock(&SUBSCRIBERS);
    subscribers.retain(|sender| !sender.is_closed());
    for sender in subscribers.iter() {
        let _ = sender.try_send(WatchEvent::Changed);
    }
}

fn windows_start_discovery() -> Result<(), Error> {
    ensure_apartment();
    let operation = "start Bluetooth discovery";
    if default_adapter().map_err(map_winrt(operation))?.is_none() {
        return Err(Error::new(operation, "no Bluetooth adapter found"));
    }
    {
        let mut state = lock(&DISCOVERY);
        if !state.watchers.is_empty() {
            return Ok(());
        }
        state.nearby.clear();
    }
    // Built without the lock held: watcher callbacks take it.
    let mut watchers = Vec::new();
    for (selector, transport) in selectors(false).map_err(map_winrt(operation))? {
        watchers.push(discovery_watcher(&selector, transport).map_err(map_winrt(operation))?);
    }
    let previous = std::mem::replace(&mut lock(&DISCOVERY).watchers, watchers);
    drop(previous);
    notify_subscribers();
    Ok(())
}

fn windows_stop_discovery() {
    ensure_apartment();
    let watchers = std::mem::take(&mut lock(&DISCOVERY).watchers);
    let stopped = !watchers.is_empty();
    drop(watchers);
    if stopped {
        notify_subscribers();
    }
}

fn discovery_watcher(selector: &HSTRING, transport: Transport) -> windows::core::Result<Watcher> {
    Watcher::start(
        selector,
        move |information| {
            let Ok(id) = information.Id() else {
                return;
            };
            lock(&DISCOVERY).nearby.insert(
                id.to_string_lossy(),
                Nearby {
                    information: information.clone(),
                    transport,
                    details: None,
                },
            );
            notify_subscribers();
        },
        |update| {
            let Ok(id) = update.Id() else {
                return;
            };
            // LE advertisers update often; only a new name changes the list.
            let renamed = lock(&DISCOVERY)
                .nearby
                .get(&id.to_string_lossy())
                .is_some_and(|nearby| {
                    let before = nearby.information.Name().ok();
                    let _ = nearby.information.Update(update);
                    nearby.information.Name().ok() != before
                });
            if renamed {
                notify_subscribers();
            }
        },
        |update| {
            let Ok(id) = update.Id() else {
                return;
            };
            let removed = lock(&DISCOVERY)
                .nearby
                .remove(&id.to_string_lossy())
                .is_some();
            if removed {
                notify_subscribers();
            }
        },
        || {},
    )
}

/// A started `DeviceWatcher` with its four handlers; dropping it removes
/// the handlers and stops the watcher.
struct Watcher {
    watcher: DeviceWatcher,
    added: i64,
    updated: i64,
    removed: i64,
    enumerated: i64,
}

impl Watcher {
    fn start(
        selector: &HSTRING,
        mut on_added: impl FnMut(&DeviceInformation) + Send + 'static,
        mut on_updated: impl FnMut(&DeviceInformationUpdate) + Send + 'static,
        mut on_removed: impl FnMut(&DeviceInformationUpdate) + Send + 'static,
        mut on_enumerated: impl FnMut() + Send + 'static,
    ) -> windows::core::Result<Self> {
        let watcher = DeviceInformation::CreateWatcherAqsFilter(selector)?;
        // An association-endpoint watcher reports nothing unless Added,
        // Updated and Removed all have handlers.
        let added = watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
            move |_, information| {
                if let Some(information) = information.as_ref() {
                    on_added(information);
                }
                Ok(())
            },
        ))?;
        let updated = watcher.Updated(&TypedEventHandler::<
            DeviceWatcher,
            DeviceInformationUpdate,
        >::new(move |_, update| {
            if let Some(update) = update.as_ref() {
                on_updated(update);
            }
            Ok(())
        }))?;
        let removed = watcher.Removed(&TypedEventHandler::<
            DeviceWatcher,
            DeviceInformationUpdate,
        >::new(move |_, update| {
            if let Some(update) = update.as_ref() {
                on_removed(update);
            }
            Ok(())
        }))?;
        let enumerated = watcher.EnumerationCompleted(&TypedEventHandler::<
            DeviceWatcher,
            IInspectable,
        >::new(move |_, _| {
            on_enumerated();
            Ok(())
        }))?;
        let watcher = Self {
            watcher,
            added,
            updated,
            removed,
            enumerated,
        };
        watcher.watcher.Start()?;
        Ok(watcher)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        ensure_apartment();
        let _ = self.watcher.RemoveAdded(self.added);
        let _ = self.watcher.RemoveUpdated(self.updated);
        let _ = self.watcher.RemoveRemoved(self.removed);
        let _ = self.watcher.RemoveEnumerationCompleted(self.enumerated);
        if matches!(
            self.watcher.Status(),
            Ok(DeviceWatcherStatus::Started | DeviceWatcherStatus::EnumerationCompleted)
        ) {
            let _ = self.watcher.Stop();
        }
    }
}

// Watching: adapters arriving or leaving, pairings added or removed, the
// radio switching, and paired devices connecting. Structural changes
// rebuild the subscriptions so new radios and devices are covered too.

type Notify = Arc<dyn Fn() + Send + Sync>;

struct Subscriptions {
    _watchers: Vec<Watcher>,
    radio: Option<(Radio, i64)>,
    classic: Vec<(BluetoothDevice, i64)>,
    low_energy: Vec<(BluetoothLEDevice, i64)>,
}

impl Drop for Subscriptions {
    fn drop(&mut self) {
        ensure_apartment();
        if let Some((radio, token)) = &self.radio {
            let _ = radio.RemoveStateChanged(*token);
        }
        for (device, token) in &self.classic {
            let _ = device.RemoveConnectionStatusChanged(*token);
        }
        for (device, token) in &self.low_energy {
            let _ = device.RemoveConnectionStatusChanged(*token);
        }
    }
}

/// A watcher whose initial enumeration is ignored: only later arrivals and
/// removals rebuild, and later updates (renames) report a change.
fn structural_watcher(
    selector: &HSTRING,
    changed: &Notify,
    rebuild: &Notify,
) -> windows::core::Result<Watcher> {
    let enumerated = Arc::new(AtomicBool::new(false));
    let added = {
        let enumerated = enumerated.clone();
        let rebuild = rebuild.clone();
        move |_: &DeviceInformation| {
            if enumerated.load(Ordering::Acquire) {
                rebuild();
            }
        }
    };
    let updated = {
        let enumerated = enumerated.clone();
        let changed = changed.clone();
        move |_: &DeviceInformationUpdate| {
            if enumerated.load(Ordering::Acquire) {
                changed();
            }
        }
    };
    let removed = {
        let enumerated = enumerated.clone();
        let rebuild = rebuild.clone();
        move |_: &DeviceInformationUpdate| {
            if enumerated.load(Ordering::Acquire) {
                rebuild();
            }
        }
    };
    let completed = move || enumerated.store(true, Ordering::Release);
    Watcher::start(selector, added, updated, removed, completed)
}

fn subscribe(changed: &Notify, rebuild: &Notify) -> windows::core::Result<Subscriptions> {
    ensure_apartment();
    let mut subscriptions = Subscriptions {
        _watchers: Vec::new(),
        radio: None,
        classic: Vec::new(),
        low_energy: Vec::new(),
    };
    subscriptions._watchers.push(structural_watcher(
        &BluetoothAdapter::GetDeviceSelector()?,
        changed,
        rebuild,
    )?);
    let paired = selectors(true)?;
    for (selector, _) in &paired {
        subscriptions
            ._watchers
            .push(structural_watcher(selector, changed, rebuild)?);
    }

    if let Some(radio) = bluetooth_radio(default_adapter()?.as_ref())? {
        let notify = changed.clone();
        let token = radio.StateChanged(&TypedEventHandler::<Radio, IInspectable>::new(
            move |_, _| {
                notify();
                Ok(())
            },
        ))?;
        subscriptions.radio = Some((radio, token));
    }

    for (selector, transport) in &paired {
        let found = DeviceInformation::FindAllAsyncAqsFilter(selector)?.get()?;
        for index in 0..found.Size()? {
            let Ok(id) = found.GetAt(index).and_then(|information| information.Id()) else {
                continue;
            };
            let notify = changed.clone();
            match transport {
                Transport::Classic => {
                    let Ok(device) = BluetoothDevice::FromIdAsync(&id).and_then(|open| open.get())
                    else {
                        continue;
                    };
                    let handler =
                        TypedEventHandler::<BluetoothDevice, IInspectable>::new(move |_, _| {
                            notify();
                            Ok(())
                        });
                    if let Ok(token) = device.ConnectionStatusChanged(&handler) {
                        subscriptions.classic.push((device, token));
                    }
                }
                Transport::LowEnergy => {
                    let Ok(device) =
                        BluetoothLEDevice::FromIdAsync(&id).and_then(|open| open.get())
                    else {
                        continue;
                    };
                    let handler =
                        TypedEventHandler::<BluetoothLEDevice, IInspectable>::new(move |_, _| {
                            notify();
                            Ok(())
                        });
                    if let Ok(token) = device.ConnectionStatusChanged(&handler) {
                        subscriptions.low_energy.push((device, token));
                    }
                }
            }
        }
    }
    Ok(subscriptions)
}

/// Removes a `watch` stream from the discovery subscribers when it ends.
struct Subscriber(async_channel::Sender<WatchEvent>);

impl Drop for Subscriber {
    fn drop(&mut self) {
        lock(&SUBSCRIBERS).retain(|sender| !sender.same_channel(&self.0));
    }
}

pub(crate) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    use futures_util::future::{select, Either};

    lock(&SUBSCRIBERS).push(sender.clone());
    let _subscriber = Subscriber(sender.clone());
    let (wake, woken) = async_channel::bounded::<()>(1);
    loop {
        let changed: Notify = {
            let sender = sender.clone();
            Arc::new(move || {
                let _ = sender.try_send(WatchEvent::Changed);
            })
        };
        let rebuild: Notify = {
            let wake = wake.clone();
            Arc::new(move || {
                let _ = wake.try_send(());
            })
        };
        let subscriptions = subscribe(&changed, &rebuild).ok();
        if subscriptions.is_some() {
            let _ = sender.try_send(WatchEvent::Changed);
        } else if sender.send(WatchEvent::Unavailable).await.is_err() {
            return Ok(());
        }

        let closed = std::pin::pin!(sender.closed());
        let next = std::pin::pin!(woken.recv());
        match select(closed, next).await {
            Either::Left(_) | Either::Right((Err(_), _)) => return Ok(()),
            Either::Right((Ok(()), _)) => {}
        }
        ensure_apartment();
        drop(subscriptions);
    }
}

// Pairing.

struct ActivePairing {
    device_id: String,
    cancel: Box<dyn Fn() + Send>,
}

static PAIRING: Mutex<Option<ActivePairing>> = Mutex::new(None);

/// The device's enumeration record: the one discovery found if it is still
/// listed, otherwise the one Windows keeps for the device object.
fn device_information(device_id: &str) -> windows::core::Result<DeviceInformation> {
    if let Some(nearby) = lock(&DISCOVERY).nearby.get(device_id) {
        return Ok(nearby.information.clone());
    }
    let id = HSTRING::from(device_id);
    match transport_from_id(device_id) {
        Transport::Classic => BluetoothDevice::FromIdAsync(&id)?
            .get()?
            .DeviceInformation(),
        Transport::LowEnergy => BluetoothLEDevice::FromIdAsync(&id)?
            .get()?
            .DeviceInformation(),
    }
}

fn windows_pair(device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
    let operation = "pair Bluetooth device";
    let result = (|| {
        ensure_apartment();
        let information = device_information(device_id).map_err(|error| {
            if error.code().is_ok() {
                Error::new(operation, "the selected device is no longer available")
            } else {
                winrt_error(operation, error)
            }
        })?;
        let pairing = information.Pairing().map_err(map_winrt(operation))?;
        if pairing.IsPaired().map_err(map_winrt(operation))? {
            return Err(Error::new(
                operation,
                "the selected device is already paired",
            ));
        }
        if session.outcome().canceled {
            return Err(Error::pairing(operation, "pairing was canceled", session));
        }

        let custom = pairing.Custom().map_err(map_winrt(operation))?;
        let handler_session = session.clone();
        let token = custom
            .PairingRequested(&TypedEventHandler::<
                DeviceInformationCustomPairing,
                DevicePairingRequestedEventArgs,
            >::new(move |_, request| {
                if let Some(request) = request.as_ref() {
                    answer_pairing_request(&handler_session, request)?;
                }
                Ok(())
            }))
            .map_err(map_winrt(operation))?;
        let kinds = DevicePairingKinds::ConfirmOnly
            | DevicePairingKinds::DisplayPin
            | DevicePairingKinds::ProvidePin
            | DevicePairingKinds::ConfirmPinMatch;
        let outcome = custom.PairAsync(kinds).and_then(|pairing_operation| {
            let cancel_handle = pairing_operation.clone();
            *lock(&PAIRING) = Some(ActivePairing {
                device_id: device_id.to_string(),
                cancel: Box::new(move || {
                    let _ = cancel_handle.Cancel();
                }),
            });
            // A cancel that arrived before the operation was registered.
            if session.outcome().canceled {
                let _ = pairing_operation.Cancel();
            }
            let result = pairing_operation.get();
            *lock(&PAIRING) = None;
            result
        });
        let _ = custom.RemovePairingRequested(token);
        let status = outcome
            .and_then(|result| result.Status())
            .map_err(|error| Error::pairing(operation, error.message(), session))?;
        if status != DevicePairingResultStatus::Paired
            && status != DevicePairingResultStatus::AlreadyPaired
        {
            return Err(Error::pairing(
                operation,
                pairing_status_detail(status),
                session,
            ));
        }
        lock(&DISCOVERY).nearby.remove(device_id);
        windows_snapshot()
    })();
    session.finish();
    result
}

/// Answers one Windows pairing ceremony step through the session's
/// prompts. Returning without accepting declines the step.
fn answer_pairing_request(
    session: &PairingSession,
    request: &DevicePairingRequestedEventArgs,
) -> windows::core::Result<()> {
    let kind = request.PairingKind()?;
    if kind == DevicePairingKinds::ConfirmOnly {
        // The user already chose Pair for this device.
        return request.Accept();
    }
    if kind == DevicePairingKinds::DisplayPin {
        let pin_code = request.Pin()?.to_string_lossy();
        if session
            .display(PairingEvent::DisplayPinCode { pin_code })
            .is_ok()
        {
            request.Accept()?;
        }
        return Ok(());
    }
    // The remaining steps wait for the user, so Windows is told to hold the
    // ceremony open until the answer arrives.
    let deferral = request.GetDeferral()?;
    let answered = if kind == DevicePairingKinds::ConfirmPinMatch {
        match parse_passkey(&request.Pin()?.to_string_lossy()) {
            Some(passkey) => match session.request(
                ExpectedResponse::Confirmation,
                PairingPromptKind::ConfirmPasskey { passkey },
            ) {
                Ok(PairingResponse::Accept) => request.Accept(),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    } else if kind == DevicePairingKinds::ProvidePin {
        match session.request(ExpectedResponse::PinCode, PairingPromptKind::EnterPinCode) {
            Ok(PairingResponse::PinCode(value)) => {
                let mut pin = value.into_string();
                let result = request.AcceptWithPin(&HSTRING::from(pin.as_str()));
                pin.zeroize();
                result
            }
            _ => Ok(()),
        }
    } else {
        Ok(())
    };
    let completed = deferral.Complete();
    answered.and(completed)
}

fn pairing_status_detail(status: DevicePairingResultStatus) -> String {
    // The "Authentication…" words let `Error::pairing` classify the outcome.
    let detail = if status == DevicePairingResultStatus::AuthenticationTimeout {
        "AuthenticationTimeout: the device did not answer in time"
    } else if status == DevicePairingResultStatus::PairingCanceled {
        "AuthenticationCanceled: pairing was canceled"
    } else if status == DevicePairingResultStatus::RejectedByHandler
        || status == DevicePairingResultStatus::ConnectionRejected
    {
        "AuthenticationRejected: the pairing request was rejected"
    } else if status == DevicePairingResultStatus::NotReadyToPair {
        "the device is not ready to pair; make sure it is in pairing mode"
    } else if status == DevicePairingResultStatus::AuthenticationFailure {
        "authentication failed; the code may not have matched"
    } else if status == DevicePairingResultStatus::AuthenticationNotAllowed {
        "Windows does not allow this kind of authentication"
    } else if status == DevicePairingResultStatus::NoSupportedProfiles {
        "the device offers no services Windows supports"
    } else if status == DevicePairingResultStatus::TooManyConnections {
        "too many Bluetooth connections are open"
    } else if status == DevicePairingResultStatus::HardwareFailure {
        "the Bluetooth hardware failed"
    } else if status == DevicePairingResultStatus::AccessDenied {
        "Windows denied access to pairing"
    } else if status == DevicePairingResultStatus::OperationAlreadyInProgress {
        "another pairing is already in progress"
    } else if status == DevicePairingResultStatus::RemoteDeviceHasAssociation {
        "the device is already paired with another computer"
    } else {
        "Windows could not pair the device"
    };
    format!("{detail} (status {})", status.0)
}

fn windows_remove_device(device_id: &str) -> Result<Snapshot, Error> {
    ensure_apartment();
    let operation = "forget Bluetooth device";
    let information = device_information(device_id).map_err(|error| {
        if error.code().is_ok() {
            Error::new(operation, "the selected device is no longer available")
        } else {
            winrt_error(operation, error)
        }
    })?;
    let status = information
        .Pairing()
        .and_then(|pairing| pairing.UnpairAsync())
        .and_then(|unpair| unpair.get())
        .and_then(|result| result.Status())
        .map_err(map_winrt(operation))?;
    if status != DeviceUnpairingResultStatus::Unpaired
        && status != DeviceUnpairingResultStatus::AlreadyUnpaired
    {
        let detail = if status == DeviceUnpairingResultStatus::AccessDenied {
            "Windows denied access"
        } else if status == DeviceUnpairingResultStatus::OperationAlreadyInProgress {
            "another pairing change is in progress"
        } else {
            "Windows could not remove the device"
        };
        return Err(Error::new(operation, detail));
    }
    windows_snapshot()
}

// Connecting: Windows lets apps connect and disconnect only a device's
// Bluetooth audio. Each audio endpoint of the device (matched by the
// device's container id) leads through its topology to the Bluetooth
// audio driver's kernel-streaming filter, which takes the one-shot
// reconnect and disconnect properties (stereo and hands-free alike).

fn windows_set_connected(device_id: &str, connected: bool) -> Result<(), Error> {
    ensure_apartment();
    let operation = if connected {
        "connect Bluetooth device"
    } else {
        "disconnect Bluetooth device"
    };
    let details = device_details(&HSTRING::from(device_id), transport_from_id(device_id));
    let nodes = bluetooth_instance_ids();
    let container = container_id(&nodes_for_address(&nodes, &details.address))
        .ok_or_else(|| Error::new(operation, "Windows has no record of this paired device"))?;
    let controls = bluetooth_audio_controls(&HashSet::from([container]));
    if controls.is_empty() {
        return Err(Error::new(
            operation,
            "Windows lets apps connect only Bluetooth audio devices; it connects this one itself when it is used",
        ));
    }
    let mut connected_any = false;
    let mut failure = None;
    for (_, control) in &controls {
        match ks_one_shot(control, connected) {
            Ok(()) => connected_any = true,
            Err(error) => failure = Some(error),
        }
    }
    match failure {
        Some(error) if !connected_any => Err(winrt_error(operation, error)),
        _ => Ok(()),
    }
}

fn audio_enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    // SAFETY: creates the system's audio endpoint enumerator; the calling
    // thread has joined the multithreaded apartment.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None::<&IUnknown>, CLSCTX_ALL) }
}

/// The Bluetooth audio filters behind the audio endpoints of the wanted
/// containers, each with its endpoint's container. A disconnected
/// headset keeps its endpoints as "unplugged", so those count too.
fn bluetooth_audio_controls(wanted: &HashSet<GUID>) -> Vec<(GUID, IKsControl)> {
    if wanted.is_empty() {
        return Vec::new();
    }
    let Ok(enumerator) = audio_enumerator() else {
        return Vec::new();
    };
    let states =
        DEVICE_STATE(DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_DISABLED.0 | DEVICE_STATE_UNPLUGGED.0);
    // SAFETY: COM calls on a live enumerator and the collection it returns.
    let endpoints = unsafe {
        let Ok(collection) = enumerator.EnumAudioEndpoints(eAll, states) else {
            return Vec::new();
        };
        let count = collection.GetCount().unwrap_or(0);
        (0..count)
            .filter_map(|index| collection.Item(index).ok())
            .collect::<Vec<_>>()
    };
    endpoints
        .iter()
        .filter_map(|endpoint| {
            let container = endpoint_container(endpoint).filter(|id| wanted.contains(id))?;
            let control = endpoint_filter_control(&enumerator, endpoint).ok()?;
            Some((container, control))
        })
        .collect()
}

fn endpoint_container(endpoint: &IMMDevice) -> Option<GUID> {
    // SAFETY: COM calls on a live endpoint and its read-only property store;
    // the PROPVARIANT is owned here and cleared when dropped.
    unsafe {
        let store = endpoint.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_ContainerId).ok()?;
        PropVariantToGUID(&value).ok()
    }
}

/// The kernel-streaming filter the endpoint's first connector leads to: on
/// a Bluetooth endpoint, the Bluetooth audio driver's filter.
fn endpoint_filter_control(
    enumerator: &IMMDeviceEnumerator,
    endpoint: &IMMDevice,
) -> windows::core::Result<IKsControl> {
    // SAFETY: COM calls on live objects; the device id string COM returns
    // is used once and freed here.
    unsafe {
        let topology: IDeviceTopology = endpoint.Activate(CLSCTX_ALL, None)?;
        let connector = topology.GetConnector(0)?;
        let filter_id = connector.GetDeviceIdConnectedTo()?;
        let filter = enumerator.GetDevice(filter_id);
        CoTaskMemFree(Some(filter_id.0.cast_const().cast()));
        filter?.Activate(CLSCTX_ALL, None)
    }
}

/// Sends a Bluetooth audio one-shot property: a get with no data, as the
/// driver defines it.
fn ks_one_shot(control: &IKsControl, connect: bool) -> windows::core::Result<()> {
    let id = if connect {
        KSPROPERTY_ONESHOT_RECONNECT
    } else {
        KSPROPERTY_ONESHOT_DISCONNECT
    };
    let property = KSIDENTIFIER {
        Anonymous: KSIDENTIFIER_0 {
            Anonymous: KSIDENTIFIER_0_0 {
                Set: KSPROPSETID_BtAudio,
                Id: id.0 as u32,
                Flags: KSPROPERTY_TYPE_GET,
            },
        },
    };
    let mut returned = 0u32;
    // SAFETY: `property` and `returned` outlive the call; no data buffer
    // is passed, and its length is zero.
    unsafe {
        control.KsProperty(
            &property,
            std::mem::size_of::<KSIDENTIFIER>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
        )
    }
}

// Plug and Play: a remote device's nodes (`BTHENUM\DEV_<address>\...`, its
// service nodes, and the `BTHLE`/`BTHLEDEVICE` ones for LE) all carry its
// address in their instance ids, and share its container id.

const BLUETOOTH_ENUMERATORS: [&str; 3] = ["BTHENUM", "BTHLE", "BTHLEDEVICE"];

/// `DEVPKEY_Bluetooth_Battery`: the charge Windows' own Bluetooth page
/// shows, one byte in percent. The windows crate does not define it.
const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

/// Instance ids of every node, present or not, under the Bluetooth
/// enumerators. Read once per snapshot.
fn bluetooth_instance_ids() -> Vec<String> {
    BLUETOOTH_ENUMERATORS
        .into_iter()
        .flat_map(instance_ids)
        .collect()
}

fn instance_ids(enumerator: &str) -> Vec<String> {
    let filter = HSTRING::from(enumerator);
    // A node can arrive between the size and list calls; the list call
    // then reports a small buffer and both are read again.
    for _ in 0..3 {
        let mut length = 0u32;
        // SAFETY: `length` outlives the call; the filter is NUL-terminated.
        let status = unsafe {
            CM_Get_Device_ID_List_SizeW(&mut length, &filter, CM_GETIDLIST_FILTER_ENUMERATOR)
        };
        if status != CR_SUCCESS || length == 0 {
            return Vec::new();
        }
        let mut buffer = vec![0u16; length as usize];
        // SAFETY: the buffer and its length are passed together.
        let status =
            unsafe { CM_Get_Device_ID_ListW(&filter, &mut buffer, CM_GETIDLIST_FILTER_ENUMERATOR) };
        if status == CR_BUFFER_SMALL {
            continue;
        }
        if status != CR_SUCCESS {
            return Vec::new();
        }
        return parse_multi_string(&buffer);
    }
    Vec::new()
}

/// The device's nodes, its own `DEV_` node first.
fn nodes_for_address<'a>(ids: &'a [String], address: &str) -> Vec<&'a str> {
    let Some(key) = address_key(address) else {
        return Vec::new();
    };
    let mut nodes = ids
        .iter()
        .filter(|id| id.to_ascii_uppercase().contains(&key))
        .map(String::as_str)
        .collect::<Vec<_>>();
    nodes.sort_by_key(|id| !id.to_ascii_uppercase().contains("\\DEV_"));
    nodes
}

fn locate(id: &str, flags: CM_LOCATE_DEVNODE_FLAGS) -> Option<u32> {
    let mut node = 0u32;
    // SAFETY: `node` outlives the call; the id is NUL-terminated.
    let status = unsafe { CM_Locate_DevNodeW(&mut node, &HSTRING::from(id), flags) };
    (status == CR_SUCCESS).then_some(node)
}

/// A fixed-size property of one node, when it has exactly that type.
fn node_property<T: Copy + Default>(node: u32, key: &DEVPROPKEY, kind: DEVPROPTYPE) -> Option<T> {
    let mut found = DEVPROPTYPE::default();
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    // SAFETY: `value` is a plain `T` of `size` bytes and, like `found` and
    // `size`, outlives the call.
    let status = unsafe {
        CM_Get_DevNode_PropertyW(
            node,
            key,
            &mut found,
            Some((&mut value as *mut T).cast::<u8>()),
            &mut size,
            0,
        )
    };
    (status == CR_SUCCESS && found == kind && size as usize == std::mem::size_of::<T>())
        .then_some(value)
}

/// The device's container id, from any of its nodes; a disconnected or
/// switched-off device's nodes may not be present but still answer.
fn container_id(nodes: &[&str]) -> Option<GUID> {
    nodes.iter().find_map(|id| {
        let node = locate(id, CM_LOCATE_DEVNODE_PHANTOM)?;
        node_property::<GUID>(node, &DEVPKEY_Device_ContainerId, DEVPROP_TYPE_GUID)
            .filter(|container| *container != GUID::zeroed())
    })
}

/// The charge a present node reports (the device node for LE, a service
/// node such as hands-free for classic devices).
fn battery_level(nodes: &[&str]) -> Option<u8> {
    nodes.iter().find_map(|id| {
        let node = locate(id, CM_LOCATE_DEVNODE_NORMAL)?;
        node_property::<u8>(node, &DEVPKEY_BLUETOOTH_BATTERY, DEVPROP_TYPE_BYTE)
            .filter(|percent| *percent <= 100)
    })
}

// Pure helpers.

/// The strings of a NUL-separated, double-NUL-terminated list.
fn parse_multi_string(buffer: &[u16]) -> Vec<String> {
    buffer
        .split(|unit| *unit == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// A formatted address (`AA:BB:CC:DD:EE:FF`) as the twelve upper-case hex
/// digits Plug and Play instance ids use.
fn address_key(address: &str) -> Option<String> {
    let key = address.replace(':', "").to_ascii_uppercase();
    let valid = key.len() == 12
        && key.bytes().all(|byte| byte.is_ascii_hexdigit())
        && key != "000000000000";
    valid.then_some(key)
}

/// Windows passes numeric-comparison codes as decimal text.
fn parse_passkey(pin: &str) -> Option<u32> {
    let pin = pin.trim();
    (!pin.is_empty() && pin.len() <= 6 && pin.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| pin.parse().ok())
        .flatten()
}

/// A 48-bit Bluetooth address as `AA:BB:CC:DD:EE:FF`.
fn format_address(address: u64) -> String {
    (0..6)
        .rev()
        .map(|byte| format!("{:02X}", (address >> (byte * 8)) & 0xff))
        .collect::<Vec<_>>()
        .join(":")
}

/// The device address at the end of an association-endpoint id
/// (`Bluetooth#Bluetooth<radio>-<device>`), upper-cased.
fn address_from_id(id: &str) -> Option<String> {
    let (_, address) = id.rsplit_once('-')?;
    let valid = address.len() == 17
        && address.split(':').count() == 6
        && address
            .split(':')
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()));
    valid.then(|| address.to_ascii_uppercase())
}

/// The BlueZ icon name for a classic Class of Device, so device kinds read
/// the same as on Linux.
fn class_icon(class: u32) -> Option<&'static str> {
    let minor = (class & 0xfc) >> 2;
    match (class & 0x1f00) >> 8 {
        0x01 => Some("computer"),
        0x02 => match minor {
            0x01 | 0x02 | 0x03 | 0x05 => Some("phone"),
            0x04 => Some("modem"),
            _ => None,
        },
        0x03 => Some("network-wireless"),
        0x04 => match minor {
            0x01 | 0x02 => Some("audio-headset"),
            0x06 => Some("audio-headphones"),
            0x0b..=0x0d => Some("camera-video"),
            _ => Some("audio-card"),
        },
        0x05 => match (class & 0xc0) >> 6 {
            0x00 => match (class & 0x3c) >> 2 {
                0x01 | 0x02 => Some("input-gaming"),
                _ => None,
            },
            0x01 => Some("input-keyboard"),
            0x02 => match (class & 0x3c) >> 2 {
                0x05 => Some("input-tablet"),
                _ => Some("input-mouse"),
            },
            _ => None,
        },
        0x06 if class & 0x80 != 0 => Some("printer"),
        0x06 if class & 0x20 != 0 => Some("camera-photo"),
        _ => None,
    }
}

/// The BlueZ icon name for an LE GAP appearance value.
fn appearance_icon(appearance: u16) -> Option<&'static str> {
    match (appearance & 0xffc0) >> 6 {
        0x01 => Some("phone"),
        0x02 => Some("computer"),
        0x05 => Some("video-display"),
        0x0a => Some("multimedia-player"),
        0x0b => Some("scanner"),
        0x0f => match appearance & 0x3f {
            0x01 => Some("input-keyboard"),
            0x02 => Some("input-mouse"),
            0x03 | 0x04 => Some("input-gaming"),
            0x05 => Some("input-tablet"),
            0x08 => Some("scanner"),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_format_as_six_upper_case_octets() {
        assert_eq!(format_address(0x5c_f3_70_8b_2e_a1), "5C:F3:70:8B:2E:A1");
        assert_eq!(format_address(0x01), "00:00:00:00:00:01");
    }

    #[test]
    fn endpoint_ids_yield_their_device_address_and_transport() {
        let classic = "Bluetooth#Bluetooth00:1a:7d:da:71:13-5c:f3:70:8b:2e:a1";
        let le = "BluetoothLE#BluetoothLE00:1a:7d:da:71:13-c1:22:33:44:55:66";
        assert_eq!(
            address_from_id(classic).as_deref(),
            Some("5C:F3:70:8B:2E:A1")
        );
        assert_eq!(address_from_id(le).as_deref(), Some("C1:22:33:44:55:66"));
        assert_eq!(address_from_id("Bluetooth#nothing"), None);
        assert_eq!(transport_from_id(classic), Transport::Classic);
        assert_eq!(transport_from_id(le), Transport::LowEnergy);
    }

    #[test]
    fn class_of_device_maps_to_bluez_icons() {
        // Audio/Video, headphones.
        assert_eq!(class_icon(0x24_0418), Some("audio-headphones"));
        // Audio/Video, wearable headset.
        assert_eq!(class_icon(0x20_0404), Some("audio-headset"));
        // Peripheral, keyboard; peripheral, mouse.
        assert_eq!(class_icon(0x00_0540), Some("input-keyboard"));
        assert_eq!(class_icon(0x00_0580), Some("input-mouse"));
        // Peripheral, gamepad.
        assert_eq!(class_icon(0x00_0508), Some("input-gaming"));
        // Phone, smartphone; computer, laptop.
        assert_eq!(class_icon(0x5a_020c), Some("phone"));
        assert_eq!(class_icon(0x10_010c), Some("computer"));
        assert_eq!(class_icon(0x00_1f00), None);
    }

    #[test]
    fn le_appearance_maps_to_bluez_icons() {
        assert_eq!(appearance_icon(0x03c1), Some("input-keyboard"));
        assert_eq!(appearance_icon(0x03c2), Some("input-mouse"));
        assert_eq!(appearance_icon(0x0040), Some("phone"));
        assert_eq!(appearance_icon(0x0000), None);
    }

    #[test]
    fn device_id_lists_split_at_nuls() {
        let buffer = "BTHENUM\\DEV_5CF3708B2EA1\\8&1&0\0BTHLE\\DEV_C12233445566\\9&2\0\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        assert_eq!(
            parse_multi_string(&buffer),
            [
                "BTHENUM\\DEV_5CF3708B2EA1\\8&1&0",
                "BTHLE\\DEV_C12233445566\\9&2"
            ]
        );
        assert!(parse_multi_string(&[0, 0]).is_empty());
    }

    #[test]
    fn nodes_match_by_address_with_the_device_node_first() {
        let ids = [
            "BTHENUM\\{0000111E-0000-1000-8000-00805F9B34FB}_VID&0001009E_PID&400F\\7&2&0&5CF3708B2EA1_C00000000",
            "BTHENUM\\DEV_5CF3708B2EA1\\7&2&0&BLUETOOTHDEVICE_5CF3708B2EA1",
            "BTHENUM\\DEV_001122334455\\7&2&0&BLUETOOTHDEVICE_001122334455",
        ]
        .map(String::from);
        let nodes = nodes_for_address(&ids, "5C:F3:70:8B:2E:A1");
        assert_eq!(nodes.len(), 2);
        assert!(nodes[0].starts_with("BTHENUM\\DEV_5CF3708B2EA1"));
        assert!(nodes_for_address(&ids, "").is_empty());
        assert_eq!(address_key("00:00:00:00:00:00"), None);
        assert_eq!(
            address_key("5c:f3:70:8b:2e:a1").as_deref(),
            Some("5CF3708B2EA1")
        );
    }

    #[test]
    fn numeric_comparison_codes_parse_with_leading_zeroes() {
        assert_eq!(parse_passkey("012345"), Some(12_345));
        assert_eq!(parse_passkey("999999"), Some(999_999));
        assert_eq!(parse_passkey("1234567"), None);
        assert_eq!(parse_passkey("12a4"), None);
        assert_eq!(parse_passkey(""), None);
    }
}
