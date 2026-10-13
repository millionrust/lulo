//! Windows Core Audio backend.
//!
//! Endpoints come from the MMDevice API, levels and balance from each
//! endpoint's `IAudioEndpointVolume`, and changes from an
//! `IMMNotificationClient` plus volume callbacks on the default endpoints.
//! Windows has no documented call to change the default endpoint; the
//! `IPolicyConfig` interface used here is the one Windows' own Sound
//! settings use. Profiles and routes are PipeWire concepts with no Windows
//! counterpart, so they stay "not available".

use std::sync::{mpsc, OnceLock};

use windows::core::{BSTR, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eMultimedia, eRender, EDataFlow, ERole, IMMDevice,
    IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator,
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

use super::*;

/// Channel volumes are kept as integers scaled like the Linux backend's, so
/// [`Balance`] means the same thing on both.
const CHANNEL_SCALE: f32 = 1_000_000.0;
/// Below this loudest-channel level the balance cannot be told apart.
const MIN_ADJUSTABLE_VOLUME: u32 = 10_000;

pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
    let operation = "read audio devices";
    let enumerator = enumerator(operation)?;
    let default_output = default_endpoint(&enumerator, DeviceKind::Output);
    let default_input = default_endpoint(&enumerator, DeviceKind::Input);
    let output_id = default_output.as_ref().and_then(device_id);
    let input_id = default_input.as_ref().and_then(device_id);
    let outputs = devices(
        &enumerator,
        DeviceKind::Output,
        output_id.as_deref(),
        operation,
    )?;
    let inputs = devices(
        &enumerator,
        DeviceKind::Input,
        input_id.as_deref(),
        operation,
    )?;
    let output_level = default_output.as_ref().and_then(device_level);
    let input_level = default_input.as_ref().and_then(device_level);
    let has_output = outputs.iter().any(|device| device.is_default);
    let has_input = inputs.iter().any(|device| device.is_default);
    Ok(Snapshot {
        available: true,
        has_output,
        has_input,
        can_set_default: policy_config_available(),
        can_mute_input: has_input && input_level.is_some(),
        configuration_available: false,
        configuration_error: None,
        output: output_level.unwrap_or_default(),
        input: input_level.unwrap_or_default(),
        outputs,
        inputs,
        hardware_devices: Vec::new(),
        recording: recording_apps(),
    })
}

/// Apps using a microphone right now, as Windows' privacy settings track
/// them: an app's entry under the microphone consent store has a start
/// time and no stop time while it records (the same record behind the
/// taskbar's microphone indicator). Packaged apps are named by their
/// package, desktop apps by their program's file name.
fn recording_apps() -> Vec<String> {
    const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    let mut names = Vec::new();
    for (key, desktop) in [
        (STORE.to_owned(), false),
        (format!(r"{STORE}\NonPackaged"), true),
    ] {
        for app in registry::subkeys(&key) {
            if app == "NonPackaged" {
                continue;
            }
            let path = format!(r"{key}\{app}");
            let start = registry::qword(&path, "LastUsedTimeStart").unwrap_or(0);
            let stop = registry::qword(&path, "LastUsedTimeStop").unwrap_or(0);
            if start == 0 || stop != 0 {
                continue;
            }
            let name = if desktop {
                // `C:#Program Files#App#app.exe`: the program's name.
                app.rsplit('#')
                    .next()
                    .map(|file| file.trim_end_matches(".exe").to_owned())
                    .unwrap_or_else(|| app.clone())
            } else {
                // `Publisher.App_hash`: the package's name.
                app.split('_').next().unwrap_or(&app).to_owned()
            };
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

/// The few per-user registry reads `recording_apps` needs.
mod registry {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        RRF_RT_REG_QWORD,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// The names of `path`'s subkeys under HKEY_CURRENT_USER.
    pub(super) fn subkeys(path: &str) -> Vec<String> {
        let path = wide(path);
        let mut key = HKEY::default();
        // SAFETY: `path` is NUL-terminated; the key is closed below.
        if unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                KEY_READ,
                &mut key,
            )
        }
        .is_err()
        {
            return Vec::new();
        }
        let mut names = Vec::new();
        for index in 0..1024u32 {
            let mut name = [0u16; 512];
            let mut length = name.len() as u32;
            // SAFETY: `name` holds `length` characters.
            let status = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut length,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status.is_err() {
                break;
            }
            names.push(String::from_utf16_lossy(&name[..length as usize]));
        }
        // SAFETY: opened above.
        let _ = unsafe { RegCloseKey(key) };
        names
    }

    /// A REG_QWORD value of `path` under HKEY_CURRENT_USER.
    pub(super) fn qword(path: &str, value: &str) -> Option<u64> {
        let path = wide(path);
        let value = wide(value);
        let mut data = 0u64;
        let mut size = std::mem::size_of::<u64>() as u32;
        // SAFETY: both names are NUL-terminated; `data` holds `size` bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_QWORD,
                None,
                Some(&mut data as *mut u64 as *mut core::ffi::c_void),
                Some(&mut size),
            )
        };
        status.is_ok().then_some(data)
    }
}

pub(super) fn system_default_device(kind: DeviceKind) -> Result<DefaultDevice, Error> {
    let operation = match kind {
        DeviceKind::Output => "read default output device",
        DeviceKind::Input => "read default input device",
    };
    let enumerator = enumerator(operation)?;
    let device = default_endpoint(&enumerator, kind)
        .ok_or_else(|| Error::new(operation, "the system has no default audio device"))?;
    let volume = endpoint_volume(&device).map_err(|error| win_error(operation, error))?;
    Ok(DefaultDevice {
        name: friendly_name(&device),
        level: read_level(&volume).map_err(|error| win_error(operation, error))?,
    })
}

pub(super) fn system_set_volume(kind: DeviceKind, volume: u8) -> Result<(), Error> {
    let operation = "change audio volume";
    let endpoint = default_volume(kind, operation)?;
    // SAFETY: a COM call on a live endpoint; a null event context is allowed.
    unsafe { endpoint.SetMasterVolumeLevelScalar(f32::from(volume) / 100.0, std::ptr::null()) }
        .map_err(|error| win_error(operation, error))
}

pub(super) fn system_set_muted(kind: DeviceKind, muted: bool) -> Result<(), Error> {
    let operation = match kind {
        DeviceKind::Output => "change output mute",
        DeviceKind::Input => "change input mute",
    };
    let endpoint = default_volume(kind, operation)?;
    // SAFETY: a COM call on a live endpoint; a null event context is allowed.
    unsafe { endpoint.SetMute(muted, std::ptr::null()) }
        .map_err(|error| win_error(operation, error))
}

pub(super) fn system_set_default_device(
    kind: DeviceKind,
    device: &Device,
) -> Result<Snapshot, Error> {
    let operation = "change default audio device";
    let enumerator = enumerator(operation)?;
    let connected = active_endpoints(&enumerator, kind, operation)?
        .iter()
        .any(|(id, _)| *id == device.id);
    if !connected {
        return Err(Error::new(
            operation,
            "the selected device is no longer connected",
        ));
    }
    let policy = policy::PolicyConfig::new().map_err(|error| win_error(operation, error))?;
    let id = wide(&device.id);
    // Windows keeps a default per role; Sound settings sets all three.
    for role in [eConsole, eMultimedia, eCommunications] {
        policy
            .set_default_endpoint(&id, role)
            .map_err(|error| win_error(operation, error))?;
    }
    let snapshot = system_snapshot()?;
    let devices = match kind {
        DeviceKind::Output => &snapshot.outputs,
        DeviceKind::Input => &snapshot.inputs,
    };
    if devices
        .iter()
        .any(|candidate| candidate.is_default && candidate.id == device.id)
    {
        Ok(snapshot)
    } else {
        Err(Error::new(
            operation,
            "Windows did not make the selected device the default",
        ))
    }
}

pub(super) fn system_set_profile(_: &HardwareDevice, _: &Profile) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change audio profile",
        "Windows has no audio profiles",
    ))
}

pub(super) fn system_set_route(_: DeviceKind, _: &Device, _: &Route) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change audio route",
        "Windows has no audio routes",
    ))
}

pub(super) fn system_set_balance(device: &Device, value: i8) -> Result<Snapshot, Error> {
    let operation = "change output balance";
    if device.balance.is_none() {
        return Err(Error::new(
            operation,
            "the selected device does not have two adjustable channels",
        ));
    }
    let enumerator = enumerator(operation)?;
    let id = wide(&device.id);
    // SAFETY: `id` is a NUL-terminated UTF-16 string that outlives the call.
    let endpoint = unsafe { enumerator.GetDevice(PCWSTR(id.as_ptr())) }
        .map_err(|error| win_error(operation, error))?;
    let volume = endpoint_volume(&endpoint).map_err(|error| win_error(operation, error))?;
    let current = read_balance(&volume).ok_or_else(|| {
        Error::new(
            operation,
            "the selected device no longer has two adjustable channels",
        )
    })?;
    if current.value != value {
        let (left, right) = channel_targets(current.left_volume, current.right_volume, value);
        // SAFETY: COM calls on a live endpoint with two channels (0: left,
        // 1: right in Windows' stereo order); a null event context is allowed.
        unsafe { volume.SetChannelVolumeLevelScalar(0, unscaled(left), std::ptr::null()) }
            .map_err(|error| win_error(operation, error))?;
        unsafe { volume.SetChannelVolumeLevelScalar(1, unscaled(right), std::ptr::null()) }
            .map_err(|error| win_error(operation, error))?;
    }
    system_snapshot()
}

/// Wake-ups for the watch thread.
enum Wake {
    /// A default endpoint changed: move the volume callbacks to the new one.
    Rewire,
    Stop,
}

pub(super) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let operation = "watch audio changes";
    let (wake, woken) = mpsc::channel();
    let (ready, started) = async_channel::bounded(1);
    let events = sender.clone();
    let rewire = wake.clone();
    // The COM objects are not `Send`; they live on this thread, which only
    // sleeps in `recv` until Windows reports a change or the watch ends.
    std::thread::Builder::new()
        .name("lulo-audio-watch".into())
        .spawn(move || match Watching::start(events, rewire) {
            Ok(mut watching) => {
                let _ = ready.send_blocking(Ok(()));
                while let Ok(Wake::Rewire) = woken.recv() {
                    watching.rewire();
                }
                watching.stop();
            }
            Err(error) => {
                let _ = ready.send_blocking(Err(error));
            }
        })
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let started = started
        .recv()
        .await
        .unwrap_or_else(|_| Err(Error::new(operation, "the watch thread stopped")));
    if let Err(error) = started {
        let _ = sender.send(WatchEvent::Unavailable).await;
        return Err(error);
    }
    sender.closed().await;
    let _ = wake.send(Wake::Stop);
    Ok(())
}

/// The registered callbacks, owned by the watch thread.
struct Watching {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
    levels: Vec<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)>,
    events: async_channel::Sender<WatchEvent>,
}

impl Watching {
    fn start(
        events: async_channel::Sender<WatchEvent>,
        wake: mpsc::Sender<Wake>,
    ) -> Result<Self, Error> {
        let operation = "watch audio changes";
        let enumerator = enumerator(operation)?;
        let client: IMMNotificationClient = EndpointWatcher {
            events: events.clone(),
            wake,
        }
        .into();
        // SAFETY: the client stays alive in `Watching` until it is
        // unregistered in `stop`.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }
            .map_err(|error| win_error(operation, error))?;
        let mut watching = Self {
            enumerator,
            client,
            levels: Vec::new(),
            events,
        };
        watching.rewire();
        Ok(watching)
    }

    /// Watch the volume and mute of the current default output and input.
    fn rewire(&mut self) {
        self.release_levels();
        for kind in [DeviceKind::Output, DeviceKind::Input] {
            let Some(device) = default_endpoint(&self.enumerator, kind) else {
                continue;
            };
            let Ok(volume) = endpoint_volume(&device) else {
                continue;
            };
            let callback: IAudioEndpointVolumeCallback = LevelWatcher {
                events: self.events.clone(),
            }
            .into();
            // SAFETY: both objects are kept in `levels` until unregistered.
            if unsafe { volume.RegisterControlChangeNotify(&callback) }.is_ok() {
                self.levels.push((volume, callback));
            }
        }
    }

    fn release_levels(&mut self) {
        for (volume, callback) in self.levels.drain(..) {
            // SAFETY: unregisters a callback registered on this endpoint.
            let _ = unsafe { volume.UnregisterControlChangeNotify(&callback) };
        }
    }

    fn stop(mut self) {
        self.release_levels();
        // SAFETY: unregisters the client registered in `start`.
        let _ = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.client)
        };
    }
}

/// Endpoint arrivals, removals, state and default changes. Windows calls it
/// on its own threads, so it only queues work.
#[windows_core::implement(IMMNotificationClient)]
struct EndpointWatcher {
    events: async_channel::Sender<WatchEvent>,
    wake: mpsc::Sender<Wake>,
}

impl EndpointWatcher {
    fn changed(&self) {
        let _ = self.events.try_send(WatchEvent::Changed);
    }
}

impl IMMNotificationClient_Impl for EndpointWatcher_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows_core::Result<()> {
        self.changed();
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows_core::Result<()> {
        self.changed();
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows_core::Result<()> {
        self.changed();
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _: EDataFlow,
        role: ERole,
        _: &PCWSTR,
    ) -> windows_core::Result<()> {
        // The snapshot reads the console role's defaults.
        if role == eConsole {
            let _ = self.wake.send(Wake::Rewire);
            self.changed();
        }
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, key: &PROPERTYKEY) -> windows_core::Result<()> {
        // Endpoint properties change constantly; only the name is shown.
        if *key == PKEY_Device_FriendlyName {
            self.changed();
        }
        Ok(())
    }
}

/// Volume and mute changes on a default endpoint, from any source (volume
/// keys, the tray flyout, other apps).
#[windows_core::implement(IAudioEndpointVolumeCallback)]
struct LevelWatcher {
    events: async_channel::Sender<WatchEvent>,
}

impl IAudioEndpointVolumeCallback_Impl for LevelWatcher_Impl {
    fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows_core::Result<()> {
        let _ = self.events.try_send(WatchEvent::Changed);
        Ok(())
    }
}

fn enumerator(operation: &'static str) -> Result<IMMDeviceEnumerator, Error> {
    // SAFETY: joins (or keeps) this thread's COM apartment; an existing
    // apartment of another kind still allows the calls below.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(
            &MMDeviceEnumerator,
            None::<&windows::core::IUnknown>,
            CLSCTX_ALL,
        )
    }
    .map_err(|error| win_error(operation, error))
}

fn flow(kind: DeviceKind) -> EDataFlow {
    match kind {
        DeviceKind::Output => eRender,
        DeviceKind::Input => eCapture,
    }
}

/// The console-role default endpoint, or `None` when there is none.
fn default_endpoint(enumerator: &IMMDeviceEnumerator, kind: DeviceKind) -> Option<IMMDevice> {
    // SAFETY: a COM call on a live enumerator.
    unsafe { enumerator.GetDefaultAudioEndpoint(flow(kind), eConsole) }.ok()
}

fn default_volume(
    kind: DeviceKind,
    operation: &'static str,
) -> Result<IAudioEndpointVolume, Error> {
    let enumerator = enumerator(operation)?;
    let device = default_endpoint(&enumerator, kind)
        .ok_or_else(|| Error::new(operation, "the system has no default audio device"))?;
    endpoint_volume(&device).map_err(|error| win_error(operation, error))
}

fn active_endpoints(
    enumerator: &IMMDeviceEnumerator,
    kind: DeviceKind,
    operation: &'static str,
) -> Result<Vec<(String, IMMDevice)>, Error> {
    // SAFETY: COM calls on a live enumerator and the collection it returns.
    unsafe {
        let collection = enumerator
            .EnumAudioEndpoints(flow(kind), DEVICE_STATE_ACTIVE)
            .map_err(|error| win_error(operation, error))?;
        let count = collection
            .GetCount()
            .map_err(|error| win_error(operation, error))?;
        Ok((0..count)
            .filter_map(|index| collection.Item(index).ok())
            .filter_map(|device| Some((device_id(&device)?, device)))
            .collect())
    }
}

fn devices(
    enumerator: &IMMDeviceEnumerator,
    kind: DeviceKind,
    default_id: Option<&str>,
    operation: &'static str,
) -> Result<Vec<Device>, Error> {
    let mut devices: Vec<Device> = active_endpoints(enumerator, kind, operation)?
        .into_iter()
        .map(|(id, device)| {
            let balance = match kind {
                DeviceKind::Output => endpoint_volume(&device)
                    .ok()
                    .and_then(|volume| read_balance(&volume)),
                DeviceKind::Input => None,
            };
            Device {
                is_default: default_id == Some(id.as_str()),
                name: friendly_name(&device),
                routes: Vec::new(),
                balance,
                authority_name: id.clone(),
                authority_device_id: None,
                authority_route_device: None,
                id,
            }
        })
        .collect();
    sort_devices(&mut devices);
    Ok(devices)
}

/// The endpoint's stable id (`{0.0.0.00000000}.{guid}`).
fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: a COM call on a live device; the string it returns is ours.
    let id = take_string(unsafe { device.GetId() }.ok()?);
    (!id.is_empty()).then_some(id)
}

fn friendly_name(device: &IMMDevice) -> String {
    // SAFETY: COM calls on a live device and its read-only property store.
    let name = unsafe {
        device
            .OpenPropertyStore(STGM_READ)
            .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
            .and_then(|value| BSTR::try_from(&value))
    }
    .map(|name| name.to_string())
    .unwrap_or_default();
    let name = name.trim();
    if name.is_empty() {
        "Audio Device".to_string()
    } else {
        name.to_string()
    }
}

/// Read a COM-allocated string and free it.
fn take_string(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    // SAFETY: `value` is a NUL-terminated string COM allocated for the
    // caller; it is read once and freed here.
    unsafe {
        let text = value.to_string().unwrap_or_default();
        CoTaskMemFree(Some(value.0 as *const core::ffi::c_void));
        text
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn endpoint_volume(device: &IMMDevice) -> windows::core::Result<IAudioEndpointVolume> {
    // SAFETY: a COM call on a live device.
    unsafe { device.Activate(CLSCTX_ALL, None) }
}

fn device_level(device: &IMMDevice) -> Option<Level> {
    read_level(&endpoint_volume(device).ok()?).ok()
}

fn read_level(volume: &IAudioEndpointVolume) -> windows::core::Result<Level> {
    // SAFETY: COM calls on a live endpoint.
    unsafe {
        Ok(Level {
            volume: percent(volume.GetMasterVolumeLevelScalar()?),
            muted: volume.GetMute()?.as_bool(),
        })
    }
}

fn read_balance(volume: &IAudioEndpointVolume) -> Option<Balance> {
    // SAFETY: COM calls on a live endpoint, for channels it reports.
    unsafe {
        if volume.GetChannelCount().ok()? != 2 {
            return None;
        }
        let left = scaled(volume.GetChannelVolumeLevelScalar(0).ok()?)?;
        let right = scaled(volume.GetChannelVolumeLevelScalar(1).ok()?)?;
        Some(Balance {
            value: balance_value(left, right)?,
            left_volume: left,
            right_volume: right,
            left_first: true,
        })
    }
}

/// Whether `IPolicyConfig` can be created here (it is undocumented, so a
/// future Windows could drop it). Checked once per process.
fn policy_config_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| policy::PolicyConfig::new().is_ok())
}

fn win_error(operation: &'static str, error: windows::core::Error) -> Error {
    Error::new(operation, error.to_string())
}

/// A 0.0–1.0 endpoint scalar as a 0–100 percentage.
fn percent(scalar: f32) -> u8 {
    if scalar.is_finite() {
        (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
    } else {
        0
    }
}

fn scaled(scalar: f32) -> Option<u32> {
    (scalar.is_finite() && (0.0..=1.0).contains(&scalar))
        .then(|| (scalar * CHANNEL_SCALE).round() as u32)
}

fn unscaled(volume: u32) -> f32 {
    (volume as f32 / CHANNEL_SCALE).clamp(0.0, 1.0)
}

/// -100 (all left) to 100 (all right), relative to the louder channel.
fn balance_value(left: u32, right: u32) -> Option<i8> {
    let maximum = left.max(right);
    if maximum < MIN_ADJUSTABLE_VOLUME {
        return None;
    }
    let difference = i64::from(right) - i64::from(left);
    Some(
        ((difference * 100) as f64 / f64::from(maximum))
            .round()
            .clamp(-100.0, 100.0) as i8,
    )
}

/// The left and right channel volumes for `value`, keeping the louder
/// channel's level so the overall volume does not change.
fn channel_targets(left: u32, right: u32, value: i8) -> (u32, u32) {
    let maximum = left.max(right);
    let attenuated = |percent: u32| ((u64::from(maximum) * u64::from(percent) + 50) / 100) as u32;
    if value >= 0 {
        (attenuated(100 - u32::from(value.unsigned_abs())), maximum)
    } else {
        (maximum, attenuated(100 - u32::from(value.unsigned_abs())))
    }
}

/// `IPolicyConfig`, the undocumented interface behind Windows' Sound
/// settings. Only `SetDefaultEndpoint` is called; the earlier methods are
/// declared so its vtable slot is right.
mod policy {
    #![allow(dead_code, non_camel_case_types, non_snake_case)]

    use std::ffi::c_void;

    use windows::core::{GUID, HRESULT, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::ERole;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

    const CLSID_POLICY_CONFIG_CLIENT: GUID =
        GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

    #[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    unsafe trait IPolicyConfig: windows_core::IUnknown {
        fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(
            &self,
            device: PCWSTR,
            use_default: i32,
            format: *mut *mut c_void,
        ) -> HRESULT;
        fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(
            &self,
            device: PCWSTR,
            endpoint_format: *mut c_void,
            mix_format: *mut c_void,
        ) -> HRESULT;
        fn GetProcessingPeriod(
            &self,
            device: PCWSTR,
            use_default: i32,
            default_period: *mut i64,
            minimum_period: *mut i64,
        ) -> HRESULT;
        fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn GetPropertyValue(
            &self,
            device: PCWSTR,
            key: *const PROPERTYKEY,
            value: *mut c_void,
        ) -> HRESULT;
        fn SetPropertyValue(
            &self,
            device: PCWSTR,
            key: *const PROPERTYKEY,
            value: *mut c_void,
        ) -> HRESULT;
        fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
    }

    pub(super) struct PolicyConfig(IPolicyConfig);

    impl PolicyConfig {
        pub(super) fn new() -> windows::core::Result<Self> {
            // SAFETY: creates an in-box COM object on this thread's
            // apartment, which the caller has joined.
            unsafe {
                CoCreateInstance(
                    &CLSID_POLICY_CONFIG_CLIENT,
                    None::<&windows::core::IUnknown>,
                    CLSCTX_ALL,
                )
            }
            .map(Self)
        }

        /// `device` is a NUL-terminated UTF-16 endpoint id.
        pub(super) fn set_default_endpoint(
            &self,
            device: &[u16],
            role: ERole,
        ) -> windows::core::Result<()> {
            if device.last() != Some(&0) {
                return Err(windows::core::Error::from(
                    windows::Win32::Foundation::E_INVALIDARG,
                ));
            }
            // SAFETY: `device` is NUL-terminated and outlives the call.
            unsafe { self.0.SetDefaultEndpoint(PCWSTR(device.as_ptr()), role) }.ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_map_to_percentages() {
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.506), 51);
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(1.5), 100);
        assert_eq!(percent(f32::NAN), 0);
    }

    #[test]
    fn channel_scalars_round_trip() {
        assert_eq!(scaled(0.25), Some(250_000));
        assert_eq!(scaled(-0.1), None);
        assert_eq!(scaled(f32::INFINITY), None);
        assert!((unscaled(250_000) - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn balance_follows_the_louder_channel() {
        assert_eq!(balance_value(500_000, 500_000), Some(0));
        assert_eq!(balance_value(250_000, 500_000), Some(50));
        assert_eq!(balance_value(500_000, 0), Some(-100));
        assert_eq!(balance_value(5_000, 0), None);
    }

    #[test]
    fn balance_targets_keep_the_overall_level() {
        assert_eq!(channel_targets(500_000, 500_000, 50), (250_000, 500_000));
        assert_eq!(channel_targets(250_000, 500_000, -100), (500_000, 0));
        assert_eq!(channel_targets(400_000, 200_000, 0), (400_000, 400_000));
    }

    #[test]
    fn wide_strings_are_nul_terminated() {
        assert_eq!(wide("ab"), vec![u16::from(b'a'), u16::from(b'b'), 0]);
    }
}
