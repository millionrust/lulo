//! The menu bar's status items, read-only for now: Wi-Fi, sound and
//! battery. Each is read once, then updated only by Windows' own change
//! notifications (WLAN notifications, the endpoint-volume callback and
//! power-setting notifications). A PC without the hardware hides the item.

use std::sync::{Mutex, OnceLock};

use windows::core::GUID;
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
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
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub level: f32,
    pub muted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wifi {
    pub network: String,
    /// One to three bars, as the menu bar draws them.
    pub bars: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusEvent {
    Battery(Option<Battery>),
    Volume(Option<Volume>),
    Wifi(Option<Wifi>),
}

static SENDER: OnceLock<async_channel::Sender<StatusEvent>> = OnceLock::new();
static LAST_WIFI: Mutex<Option<Option<Wifi>>> = Mutex::new(None);
static WLAN: Mutex<Option<usize>> = Mutex::new(None);

fn send(event: StatusEvent) {
    if let Some(sender) = SENDER.get() {
        let _ = sender.try_send(event);
    }
}

/// Read every item and keep them current; events arrive on `sender`.
pub fn start(sender: async_channel::Sender<StatusEvent>) {
    let _ = SENDER.set(sender);
    let spawned = std::thread::Builder::new()
        .name("lulo-status".into())
        .spawn(|| {
            // SAFETY: once, before any COM call on this thread.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            watch_battery();
            watch_wifi();
            // The volume callback and its endpoint must stay alive.
            let _volume = watch_volume();
            loop {
                std::thread::park();
            }
        });
    if let Err(error) = spawned {
        eprintln!("lulo-shell: the menu bar will show no status items: {error}");
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
    Some(Battery {
        percent: status.BatteryLifePercent,
        charging: status.ACLineStatus == 1,
    })
}

unsafe extern "system" fn on_power(
    _context: *const core::ffi::c_void,
    _kind: u32,
    _setting: *const core::ffi::c_void,
) -> u32 {
    send(StatusEvent::Battery(read_battery()));
    0
}

fn watch_battery() {
    let battery = read_battery();
    send(StatusEvent::Battery(battery));
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

fn bars(signal: u32) -> u8 {
    match signal {
        0..=33 => 1,
        34..=66 => 2,
        _ => 3,
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
                network: String::from_utf8_lossy(&ssid.ucSSID[..length]).into_owned(),
                bars: bars(association.wlanSignalQuality),
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
    let Ok(mut last) = LAST_WIFI.lock() else {
        return;
    };
    // Signal-quality notifications repeat; only a visible change is sent.
    if last.as_ref() != Some(&wifi) {
        *last = Some(wifi.clone());
        send(StatusEvent::Wifi(wifi));
    }
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
        send(StatusEvent::Wifi(None));
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
            send(StatusEvent::Volume(Some(Volume {
                level: data.fMasterVolume,
                muted: data.bMuted.as_bool(),
            })));
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
        send(StatusEvent::Volume(Some(Volume {
            level: endpoint.GetMasterVolumeLevelScalar()?,
            muted: endpoint.GetMute()?.as_bool(),
        })));
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
            send(StatusEvent::Volume(None));
            None
        }
    }
}
