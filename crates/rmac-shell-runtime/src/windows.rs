//! The status items' sources on Windows (ADR 0023, "Phase 3 revised: shared
//! shell views"): Wi-Fi from the WLAN API, the battery from the power
//! status, sound from the default output's endpoint volume. Each is read
//! once, then again only when Windows reports a change (WLAN notifications,
//! power-setting notifications, the endpoint-volume callback); nothing
//! polls. The readings are handed to the same coordinator as on Lulo OS,
//! through [`WindowsServiceReader`], so the menu bar draws the same items
//! from the same snapshots.

use std::sync::{Mutex, OnceLock};

use async_channel::Sender;
use windows::core::w;
use windows::core::GUID;
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA,
};
use windows::Win32::NetworkManagement::WiFi::{
    wlan_interface_state_connected, wlan_intf_opcode_current_connection, WlanEnumInterfaces,
    WlanFreeMemory, WlanOpenHandle, WlanQueryInterface, WlanRegisterNotification,
    L2_NOTIFICATION_DATA, WLAN_API_VERSION_2_0, WLAN_CONNECTION_ATTRIBUTES,
    WLAN_INTERFACE_INFO_LIST, WLAN_NOTIFICATION_SOURCE_ACM, WLAN_NOTIFICATION_SOURCE_MSM,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerSettingRegisterNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
    SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    DEVICE_NOTIFY_CALLBACK, MSG, PBT_APMPOWERSTATUSCHANGE, WM_POWERBROADCAST, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::runtime::ServiceReader;
use crate::Error;

/// The power status's charge, battery flags and AC line: only compared,
/// so a change (plugged in, charging, charged, a percent) wakes the
/// coordinator, which reads the full state through `rmac_power`.
type Battery = (u8, u8, u8);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Wifi {
    ssid: Vec<u8>,
    /// Signal quality, 0 to 100.
    quality: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Readings {
    /// Whether the PC has a WLAN service at all.
    wlan: bool,
    wifi: Option<Wifi>,
    battery: Option<Battery>,
    /// The default output's volume (0 to 1) and mute.
    volume: Option<(f32, bool)>,
}

static READINGS: Mutex<Option<Readings>> = Mutex::new(None);
static CHANGES: Mutex<Vec<Sender<()>>> = Mutex::new(Vec::new());
static STARTED: OnceLock<()> = OnceLock::new();
static WLAN: Mutex<Option<usize>> = Mutex::new(None);

fn update(change: impl FnOnce(&mut Readings)) {
    let changed = {
        let Ok(mut readings) = READINGS.lock() else {
            return;
        };
        let readings = readings.get_or_insert_with(Readings::default);
        let before = readings.clone();
        change(readings);
        *readings != before
    };
    if changed {
        if let Ok(mut watchers) = CHANGES.lock() {
            watchers.retain(|watcher| !watcher.is_closed());
            for watcher in watchers.iter() {
                let _ = watcher.try_send(());
            }
        }
    }
}

fn readings() -> Readings {
    READINGS
        .lock()
        .ok()
        .and_then(|readings| readings.clone())
        .unwrap_or_default()
}

/// Start reading, once per process.
fn start() {
    STARTED.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("lulo-status".into())
            .spawn(|| {
                // SAFETY: once, before any COM call on this thread.
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                watch_battery();
                watch_wifi();
                // The volume callback and its endpoint must stay alive.
                let _volume = watch_volume();
                // Windows broadcasts every power status change (plugged in,
                // charging, charged) to top-level windows: this thread's
                // hidden one hears them and waits for messages, nothing else.
                watch_power_broadcasts();
                let mut message = MSG::default();
                // SAFETY: this thread's own message loop.
                while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
                    // SAFETY: a message GetMessageW just filled in.
                    unsafe { DispatchMessageW(&message) };
                }
            });
        if let Err(error) = spawned {
            eprintln!("the menu bar will show no status items: {error}");
        }
    });
}

/// Ask the coordinator to read the status sources again whenever Windows
/// reports a change, until `sender` closes.
pub(crate) async fn watch(sender: Sender<rmac_shell_status_linux::Event>) -> Result<(), Error> {
    let (changed, changes) = async_channel::bounded(1);
    if let Ok(mut watchers) = CHANGES.lock() {
        watchers.push(changed);
    }
    start();
    // Bluetooth comes from WinRT (`rmac_bluetooth`), with its own change
    // events (radio on/off, pairing, connections); reading it is heavier
    // than the other sources, so each kind of change refreshes only its own.
    let others = rmac_shell_status_linux::Sources {
        bluetooth: false,
        ..rmac_shell_status_linux::Sources::all()
    };
    let bluetooth_only = rmac_shell_status_linux::Sources {
        network: false,
        bluetooth: true,
        audio: false,
        power: false,
    };
    let (bluetooth_changed, bluetooth_changes) = async_channel::bounded(1);
    // Held here so the channel stays open if the watch ends (no adapter):
    // a closed channel would wake this loop for nothing.
    let _keep_open = bluetooth_changed.clone();
    let bluetooth = futures_util::FutureExt::fuse(rmac_bluetooth::watch(bluetooth_changed));
    futures_util::pin_mut!(bluetooth);
    let mut sources = rmac_shell_status_linux::Sources::all();
    loop {
        if sender
            .send(rmac_shell_status_linux::Event::Refresh(sources))
            .await
            .is_err()
        {
            return Ok(());
        }
        let next = futures_util::FutureExt::fuse(changes.recv());
        let next_bluetooth = futures_util::FutureExt::fuse(bluetooth_changes.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(next, next_bluetooth, closed);
        loop {
            futures_util::select! {
                next = next => {
                    if next.is_err() {
                        return Ok(());
                    }
                    sources = others;
                    break;
                },
                _ = next_bluetooth => {
                    sources = bluetooth_only;
                    break;
                },
                // The Bluetooth watch ended (no adapter, or WinRT refused
                // it); the other sources carry on.
                _ = bluetooth => {},
                _ = closed => return Ok(()),
            }
        }
    }
}

/// The status snapshots from the latest Windows readings.
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsServiceReader;

impl ServiceReader for WindowsServiceReader {
    fn network(
        &self,
    ) -> Result<
        (
            rmac_network::NetworkSnapshot,
            rmac_network::WifiSnapshot,
            rmac_network::VpnSnapshot,
        ),
        String,
    > {
        let readings = readings();
        let ssid = readings
            .wifi
            .as_ref()
            .map(|wifi| String::from_utf8_lossy(&wifi.ssid).into_owned());
        let networks = readings
            .wifi
            .iter()
            .filter_map(|wifi| {
                let security = rmac_network::WifiSecurity::Protected;
                Some(rmac_network::WifiNetwork {
                    id: rmac_network::WifiNetworkId::from_bytes(wifi.ssid.clone(), security)?,
                    ssid: String::from_utf8_lossy(&wifi.ssid).into_owned(),
                    strength: wifi.quality.min(100),
                    security,
                    known: true,
                    connected: true,
                })
            })
            .collect();
        Ok((
            rmac_network::NetworkSnapshot::default(),
            rmac_network::WifiSnapshot {
                available: readings.wlan,
                enabled: readings.wlan,
                interface: None,
                current_ssid: ssid,
                networks,
                saved_networks: Vec::new(),
            },
            rmac_network::VpnSnapshot::default(),
        ))
    }

    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
        rmac_bluetooth::snapshot().map_err(|error| error.to_string())
    }

    fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
        let Some((level, muted)) = readings().volume else {
            return Ok(rmac_audio::Snapshot::default());
        };
        Ok(rmac_audio::Snapshot {
            available: true,
            has_output: true,
            output: rmac_audio::Level {
                volume: (level.clamp(0.0, 1.0) * 100.0).round() as u8,
                muted,
            },
            ..rmac_audio::Snapshot::default()
        })
    }

    /// The same reading as the Battery menu and System Settings
    /// (`rmac_power`: the power status and the battery driver's charge
    /// flags), taken when Windows reports a change.
    fn power(&self) -> Result<rmac_power::Snapshot, String> {
        rmac_power::snapshot().map_err(|error| error.to_string())
    }
}

fn read_battery() -> Option<Battery> {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: plain out-parameter.
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    // 128: no system battery; 255: unknown.
    if status.BatteryFlag & 128 != 0 || status.BatteryFlag == 255 || status.BatteryLifePercent > 100
    {
        return None;
    }
    Some((
        status.BatteryLifePercent,
        status.BatteryFlag,
        status.ACLineStatus,
    ))
}

unsafe extern "system" fn on_power_broadcast(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_POWERBROADCAST && wparam.0 == PBT_APMPOWERSTATUSCHANGE as usize {
        let battery = read_battery();
        update(|readings| readings.battery = battery);
        return LRESULT(1);
    }
    // SAFETY: the default handling of a message to this window.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// A hidden top-level window for `WM_POWERBROADCAST`, which Windows sends
/// only to top-level windows; it is never shown, so no window list sees it.
fn watch_power_broadcasts() {
    let class = w!("LuloPowerWatch");
    let description = WNDCLASSW {
        lpfnWndProc: Some(on_power_broadcast),
        lpszClassName: class,
        ..Default::default()
    };
    // SAFETY: registers a class with a static name and a valid procedure,
    // then creates one hidden window of it on this thread.
    unsafe {
        RegisterClassW(&description);
        let _ = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        );
    }
}

unsafe extern "system" fn on_power(
    _context: *const core::ffi::c_void,
    _kind: u32,
    _setting: *const core::ffi::c_void,
) -> u32 {
    let battery = read_battery();
    update(|readings| readings.battery = battery);
    0
}

fn watch_battery() {
    let battery = read_battery();
    update(|readings| readings.battery = battery);
    if battery.is_none() {
        return;
    }
    // Windows keeps a pointer to the parameters for as long as the
    // registration lives, which is the life of the process.
    let parameters: &'static DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS =
        Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_power),
            Context: std::ptr::null_mut(),
        }));
    for setting in [GUID_BATTERY_PERCENTAGE_REMAINING, GUID_ACDC_POWER_SOURCE] {
        let mut registration: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: the parameters are 'static; the registration is kept.
        let _: WIN32_ERROR = unsafe {
            PowerSettingRegisterNotification(
                &setting as *const GUID,
                DEVICE_NOTIFY_CALLBACK,
                HANDLE(
                    parameters as *const DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS
                        as *mut core::ffi::c_void,
                ),
                &mut registration,
            )
        };
    }
}

fn read_wifi(client: HANDLE) -> Option<Wifi> {
    // SAFETY: the WLAN API allocates the lists it returns; each is freed
    // with WlanFreeMemory after it is read.
    unsafe {
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(client, None, &mut list) != 0 || list.is_null() {
            return None;
        }
        let count = (*list).dwNumberOfItems as usize;
        let interfaces = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), count);
        let mut found = None;
        for interface in interfaces {
            if interface.isState != wlan_interface_state_connected {
                continue;
            }
            let mut size = 0u32;
            let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
            if WlanQueryInterface(
                client,
                &interface.InterfaceGuid,
                wlan_intf_opcode_current_connection,
                None,
                &mut size,
                &mut data,
                None,
            ) != 0
                || data.is_null()
            {
                continue;
            }
            let connection = &*(data as *const WLAN_CONNECTION_ATTRIBUTES);
            let association = &connection.wlanAssociationAttributes;
            let ssid = &association.dot11Ssid;
            let length = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
            found = Some(Wifi {
                ssid: ssid.ucSSID[..length].to_vec(),
                quality: association.wlanSignalQuality.min(100) as u8,
            });
            WlanFreeMemory(data);
            break;
        }
        WlanFreeMemory(list as *const core::ffi::c_void);
        found
    }
}

fn report_wifi(client: HANDLE) {
    let wifi = read_wifi(client);
    // Signal-quality notifications repeat; the coordinator quantises the
    // signal to the bar's three bars, so only a visible change wakes it.
    update(|readings| {
        readings.wlan = true;
        readings.wifi = wifi;
    });
}

unsafe extern "system" fn on_wlan(
    _data: *mut L2_NOTIFICATION_DATA,
    _context: *mut core::ffi::c_void,
) {
    if let Some(client) = WLAN.lock().ok().and_then(|client| *client) {
        report_wifi(HANDLE(client as *mut core::ffi::c_void));
    }
}

fn watch_wifi() {
    let mut negotiated = 0u32;
    let mut client = HANDLE::default();
    // SAFETY: plain out-parameters; the handle is kept for the process.
    if unsafe { WlanOpenHandle(WLAN_API_VERSION_2_0, None, &mut negotiated, &mut client) } != 0 {
        // No WLAN service (a desktop or a server): no Wi-Fi item.
        update(|readings| readings.wlan = false);
        return;
    }
    if let Ok(mut slot) = WLAN.lock() {
        *slot = Some(client.0 as usize);
    }
    report_wifi(client);
    // SAFETY: the callback reads the handle kept above.
    unsafe {
        WlanRegisterNotification(
            client,
            WLAN_NOTIFICATION_SOURCE_ACM | WLAN_NOTIFICATION_SOURCE_MSM,
            true,
            Some(on_wlan),
            None,
            None,
            None,
        );
    }
}

#[windows_core::implement(IAudioEndpointVolumeCallback)]
struct VolumeWatcher;

impl IAudioEndpointVolumeCallback_Impl for VolumeWatcher_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows_core::Result<()> {
        // SAFETY: Windows passes valid notification data for the call.
        if let Some(data) = unsafe { data.as_ref() } {
            let volume = Some((data.fMasterVolume, data.bMuted.as_bool()));
            update(|readings| readings.volume = volume);
        }
        Ok(())
    }
}

fn open_volume() -> windows_core::Result<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)> {
    // SAFETY: COM calls on this MTA thread, on objects it owns.
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(
            &MMDeviceEnumerator,
            None::<&windows_core::IUnknown>,
            CLSCTX_ALL,
        )?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        let volume = Some((
            endpoint.GetMasterVolumeLevelScalar()?,
            endpoint.GetMute()?.as_bool(),
        ));
        update(|readings| readings.volume = volume);
        let callback: IAudioEndpointVolumeCallback = VolumeWatcher.into();
        endpoint.RegisterControlChangeNotify(&callback)?;
        Ok((endpoint, callback))
    }
}

/// The default output's volume, kept current by its callback. Returns
/// what must stay alive for the callback to keep arriving.
fn watch_volume() -> Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)> {
    match open_volume() {
        Ok(watched) => Some(watched),
        Err(_) => {
            // No audio device (a server, or none plugged in): no item.
            update(|readings| readings.volume = None);
            None
        }
    }
}
