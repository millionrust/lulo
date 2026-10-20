//! Windows input settings: the same model as niri, read from and written to
//! the user's real Windows settings.
//!
//! - Keyboard repeat, pointer speed and precision, wheel lines, double-click
//!   time and the primary button go through `SystemParametersInfoW` with
//!   `SPIF_UPDATEINIFILE | SPIF_SENDCHANGE`, so they persist and every app
//!   hears about them.
//! - Precision Touchpad settings go through `SPI_GET/SETTOUCHPADPARAMETERS`
//!   (Windows 11 24H2 and later), which applies them at once. Older Windows
//!   has no API for them, so there they are read from and written to the
//!   `HKCU\...\PrecisionTouchPad` values Windows' own Settings app uses, and
//!   the snapshot says they may need a new sign-in.
//! - The watch is one thread owning a hidden top-level window (top-level so
//!   it receives the `WM_SETTINGCHANGE` and `WM_DEVICECHANGE` broadcasts) plus
//!   a `RegNotifyChangeKeyValue` event on the touchpad key, all waited on by
//!   `MsgWaitForMultipleObjects`; nothing polls.
//!
//! Windows has no per-user equivalent for: mouse natural scrolling, middle
//! button emulation, scrolling with the middle button, turning the mouse or
//! touchpad off, Num Lock at sign-in, a separate touchpad left-handed or
//! acceleration choice, touchpad drag lock, pointing-stick settings (Windows
//! treats a pointing stick as a mouse), and touchscreen settings. Those are
//! marked unsupported in [`Supported`] and `save` refuses to change them.

use super::{
    AccelProfile, DeviceOverrideCapability, Error, InputSettings, KeyboardLayoutAuthority,
    KeyboardSettings, PointerSettings, SecondaryClick, Snapshot, Supported, TouchSettings,
    TouchpadSettings, WatchEvent, WHEEL_SCROLL_PAGE,
};
use async_channel::Sender;
use std::cell::RefCell;
use std::ffi::c_void;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, RegSetKeyValueW, HKEY,
    HKEY_CURRENT_USER, KEY_NOTIFY, REG_DWORD, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD,
};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentThreadId, INFINITE};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::Input::{
    GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO,
    RID_DEVICE_INFO, RIM_TYPEHID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics,
    MsgWaitForMultipleObjects, PeekMessageW, PostThreadMessageW, RegisterClassExW,
    SystemParametersInfoW, TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT, SM_SWAPBUTTON,
    SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED, SPI_GETMOUSE,
    SPI_GETMOUSESPEED, SPI_GETWHEELSCROLLLINES, SPI_SETDOUBLECLICKTIME, SPI_SETKEYBOARDDELAY,
    SPI_SETKEYBOARDSPEED, SPI_SETMOUSE, SPI_SETMOUSEBUTTONSWAP, SPI_SETMOUSESPEED,
    SPI_SETWHEELSCROLLLINES, SYSTEM_PARAMETERS_INFO_ACTION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    WM_DEVICECHANGE, WM_QUIT, WM_SETTINGCHANGE, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

const TOUCHPAD_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\PrecisionTouchPad");
const LISTENER_CLASS: PCWSTR = w!("LuloInputSettingsListener");

// Not in the `windows` crate's metadata yet; values from winuser.h
// (documented under SystemParametersInfo, Windows 11 24H2).
const SPI_GETTOUCHPADPARAMETERS: SYSTEM_PARAMETERS_INFO_ACTION =
    SYSTEM_PARAMETERS_INFO_ACTION(0x00AE);
const SPI_SETTOUCHPADPARAMETERS: SYSTEM_PARAMETERS_INFO_ACTION =
    SYSTEM_PARAMETERS_INFO_ACTION(0x00AF);
const TOUCHPAD_PARAMETERS_VERSION_1: u32 = 1;

/// `TOUCHPAD_SENSITIVITY_LEVEL_MOST_SENSITIVE`: no input suppression after
/// typing (the registry's `AAPThreshold` uses the same 0..4 scale).
const SENSITIVITY_MOST: u32 = 0;
/// `TOUCHPAD_SENSITIVITY_LEVEL_MEDIUM_SENSITIVITY`, Windows' default.
const SENSITIVITY_MEDIUM: u32 = 2;

/// Windows' own "Enhance pointer precision" thresholds and level, as the
/// Mouse control panel writes them when the box is ticked.
const ENHANCED_PRECISION: [i32; 3] = [6, 10, 1];
const NO_PRECISION: [i32; 3] = [0, 0, 0];

/// `TOUCHPAD_PARAMETERS_V1` from winuser.h. Each run of `BOOL x : 1`
/// bit-fields packs into one 32-bit unit, least significant bit first
/// (MSVC layout); the bit positions are the `STATUS_*` / `SETTING_*`
/// constants below. Windows fills every field; Lulo reads only some, and the
/// rest must still be present for the layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
struct TouchpadParameters {
    version_number: u32,
    max_supported_contacts: u32,
    legacy_touchpad_features: i32,
    status: u32,
    settings: u32,
    sensitivity_level: u32,
    cursor_speed: u32,
    feedback_intensity: u32,
    click_force_sensitivity: u32,
    right_click_zone_width: u32,
    right_click_zone_height: u32,
}

const STATUS_TOUCHPAD_PRESENT: u32 = 1 << 0;
const STATUS_TOUCHPAD_ENABLED: u32 = 1 << 3;
const SETTING_TAP: u32 = 1 << 2;
const SETTING_TWO_FINGER_TAP: u32 = 1 << 4;
const SETTING_RIGHT_CLICK_ZONE: u32 = 1 << 5;
const SETTING_MOUSE_ACCEL_HONORED: u32 = 1 << 6;
const SETTING_SCROLL_REVERSED: u32 = 1 << 9;

/// The touchpad values Lulo maps, whichever way Windows exposes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TouchpadValues {
    enabled: bool,
    /// Windows' default: content follows the fingers. Stored by Windows as
    /// "scroll direction reversed" = false (`ScrollDirection` = 0).
    natural_scroll: bool,
    cursor_speed: u32,
    tap: bool,
    two_finger_tap: bool,
    right_click_zone: bool,
    sensitivity: u32,
    honors_mouse_accel: bool,
}

impl TouchpadValues {
    fn from_parameters(parameters: &TouchpadParameters) -> Self {
        let setting = |bit: u32| parameters.settings & bit != 0;
        Self {
            enabled: parameters.status & STATUS_TOUCHPAD_ENABLED != 0,
            natural_scroll: !setting(SETTING_SCROLL_REVERSED),
            cursor_speed: parameters.cursor_speed,
            tap: setting(SETTING_TAP),
            two_finger_tap: setting(SETTING_TWO_FINGER_TAP),
            right_click_zone: setting(SETTING_RIGHT_CLICK_ZONE),
            sensitivity: parameters.sensitivity_level,
            honors_mouse_accel: setting(SETTING_MOUSE_ACCEL_HONORED),
        }
    }

    fn write_to(self, parameters: &mut TouchpadParameters) {
        let mut put_bit = |bit: u32, on: bool| {
            if on {
                parameters.settings |= bit;
            } else {
                parameters.settings &= !bit;
            }
        };
        put_bit(SETTING_SCROLL_REVERSED, !self.natural_scroll);
        put_bit(SETTING_TAP, self.tap);
        put_bit(SETTING_TWO_FINGER_TAP, self.two_finger_tap);
        put_bit(SETTING_RIGHT_CLICK_ZONE, self.right_click_zone);
        parameters.cursor_speed = self.cursor_speed;
        parameters.sensitivity_level = self.sensitivity;
    }

    /// The registry values, with the defaults Microsoft documents for a
    /// value Windows has not written yet.
    fn from_registry() -> Self {
        let flag = |name: PCWSTR, default: bool| read_dword(name).map_or(default, |v| v != 0);
        Self {
            enabled: true,
            natural_scroll: !flag(w!("ScrollDirection"), false),
            cursor_speed: read_dword(w!("CursorSpeed")).unwrap_or(10),
            tap: flag(w!("TapsEnabled"), true),
            two_finger_tap: flag(w!("TwoFingerTapEnabled"), true),
            right_click_zone: flag(w!("RightClickZoneEnabled"), true),
            sensitivity: read_dword(w!("AAPThreshold")).unwrap_or(SENSITIVITY_MEDIUM),
            honors_mouse_accel: flag(w!("HonorMouseAccelSetting"), false),
        }
    }

    fn write_registry(self, current: Self) -> Result<(), Error> {
        if self.natural_scroll != current.natural_scroll {
            write_dword(w!("ScrollDirection"), u32::from(!self.natural_scroll))?;
        }
        if self.cursor_speed != current.cursor_speed {
            write_dword(w!("CursorSpeed"), self.cursor_speed)?;
        }
        if self.tap != current.tap {
            write_dword(w!("TapsEnabled"), u32::from(self.tap))?;
        }
        if self.two_finger_tap != current.two_finger_tap {
            write_dword(w!("TwoFingerTapEnabled"), u32::from(self.two_finger_tap))?;
        }
        if self.right_click_zone != current.right_click_zone {
            write_dword(
                w!("RightClickZoneEnabled"),
                u32::from(self.right_click_zone),
            )?;
        }
        if self.sensitivity != current.sensitivity {
            write_dword(w!("AAPThreshold"), self.sensitivity)?;
        }
        Ok(())
    }
}

enum Touchpad {
    /// Windows 11 24H2+: live parameters that apply immediately.
    Live(TouchpadParameters),
    /// Older Windows with a Precision Touchpad: registry values only.
    Registry(TouchpadValues),
    Absent,
}

impl Touchpad {
    fn read() -> Self {
        if let Some(parameters) = read_touchpad_parameters() {
            return if parameters.status & STATUS_TOUCHPAD_PRESENT != 0 {
                Self::Live(parameters)
            } else {
                Self::Absent
            };
        }
        if precision_touchpad_present() {
            Self::Registry(TouchpadValues::from_registry())
        } else {
            Self::Absent
        }
    }

    fn values(&self) -> Option<TouchpadValues> {
        match self {
            Self::Live(parameters) => Some(TouchpadValues::from_parameters(parameters)),
            Self::Registry(values) => Some(*values),
            Self::Absent => None,
        }
    }
}

/// The raw Windows values behind the non-touchpad settings.
struct SystemValues {
    keyboard_speed: u32,
    keyboard_delay: u32,
    mouse_speed: u32,
    mouse_precision: [i32; 3],
    buttons_swapped: bool,
    double_click_ms: u32,
    wheel_lines: u32,
}

impl SystemValues {
    fn read() -> Result<Self, Error> {
        let mut keyboard_speed = 0u32;
        let mut keyboard_delay = 0u32;
        let mut mouse_speed = 0u32;
        let mut mouse_precision = [0i32; 3];
        let mut wheel_lines = 0u32;
        get(
            SPI_GETKEYBOARDSPEED,
            &mut keyboard_speed,
            "read the key repeat rate",
        )?;
        get(
            SPI_GETKEYBOARDDELAY,
            &mut keyboard_delay,
            "read the key repeat delay",
        )?;
        get(
            SPI_GETMOUSESPEED,
            &mut mouse_speed,
            "read the pointer speed",
        )?;
        get(SPI_GETMOUSE, &mut mouse_precision, "read pointer precision")?;
        get(
            SPI_GETWHEELSCROLLLINES,
            &mut wheel_lines,
            "read wheel scroll lines",
        )?;
        // SAFETY: plain queries without pointers.
        let (buttons_swapped, double_click_ms) =
            unsafe { (GetSystemMetrics(SM_SWAPBUTTON) != 0, GetDoubleClickTime()) };
        Ok(Self {
            keyboard_speed,
            keyboard_delay,
            mouse_speed,
            mouse_precision,
            buttons_swapped,
            double_click_ms,
            wheel_lines,
        })
    }

    fn accel_profile(&self) -> AccelProfile {
        if self.mouse_precision[2] != 0 {
            AccelProfile::Adaptive
        } else {
            AccelProfile::Flat
        }
    }
}

/// `SystemParametersInfoW` reading into `value`.
fn get<T>(
    action: SYSTEM_PARAMETERS_INFO_ACTION,
    value: &mut T,
    operation: &'static str,
) -> Result<(), Error> {
    // SAFETY: `value` is the buffer type the action writes (a UINT/int or,
    // for SPI_GETMOUSE, an int[3]), valid for the call.
    unsafe {
        SystemParametersInfoW(
            action,
            0,
            Some((value as *mut T).cast::<c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|error| Error::new(operation, error.to_string()))
}

/// `SystemParametersInfoW` setting a value, persisted and broadcast.
fn set(
    action: SYSTEM_PARAMETERS_INFO_ACTION,
    uiparam: u32,
    pvparam: Option<*mut c_void>,
    operation: &'static str,
) -> Result<(), Error> {
    // SAFETY: callers pass either no pointer, an integer the action takes in
    // `pvParam` by value, or a pointer to a live buffer of the action's type.
    unsafe {
        SystemParametersInfoW(
            action,
            uiparam,
            pvparam,
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    }
    .map_err(|error| Error::new(operation, error.to_string()))
}

fn read_touchpad_parameters() -> Option<TouchpadParameters> {
    let mut parameters = TouchpadParameters {
        version_number: TOUCHPAD_PARAMETERS_VERSION_1,
        ..TouchpadParameters::default()
    };
    // SAFETY: `parameters` is a TOUCHPAD_PARAMETERS_V1 and uiParam says its
    // size. Windows before 24H2 rejects the action, which means "no API".
    unsafe {
        SystemParametersInfoW(
            SPI_GETTOUCHPADPARAMETERS,
            std::mem::size_of::<TouchpadParameters>() as u32,
            Some((&mut parameters as *mut TouchpadParameters).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    Some(parameters)
}

fn write_touchpad_parameters(mut parameters: TouchpadParameters) -> Result<(), Error> {
    parameters.version_number = TOUCHPAD_PARAMETERS_VERSION_1;
    set(
        SPI_SETTOUCHPADPARAMETERS,
        std::mem::size_of::<TouchpadParameters>() as u32,
        Some((&mut parameters as *mut TouchpadParameters).cast()),
        "change trackpad settings",
    )
}

/// Whether a HID Precision Touchpad (usage page Digitizer 0x0D, usage
/// Touch Pad 0x05) is connected.
fn precision_touchpad_present() -> bool {
    let entry = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
    // A device can arrive between the count and the fill; retry briefly.
    for _ in 0..3 {
        let mut count = 0u32;
        // SAFETY: a null list asks only for the count.
        if unsafe { GetRawInputDeviceList(None, &mut count, entry) } == u32::MAX {
            return false;
        }
        let mut devices = vec![RAWINPUTDEVICELIST::default(); count as usize];
        // SAFETY: `devices` holds `count` entries of `entry` bytes.
        let filled =
            unsafe { GetRawInputDeviceList(Some(devices.as_mut_ptr()), &mut count, entry) };
        if filled == u32::MAX {
            continue;
        }
        devices.truncate(filled as usize);
        return devices
            .iter()
            .any(|device| device.dwType == RIM_TYPEHID && is_precision_touchpad(device.hDevice));
    }
    false
}

fn is_precision_touchpad(device: HANDLE) -> bool {
    let mut info = RID_DEVICE_INFO {
        cbSize: std::mem::size_of::<RID_DEVICE_INFO>() as u32,
        ..RID_DEVICE_INFO::default()
    };
    let mut size = info.cbSize;
    // SAFETY: `info` is an RID_DEVICE_INFO with cbSize set, `size` its size.
    let read = unsafe {
        GetRawInputDeviceInfoW(
            Some(device),
            RIDI_DEVICEINFO,
            Some((&mut info as *mut RID_DEVICE_INFO).cast()),
            &mut size,
        )
    };
    if read == 0 || read == u32::MAX || info.dwType != RIM_TYPEHID {
        return false;
    }
    // SAFETY: dwType is RIM_TYPEHID, so the `hid` member is the one written.
    let hid = unsafe { info.Anonymous.hid };
    hid.usUsagePage == 0x0D && hid.usUsage == 0x05
}

fn read_dword(name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: `value` is a u32 and `size` says so.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            TOUCHPAD_KEY,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    Some(value)
}

fn write_dword(name: PCWSTR, value: u32) -> Result<(), Error> {
    // SAFETY: `value` is a live u32 and cbData says 4 bytes.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            TOUCHPAD_KEY,
            name,
            REG_DWORD.0,
            Some((&value as *const u32).cast()),
            std::mem::size_of::<u32>() as u32,
        )
    }
    .ok()
    .map_err(|error| Error::new("save trackpad settings", error.to_string()))
}

// --- Mappings between Windows' scales and the model's -------------------

/// Pointer and touchpad speed 1..=20 (default 10) to the model's -1..=1,
/// with Windows' default at 0: 1 maps to -1 and 20 to 1.
fn speed_to_accel(speed: u32) -> f64 {
    let speed = f64::from(speed.clamp(1, 20));
    if speed <= 10.0 {
        (speed - 10.0) / 9.0
    } else {
        (speed - 10.0) / 10.0
    }
}

fn accel_to_speed(accel: f64) -> u32 {
    let accel = accel.clamp(-1.0, 1.0);
    let speed = if accel <= 0.0 {
        10.0 + accel * 9.0
    } else {
        10.0 + accel * 10.0
    };
    (speed.round() as u32).clamp(1, 20)
}

/// `SPI_GETKEYBOARDDELAY` 0..=3 is "approximately 250 ms to 1 second" in
/// equal steps: 250, 500, 750, 1000 ms.
fn keyboard_delay_ms(index: u32) -> u32 {
    250 * (index.min(3) + 1)
}

fn keyboard_delay_index(delay_ms: u32) -> u32 {
    let steps = (f64::from(delay_ms) / 250.0 - 1.0).round();
    steps.clamp(0.0, 3.0) as u32
}

/// `SPI_GETKEYBOARDSPEED` 0..=31 is "approximately 2.5 to 30 repetitions
/// per second", linear in between; reported as whole characters per second.
fn keyboard_rate(speed: u32) -> u32 {
    (2.5 + f64::from(speed.min(31)) * 27.5 / 31.0).round() as u32
}

/// The lowest speed whose reported rate is closest to `rate`, so a rate read
/// from Windows writes back as the same rate.
fn keyboard_speed(rate: u32) -> u32 {
    (0..=31)
        .min_by_key(|speed| keyboard_rate(*speed).abs_diff(rate))
        .unwrap_or(31)
}

fn secondary_click(values: &TouchpadValues) -> SecondaryClick {
    if values.right_click_zone {
        SecondaryClick::CornerClick
    } else {
        SecondaryClick::TwoFingerClickOrTap
    }
}

/// The touchpad values `requested` asks for, starting from `current`.
fn requested_touchpad(current: TouchpadValues, requested: &TouchpadSettings) -> TouchpadValues {
    let mut next = current;
    next.natural_scroll = requested.pointer.natural_scroll;
    next.cursor_speed = accel_to_speed(requested.pointer.accel_speed);
    next.tap = requested.tap_to_click;
    // Typing suppression on Windows is a sensitivity level: "off" is the
    // most sensitive level; "on" keeps the user's level unless it was off.
    if !requested.disable_while_typing {
        next.sensitivity = SENSITIVITY_MOST;
    } else if current.sensitivity == SENSITIVITY_MOST {
        next.sensitivity = SENSITIVITY_MEDIUM;
    }
    match requested.secondary_click {
        SecondaryClick::CornerClick => next.right_click_zone = true,
        SecondaryClick::TwoFingerClickOrTap => {
            next.right_click_zone = false;
            next.two_finger_tap = true;
        }
    }
    next
}

fn supported(touchpad: bool) -> Supported {
    Supported {
        keyboard_repeat: true,
        keyboard_numlock: false,
        mouse_enabled: false,
        mouse_natural_scroll: false,
        mouse_accel_speed: true,
        mouse_accel_profile: true,
        mouse_left_handed: true,
        mouse_middle_emulation: false,
        mouse_scroll_with_middle_button: false,
        mouse_scroll_factor: false,
        touchpad,
        touchpad_enabled: false,
        touchpad_natural_scroll: touchpad,
        touchpad_accel_speed: touchpad,
        touchpad_accel_profile: false,
        touchpad_left_handed: false,
        touchpad_middle_emulation: false,
        touchpad_tap_to_click: touchpad,
        touchpad_disable_while_typing: touchpad,
        touchpad_drag_lock: false,
        touchpad_secondary_click: touchpad,
        trackpoint: false,
        touch: false,
        double_click_time: true,
        wheel_scroll_lines: true,
    }
}

fn settings_from(system: &SystemValues, touchpad: Option<TouchpadValues>) -> InputSettings {
    let mouse = PointerSettings {
        enabled: true,
        natural_scroll: false,
        accel_speed: speed_to_accel(system.mouse_speed),
        accel_profile: system.accel_profile(),
        left_handed: system.buttons_swapped,
        middle_emulation: false,
    };
    let touchpad = touchpad.map_or_else(TouchpadSettings::default, |values| TouchpadSettings {
        pointer: PointerSettings {
            enabled: values.enabled,
            natural_scroll: values.natural_scroll,
            accel_speed: speed_to_accel(values.cursor_speed),
            // Without "honor", Windows always accelerates touchpad motion.
            accel_profile: if values.honors_mouse_accel {
                system.accel_profile()
            } else {
                AccelProfile::Adaptive
            },
            // The primary button swap is system-wide.
            left_handed: system.buttons_swapped,
            middle_emulation: false,
        },
        tap_to_click: values.tap,
        disable_while_typing: values.sensitivity != SENSITIVITY_MOST,
        drag_lock: false,
        secondary_click: secondary_click(&values),
    });
    InputSettings {
        keyboard: KeyboardSettings {
            repeat_delay_ms: keyboard_delay_ms(system.keyboard_delay),
            repeat_rate: keyboard_rate(system.keyboard_speed),
            numlock: false,
            xkb_override: None,
        },
        mouse,
        mouse_scroll_with_middle_button: false,
        mouse_scroll_factor: 1.0,
        touchpad,
        trackpoint: PointerSettings::default(),
        trackpoint_scroll_with_middle_button: false,
        touch: TouchSettings::default_enabled(),
        double_click_ms: Some(system.double_click_ms),
        wheel_scroll_lines: Some(system.wheel_lines),
    }
}

fn snapshot_from(system: &SystemValues, touchpad: &Touchpad) -> Snapshot {
    let values = touchpad.values();
    Snapshot {
        available: true,
        can_configure: true,
        config_path: None,
        detail: matches!(touchpad, Touchpad::Registry(_)).then(|| {
            "This version of Windows may apply trackpad changes only after you sign in again."
                .into()
        }),
        keyboard_layout_authority: KeyboardLayoutAuthority::Unavailable,
        settings: settings_from(system, values),
        included_files: 0,
        devices: Vec::new(),
        device_detail: None,
        device_overrides: DeviceOverrideCapability::Unsupported,
        supported: supported(values.is_some()),
    }
}

pub(super) fn snapshot() -> Result<Snapshot, Error> {
    let system = SystemValues::read()?;
    Ok(snapshot_from(&system, &Touchpad::read()))
}

fn validate(settings: &InputSettings) -> Result<(), Error> {
    if !(100..=2_000).contains(&settings.keyboard.repeat_delay_ms) {
        return Err(Error::new(
            "validate keyboard settings",
            "repeat delay must be between 100 and 2000 ms",
        ));
    }
    if !(1..=100).contains(&settings.keyboard.repeat_rate) {
        return Err(Error::new(
            "validate keyboard settings",
            "repeat rate must be between 1 and 100 characters per second",
        ));
    }
    for (device, pointer) in [
        ("mouse", &settings.mouse),
        ("trackpad", &settings.touchpad.pointer),
    ] {
        if !pointer.accel_speed.is_finite() || !(-1.0..=1.0).contains(&pointer.accel_speed) {
            return Err(Error::new(
                "validate pointer settings",
                format!("{device} tracking speed must be between -1 and 1"),
            ));
        }
    }
    if settings
        .double_click_ms
        .is_some_and(|ms| !(100..=5_000).contains(&ms))
    {
        return Err(Error::new(
            "validate pointer settings",
            "double-click time must be between 100 and 5000 ms",
        ));
    }
    if settings
        .wheel_scroll_lines
        .is_some_and(|lines| lines > 100 && lines != WHEEL_SCROLL_PAGE)
    {
        return Err(Error::new(
            "validate pointer settings",
            "wheel scrolling must be at most 100 lines or one screen",
        ));
    }
    Ok(())
}

/// The names of fields `requested` changes from `current` that this
/// platform cannot change.
fn unsupported_changes(
    current: &InputSettings,
    requested: &InputSettings,
    supported: &Supported,
) -> Vec<&'static str> {
    let mut changes = Vec::new();
    let mut check = |allowed: bool, changed: bool, name: &'static str| {
        if !allowed && changed {
            changes.push(name);
        }
    };
    let (now, next) = (&current.keyboard, &requested.keyboard);
    check(
        supported.keyboard_repeat,
        now.repeat_delay_ms != next.repeat_delay_ms || now.repeat_rate != next.repeat_rate,
        "key repeat",
    );
    check(
        supported.keyboard_numlock,
        now.numlock != next.numlock,
        "Num Lock",
    );
    check(
        false,
        now.xkb_override != next.xkb_override,
        "keyboard layout",
    );

    let (now, next) = (&current.mouse, &requested.mouse);
    check(
        supported.mouse_enabled,
        now.enabled != next.enabled,
        "mouse on/off",
    );
    check(
        supported.mouse_natural_scroll,
        now.natural_scroll != next.natural_scroll,
        "mouse natural scrolling",
    );
    check(
        supported.mouse_accel_speed,
        now.accel_speed != next.accel_speed,
        "mouse tracking speed",
    );
    check(
        supported.mouse_accel_profile,
        now.accel_profile != next.accel_profile,
        "pointer precision",
    );
    check(
        supported.mouse_left_handed,
        now.left_handed != next.left_handed,
        "primary mouse button",
    );
    check(
        supported.mouse_middle_emulation,
        now.middle_emulation != next.middle_emulation,
        "middle-button emulation",
    );
    check(
        supported.mouse_scroll_with_middle_button,
        current.mouse_scroll_with_middle_button != requested.mouse_scroll_with_middle_button,
        "scrolling with the middle button",
    );

    let (now, next) = (&current.touchpad, &requested.touchpad);
    check(
        supported.touchpad_enabled,
        now.pointer.enabled != next.pointer.enabled,
        "trackpad on/off",
    );
    check(
        supported.touchpad_natural_scroll,
        now.pointer.natural_scroll != next.pointer.natural_scroll,
        "trackpad natural scrolling",
    );
    check(
        supported.touchpad_accel_speed,
        now.pointer.accel_speed != next.pointer.accel_speed,
        "trackpad tracking speed",
    );
    check(
        supported.touchpad_accel_profile,
        now.pointer.accel_profile != next.pointer.accel_profile,
        "trackpad acceleration",
    );
    check(
        supported.touchpad_left_handed,
        now.pointer.left_handed != next.pointer.left_handed,
        "trackpad primary button",
    );
    check(
        supported.touchpad_middle_emulation,
        now.pointer.middle_emulation != next.pointer.middle_emulation,
        "trackpad middle-button emulation",
    );
    check(
        supported.touchpad_tap_to_click,
        now.tap_to_click != next.tap_to_click,
        "tap to click",
    );
    check(
        supported.touchpad_disable_while_typing,
        now.disable_while_typing != next.disable_while_typing,
        "trackpad typing suppression",
    );
    check(
        supported.touchpad_drag_lock,
        now.drag_lock != next.drag_lock,
        "drag lock",
    );
    check(
        supported.touchpad_secondary_click,
        now.secondary_click != next.secondary_click,
        "secondary click",
    );

    check(
        supported.trackpoint,
        current.trackpoint != requested.trackpoint
            || current.trackpoint_scroll_with_middle_button
                != requested.trackpoint_scroll_with_middle_button,
        "pointing stick",
    );
    check(
        supported.touch,
        current.touch != requested.touch,
        "touchscreen",
    );
    check(
        supported.double_click_time,
        current.double_click_ms != requested.double_click_ms,
        "double-click speed",
    );
    check(
        supported.wheel_scroll_lines,
        current.wheel_scroll_lines != requested.wheel_scroll_lines,
        "wheel scrolling",
    );
    check(
        supported.mouse_scroll_factor,
        current.mouse_scroll_factor != requested.mouse_scroll_factor,
        "mouse scrolling speed",
    );
    changes
}

pub(super) fn save(requested: &InputSettings) -> Result<Snapshot, Error> {
    validate(requested)?;
    let system = SystemValues::read()?;
    let touchpad = Touchpad::read();
    let current = snapshot_from(&system, &touchpad);
    let unsupported = unsupported_changes(&current.settings, requested, &current.supported);
    if !unsupported.is_empty() {
        return Err(Error::new(
            "save input settings",
            format!("Windows has no setting for: {}", unsupported.join(", ")),
        ));
    }

    let repeat_speed = keyboard_speed(requested.keyboard.repeat_rate);
    if repeat_speed != system.keyboard_speed {
        set(
            SPI_SETKEYBOARDSPEED,
            repeat_speed,
            None,
            "change the key repeat rate",
        )?;
    }
    let repeat_delay = keyboard_delay_index(requested.keyboard.repeat_delay_ms);
    if repeat_delay != system.keyboard_delay {
        set(
            SPI_SETKEYBOARDDELAY,
            repeat_delay,
            None,
            "change the key repeat delay",
        )?;
    }
    let mouse_speed = accel_to_speed(requested.mouse.accel_speed);
    if mouse_speed != system.mouse_speed {
        // SPI_SETMOUSESPEED takes the speed itself in pvParam.
        set(
            SPI_SETMOUSESPEED,
            0,
            Some(mouse_speed as usize as *mut c_void),
            "change the pointer speed",
        )?;
    }
    if requested.mouse.accel_profile != system.accel_profile() {
        let mut precision = match requested.mouse.accel_profile {
            AccelProfile::Adaptive => ENHANCED_PRECISION,
            AccelProfile::Flat => NO_PRECISION,
        };
        set(
            SPI_SETMOUSE,
            0,
            Some(precision.as_mut_ptr().cast()),
            "change pointer precision",
        )?;
    }
    if requested.mouse.left_handed != system.buttons_swapped {
        set(
            SPI_SETMOUSEBUTTONSWAP,
            u32::from(requested.mouse.left_handed),
            None,
            "change the primary mouse button",
        )?;
    }
    if let Some(ms) = requested
        .double_click_ms
        .filter(|ms| *ms != system.double_click_ms)
    {
        set(
            SPI_SETDOUBLECLICKTIME,
            ms,
            None,
            "change the double-click speed",
        )?;
    }
    if let Some(lines) = requested
        .wheel_scroll_lines
        .filter(|lines| *lines != system.wheel_lines)
    {
        set(
            SPI_SETWHEELSCROLLLINES,
            lines,
            None,
            "change wheel scrolling",
        )?;
    }

    match touchpad {
        Touchpad::Live(mut parameters) => {
            let now = TouchpadValues::from_parameters(&parameters);
            let next = requested_touchpad(now, &requested.touchpad);
            if next != now {
                next.write_to(&mut parameters);
                write_touchpad_parameters(parameters)?;
            }
        }
        Touchpad::Registry(now) => {
            requested_touchpad(now, &requested.touchpad).write_registry(now)?;
        }
        Touchpad::Absent => {}
    }
    snapshot()
}

// --- Watch ---------------------------------------------------------------

thread_local! {
    /// The watch thread's sender, for its window procedure.
    static EVENTS: RefCell<Option<Sender<WatchEvent>>> = const { RefCell::new(None) };
}

/// `WM_SETTINGCHANGE` carries the `SPI_SET*` action in wParam; only these
/// change something in the model.
fn is_input_setting(action: usize) -> bool {
    [
        SPI_SETKEYBOARDSPEED,
        SPI_SETKEYBOARDDELAY,
        SPI_SETMOUSESPEED,
        SPI_SETMOUSE,
        SPI_SETMOUSEBUTTONSWAP,
        SPI_SETDOUBLECLICKTIME,
        SPI_SETWHEELSCROLLLINES,
        SPI_SETTOUCHPADPARAMETERS,
    ]
    .iter()
    .any(|setting| setting.0 as usize == action)
}

unsafe extern "system" fn listener_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if (message == WM_SETTINGCHANGE && is_input_setting(wparam.0)) || message == WM_DEVICECHANGE {
        EVENTS.with(|events| {
            if let Some(events) = events.borrow().as_ref() {
                let _ = events.try_send(WatchEvent::Changed);
            }
        });
    }
    // SAFETY: forwarding this window's own message unchanged.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// A hidden top-level window: broadcasts skip message-only windows.
fn create_listener_window() -> Result<HWND, Error> {
    let operation = "watch Windows input settings";
    // SAFETY: the null name asks for this executable's own module.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|error| Error::new(operation, error.to_string()))?
        .into();
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(listener_proc),
        hInstance: instance,
        lpszClassName: LISTENER_CLASS,
        ..WNDCLASSEXW::default()
    };
    // SAFETY: `class` is fully initialised. A second watch finds the class
    // already registered, which is fine; CreateWindowExW reports a real
    // failure.
    unsafe { RegisterClassExW(&class) };
    // SAFETY: the class above, no parent, never shown.
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            LISTENER_CLASS,
            w!("Lulo input settings"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|error| Error::new(operation, error.to_string()))
}

/// A one-shot `RegNotifyChangeKeyValue` on the touchpad key, re-armed after
/// each change.
struct RegistryWatch {
    key: HKEY,
    event: HANDLE,
}

impl RegistryWatch {
    /// `None` when the key does not exist (no touchpad settings yet).
    fn open() -> Option<Self> {
        let mut key = HKEY::default();
        // SAFETY: `key` receives an open handle, closed by Drop.
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, TOUCHPAD_KEY, None, KEY_NOTIFY, &mut key) }
            .ok()
            .ok()?;
        // SAFETY: an unnamed auto-reset event.
        let event = match unsafe { CreateEventW(None, false, false, PCWSTR::null()) } {
            Ok(event) => event,
            Err(_) => {
                // SAFETY: the key opened above.
                let _ = unsafe { RegCloseKey(key) };
                return None;
            }
        };
        let watch = Self { key, event };
        watch.arm().then_some(watch)
    }

    fn arm(&self) -> bool {
        // SAFETY: the key and event stay open while `self` lives, and the
        // arming thread (the watch thread) outlives every notification.
        unsafe {
            RegNotifyChangeKeyValue(
                self.key,
                false,
                REG_NOTIFY_CHANGE_LAST_SET,
                Some(self.event),
                true,
            )
        }
        .is_ok()
    }
}

impl Drop for RegistryWatch {
    fn drop(&mut self) {
        // SAFETY: both handles were opened by `open` and are closed once.
        unsafe {
            let _ = RegCloseKey(self.key);
            let _ = CloseHandle(self.event);
        }
    }
}

fn watch_thread(events: Sender<WatchEvent>, ready: Sender<Result<u32, Error>>) {
    EVENTS.with(|slot| *slot.borrow_mut() = Some(events.clone()));
    let window = match create_listener_window() {
        Ok(window) => window,
        Err(error) => {
            let _ = ready.send_blocking(Err(error));
            return;
        }
    };
    let mut registry = RegistryWatch::open();
    // SAFETY: no arguments; the window above created this thread's queue.
    let _ = ready.send_blocking(Ok(unsafe { GetCurrentThreadId() }));
    drop(ready);

    'wait: loop {
        let handles = registry
            .as_ref()
            .map(|watch| std::slice::from_ref(&watch.event));
        // SAFETY: the handles stay open across the wait.
        let woke = unsafe { MsgWaitForMultipleObjects(handles, false, INFINITE, QS_ALLINPUT) };
        if woke == WAIT_FAILED {
            break;
        }
        if registry.is_some() && woke == WAIT_OBJECT_0 {
            if !registry.as_ref().is_some_and(RegistryWatch::arm) {
                registry = None;
            }
            if events.try_send(WatchEvent::Changed).is_err() && events.is_closed() {
                break;
            }
            continue;
        }
        let mut message = MSG::default();
        // SAFETY: `message` is a valid MSG; sent messages are dispatched to
        // `listener_proc` inside PeekMessageW.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT {
                break 'wait;
            }
            // SAFETY: a message just retrieved for this thread.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if events.is_closed() {
            break;
        }
    }

    drop(registry);
    // SAFETY: the window this thread created.
    let _ = unsafe { DestroyWindow(window) };
    EVENTS.with(|slot| slot.borrow_mut().take());
}

pub(super) async fn watch(sender: Sender<WatchEvent>) -> Result<(), Error> {
    let operation = "watch Windows input settings";
    let (ready, started) = async_channel::bounded(1);
    let events = sender.clone();
    std::thread::Builder::new()
        .name("lulo input settings watch".into())
        .spawn(move || watch_thread(events, ready))
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let thread = started
        .recv()
        .await
        .map_err(|_| Error::new(operation, "the watch thread stopped"))??;
    sender.closed().await;
    // Wake the thread out of MsgWaitForMultipleObjects; it may already have
    // left after noticing the closed channel.
    // SAFETY: posting to a thread id; no pointers.
    let _ = unsafe { PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_speed_maps_ends_and_default_and_round_trips() {
        assert_eq!(speed_to_accel(1), -1.0);
        assert_eq!(speed_to_accel(10), 0.0);
        assert_eq!(speed_to_accel(20), 1.0);
        for speed in 1..=20 {
            assert_eq!(accel_to_speed(speed_to_accel(speed)), speed);
        }
    }

    #[test]
    fn keyboard_delay_uses_quarter_second_steps() {
        assert_eq!(keyboard_delay_ms(0), 250);
        assert_eq!(keyboard_delay_ms(3), 1_000);
        assert_eq!(keyboard_delay_index(100), 0);
        assert_eq!(keyboard_delay_index(600), 1);
        assert_eq!(keyboard_delay_index(2_000), 3);
        for index in 0..=3 {
            assert_eq!(keyboard_delay_index(keyboard_delay_ms(index)), index);
        }
    }

    #[test]
    fn keyboard_rate_spans_windows_range_and_is_stable() {
        assert_eq!(keyboard_rate(0), 3);
        assert_eq!(keyboard_rate(31), 30);
        assert_eq!(keyboard_speed(100), 31);
        for speed in 0..=31 {
            let rate = keyboard_rate(speed);
            assert_eq!(keyboard_rate(keyboard_speed(rate)), rate);
        }
    }

    #[test]
    fn touchpad_parameters_match_the_v1_layout_and_bits() {
        assert_eq!(std::mem::size_of::<TouchpadParameters>(), 44);
        let mut parameters = TouchpadParameters {
            settings: SETTING_SCROLL_REVERSED | SETTING_TAP,
            cursor_speed: 10,
            ..TouchpadParameters::default()
        };
        let values = TouchpadValues::from_parameters(&parameters);
        assert!(!values.natural_scroll && values.tap);
        let next = TouchpadValues {
            natural_scroll: true,
            tap: false,
            right_click_zone: true,
            ..values
        };
        next.write_to(&mut parameters);
        assert_eq!(parameters.settings, SETTING_RIGHT_CLICK_ZONE);
        assert_eq!(TouchpadValues::from_parameters(&parameters), next);
    }

    #[test]
    fn typing_suppression_and_secondary_click_map_to_windows_settings() {
        let current = TouchpadValues {
            enabled: true,
            natural_scroll: true,
            cursor_speed: 10,
            tap: true,
            two_finger_tap: false,
            right_click_zone: true,
            sensitivity: 3,
            honors_mouse_accel: false,
        };
        let mut requested = TouchpadSettings {
            pointer: PointerSettings {
                natural_scroll: true,
                ..PointerSettings::default()
            },
            tap_to_click: true,
            disable_while_typing: true,
            drag_lock: false,
            secondary_click: SecondaryClick::CornerClick,
        };
        assert_eq!(requested_touchpad(current, &requested), current);
        requested.disable_while_typing = false;
        requested.secondary_click = SecondaryClick::TwoFingerClickOrTap;
        let next = requested_touchpad(current, &requested);
        assert_eq!(next.sensitivity, SENSITIVITY_MOST);
        assert!(!next.right_click_zone && next.two_finger_tap);
        requested.disable_while_typing = true;
        assert_eq!(
            requested_touchpad(next, &requested).sensitivity,
            SENSITIVITY_MEDIUM
        );
    }

    #[test]
    fn unsupported_fields_are_named_and_supported_ones_pass() {
        let current = InputSettings {
            double_click_ms: Some(500),
            wheel_scroll_lines: Some(3),
            ..InputSettings::default()
        };
        let supported = supported(false);
        let mut requested = current.clone();
        requested.mouse.accel_speed = 0.5;
        requested.double_click_ms = Some(400);
        assert!(unsupported_changes(&current, &requested, &supported).is_empty());
        requested.mouse.natural_scroll = true;
        requested.touchpad.tap_to_click = true;
        assert_eq!(
            unsupported_changes(&current, &requested, &supported),
            ["mouse natural scrolling", "tap to click"]
        );
    }
}
