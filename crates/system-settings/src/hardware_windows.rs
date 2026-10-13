//! The hardware scan on Windows: the same [`Capabilities`] the Linux sysfs
//! scan fills, from Windows' own device APIs, so a PC without a battery,
//! Bluetooth radio, Wi-Fi adapter, touchpad, mouse or backlight gets no
//! pane or control for it. Called on a blocking-pool thread, never the UI
//! thread.

use windows::core::{w, PWSTR};
use windows::Win32::Devices::Bluetooth::{
    BluetoothFindFirstRadio, BluetoothFindRadioClose, BLUETOOTH_FIND_RADIO_PARAMS,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::NetworkManagement::WiFi::{
    WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle, WLAN_API_VERSION_2_0,
    WLAN_INTERFACE_INFO_LIST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::UI::Input::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO,
    RIDI_DEVICENAME, RID_DEVICE_INFO, RIM_TYPEHID, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, MSG,
    SM_DIGITIZER, WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_DEVICECHANGE, WM_DISPLAYCHANGE,
    WNDCLASSW, WS_OVERLAPPED,
};

use super::Capabilities;

/// HID usage page and usage of a precision touchpad's collection.
const DIGITIZER_PAGE: u16 = 0x0d;
const TOUCH_PAD: u16 = 0x05;
const TOUCH_SCREEN: u16 = 0x04;
/// `SM_DIGITIZER` bits: an integrated touch screen and pen.
const NID_INTEGRATED_TOUCH: i32 = 0x01;
const NID_INTEGRATED_PEN: i32 = 0x04;

pub(crate) fn current() -> Capabilities {
    let mut result = Capabilities {
        has_battery: has_battery(),
        has_wifi: has_wifi(),
        has_bluetooth: has_bluetooth(),
        // The built-in panel's brightness, as Windows' own slider reads it
        // (WMI); external monitors have none, as on Lulo OS.
        has_backlight: rmac_osd::brightness().is_ok(),
        ..Capabilities::default()
    };
    let pointers = pointing_devices();
    result.has_touchpad = pointers.touchpads > 0;
    // A precision touchpad also reports a mouse collection, so only mice
    // beyond the touchpads are a mouse someone plugged in.
    result.has_external_mouse = pointers.mice > pointers.touchpads;
    // SAFETY: a plain metrics query.
    let digitizer = unsafe { GetSystemMetrics(SM_DIGITIZER) };
    result.has_touchscreen = digitizer & NID_INTEGRATED_TOUCH != 0 || pointers.touchscreens > 0;
    result.has_pen = digitizer & NID_INTEGRATED_PEN != 0;
    result
}

fn has_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: plain out-parameter.
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return false;
    }
    // 128: no system battery; 255: unknown.
    status.BatteryFlag & 128 == 0 && status.BatteryFlag != 255
}

fn has_wifi() -> bool {
    let mut negotiated = 0u32;
    let mut client = HANDLE::default();
    // SAFETY: plain out-parameters; the handle is closed below.
    if unsafe { WlanOpenHandle(WLAN_API_VERSION_2_0, None, &mut negotiated, &mut client) } != 0 {
        // No WLAN service: a desktop without Wi-Fi.
        return false;
    }
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    // SAFETY: the API allocates `list`, freed below.
    let found = unsafe {
        let ok = WlanEnumInterfaces(client, None, &mut list) == 0 && !list.is_null();
        let found = ok && (*list).dwNumberOfItems > 0;
        if !list.is_null() {
            WlanFreeMemory(list as *const core::ffi::c_void);
        }
        found
    };
    // SAFETY: opened above.
    unsafe { WlanCloseHandle(client, None) };
    found
}

fn has_bluetooth() -> bool {
    let parameters = BLUETOOTH_FIND_RADIO_PARAMS {
        dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
    };
    let mut radio = HANDLE::default();
    // SAFETY: plain parameters; the find handle and radio are closed below.
    match unsafe { BluetoothFindFirstRadio(&parameters, &mut radio) } {
        Ok(find) => {
            // SAFETY: both handles came from the call above.
            unsafe {
                let _ = CloseHandle(radio);
                let _ = BluetoothFindRadioClose(find);
            }
            true
        }
        Err(_) => false,
    }
}

#[derive(Default)]
struct Pointers {
    mice: usize,
    touchpads: usize,
    touchscreens: usize,
}

/// Count the pointing devices raw input lists. Remote Desktop's virtual
/// mouse (`RDP_MOU`) is not a device someone plugged in.
fn pointing_devices() -> Pointers {
    let mut pointers = Pointers::default();
    let entry = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut count = 0u32;
    // SAFETY: the first call only asks for the count.
    if unsafe { GetRawInputDeviceList(None, &mut count, entry) } == u32::MAX || count == 0 {
        return pointers;
    }
    let mut devices = vec![RAWINPUTDEVICELIST::default(); count as usize];
    // SAFETY: `devices` holds `count` entries.
    let read = unsafe { GetRawInputDeviceList(Some(devices.as_mut_ptr()), &mut count, entry) };
    if read == u32::MAX {
        return pointers;
    }
    devices.truncate(read as usize);
    for device in devices {
        if device.dwType == RIM_TYPEMOUSE {
            if !device_name(device.hDevice)
                .to_ascii_uppercase()
                .contains("RDP_MOU")
            {
                pointers.mice += 1;
            }
        } else if device.dwType == RIM_TYPEHID {
            let mut info = RID_DEVICE_INFO {
                cbSize: std::mem::size_of::<RID_DEVICE_INFO>() as u32,
                ..Default::default()
            };
            let mut size = info.cbSize;
            // SAFETY: `info` is a RID_DEVICE_INFO of `size` bytes.
            let written = unsafe {
                GetRawInputDeviceInfoW(
                    Some(device.hDevice),
                    RIDI_DEVICEINFO,
                    Some(&mut info as *mut RID_DEVICE_INFO as *mut core::ffi::c_void),
                    &mut size,
                )
            };
            if written == u32::MAX || info.dwType != RIM_TYPEHID {
                continue;
            }
            // SAFETY: a HID device fills the `hid` arm.
            let hid = unsafe { info.Anonymous.hid };
            if hid.usUsagePage == DIGITIZER_PAGE && hid.usUsage == TOUCH_PAD {
                pointers.touchpads += 1;
            } else if hid.usUsagePage == DIGITIZER_PAGE && hid.usUsage == TOUCH_SCREEN {
                pointers.touchscreens += 1;
            }
        }
    }
    pointers
}

fn device_name(device: HANDLE) -> String {
    let mut length = 0u32;
    // SAFETY: the first call only asks for the length in characters.
    unsafe { GetRawInputDeviceInfoW(Some(device), RIDI_DEVICENAME, None, &mut length) };
    if length == 0 || length > 1024 {
        return String::new();
    }
    let mut name = vec![0u16; length as usize];
    // SAFETY: `name` holds `length` characters.
    let written = unsafe {
        GetRawInputDeviceInfoW(
            Some(device),
            RIDI_DEVICENAME,
            Some(PWSTR(name.as_mut_ptr()).0.cast()),
            &mut length,
        )
    };
    if written == u32::MAX {
        return String::new();
    }
    let end = name
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(name.len());
    String::from_utf16_lossy(&name[..end])
}

/// Wait for Windows to say devices or displays changed: it broadcasts
/// `WM_DEVICECHANGE` (`DBT_DEVNODES_CHANGED`) and `WM_DISPLAYCHANGE` to
/// top-level windows, so a hidden one on its own thread listens. Nothing
/// polls; the window closes when Settings drops the receiver.
pub(crate) async fn watch(sender: async_channel::Sender<()>) -> std::io::Result<()> {
    let (ready_tx, ready_rx) = async_channel::bounded::<Option<isize>>(1);
    let events = sender.clone();
    let thread = std::thread::Builder::new()
        .name("lulo-hardware-watch".into())
        .spawn(move || listen(events, ready_tx))?;
    let window = ready_rx.recv().await.ok().flatten();
    if let Some(window) = window {
        sender.closed().await;
        // SAFETY: posting to the window this module created; it may already
        // be gone, which the call reports and nothing else depends on.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(window as *mut core::ffi::c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
    let _ = blocking::unblock(move || thread.join()).await;
    if window.is_none() {
        return Err(std::io::Error::other("could not watch Windows' devices"));
    }
    Ok(())
}

thread_local! {
    static EVENTS: std::cell::RefCell<Option<async_channel::Sender<()>>> =
        const { std::cell::RefCell::new(None) };
}

fn listen(events: async_channel::Sender<()>, ready: async_channel::Sender<Option<isize>>) {
    EVENTS.with(|slot| *slot.borrow_mut() = Some(events));
    // SAFETY: a class and window owned by this thread, with a static name
    // and procedure; the message loop below runs until WM_QUIT.
    unsafe {
        let instance = GetModuleHandleW(None).ok().map(|module| module.into());
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.unwrap_or_default(),
            lpszClassName: w!("LuloSettingsHardwareWatch"),
            ..Default::default()
        };
        // Registering twice (a second watch) fails harmlessly: the class
        // already exists with the same procedure.
        RegisterClassW(&class);
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("LuloSettingsHardwareWatch"),
            w!(""),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            instance,
            None,
        );
        let Ok(window) = window else {
            let _ = ready.send_blocking(None);
            return;
        };
        let _ = ready.send_blocking(Some(window.0 as isize));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_DEVICECHANGE | WM_DISPLAYCHANGE => {
            EVENTS.with(|slot| {
                if let Some(events) = slot.borrow().as_ref() {
                    let _ = events.try_send(());
                }
            });
            LRESULT(1)
        }
        WM_CLOSE => {
            // SAFETY: this thread's own window.
            let _ = unsafe { DestroyWindow(window) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: the default handling for everything else.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
