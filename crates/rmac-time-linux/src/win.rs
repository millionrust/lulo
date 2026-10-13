//! Windows date and time: the time zone through the dynamic time-zone API,
//! automatic time through the W32Time service configuration, and change
//! notifications through the WM_TIMECHANGE / WM_SETTINGCHANGE broadcasts.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::OnceLock;

use rmac_time::{Error, ErrorKind, Snapshot, WatchEvent};
use windows::core::{s, w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_CLASS_ALREADY_EXISTS,
    ERROR_FILE_NOT_FOUND, ERROR_NOT_ALL_ASSIGNED, ERROR_PRIVILEGE_NOT_HELD,
    ERROR_SERVICE_NOT_ACTIVE, ERROR_SUCCESS, FILETIME, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT,
    LUID, SYSTEMTIME, WPARAM,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LookupPrivilegeValueW, TokenElevation,
    LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, SE_SYSTEMTIME_NAME, SE_TIME_ZONE_NAME,
    TOKEN_ACCESS_MASK, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
    KEY_SET_VALUE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_QWORD, RRF_RT_REG_SZ,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, StartServiceW,
    SC_MANAGER_CONNECT, SERVICE_CONTROL_PARAMCHANGE, SERVICE_PAUSE_CONTINUE, SERVICE_START,
    SERVICE_STATUS,
};
use windows::Win32::System::SystemInformation::SetSystemTime;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Time::{
    EnumDynamicTimeZoneInformation, FileTimeToSystemTime, GetDynamicTimeZoneInformation,
    SetDynamicTimeZoneInformation, DYNAMIC_TIME_ZONE_INFORMATION, TIME_ZONE_ID_INVALID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, MSG, WM_CLOSE, WM_DESTROY, WM_SETTINGCHANGE,
    WM_TIMECHANGE, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

const W32TIME_PARAMETERS: PCWSTR = w!("SYSTEM\\CurrentControlSet\\Services\\W32Time\\Parameters");
const TIME_ZONE_INFORMATION_KEY: PCWSTR =
    w!("SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation");
/// Windows ships a few hundred zones; the bound only guards a broken API.
const MAX_ENUMERATED_ZONES: u32 = 4096;
/// 100-ns intervals between 1601-01-01 (FILETIME) and 1970-01-01.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

pub(crate) fn system_snapshot() -> Result<Snapshot, Error> {
    let mut current = DYNAMIC_TIME_ZONE_INFORMATION::default();
    // SAFETY: `current` is a valid, writable structure.
    if unsafe { GetDynamicTimeZoneInformation(&mut current) } == TIME_ZONE_ID_INVALID {
        return Err(Error::new(
            ErrorKind::Unavailable,
            "Windows did not report the current time zone",
        ));
    }
    let key = wide_to_string(&current.TimeZoneKeyName);
    let timezone = windows_to_iana(&key)
        .filter(|zone| rmac_time::validate_timezone_syntax(zone).is_ok())
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Protocol,
                "the current Windows time zone has no IANA equivalent",
            )
        })?;
    let names = enumerate_zones()
        .iter()
        .filter_map(|zone| windows_to_iana(&wide_to_string(&zone.TimeZoneKeyName)))
        .collect::<Vec<_>>();
    let (mut timezones, timezones_truncated) = rmac_time::normalize_timezones(names);
    if !timezones.iter().any(|candidate| candidate == &timezone) {
        if timezones_truncated {
            timezones.pop();
        }
        timezones.push(timezone.clone());
        timezones.sort_unstable();
    }

    let sync_type = read_registry_string(W32TIME_PARAMETERS, w!("Type"));
    let ntp_enabled = sync_type.as_deref().is_some_and(sync_type_enabled);
    // Changing the W32Time configuration is reserved for administrators, so
    // a standard (or non-elevated) session reports automatic time as fixed.
    let can_ntp = sync_type.is_some() && process_is_elevated();

    Ok(Snapshot {
        timezone,
        local_rtc: !real_time_is_universal(),
        can_ntp,
        ntp_enabled,
        // W32Time exposes its synchronization state only through its own
        // RPC tooling; report "not confirmed" instead of guessing.
        synchronized: false,
        time_usec: now_usec()?,
        timezones,
        timezones_truncated,
    })
}

pub(crate) fn system_set_ntp(enabled: bool) -> Result<(), Error> {
    let value = to_wide(if enabled { "NTP" } else { "NoSync" });
    let mut key = HKEY::default();
    // SAFETY: valid constant key path; `key` receives the handle.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            W32TIME_PARAMETERS,
            None,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(if status == ERROR_ACCESS_DENIED {
            admin_error("automatic time")
        } else if status == ERROR_FILE_NOT_FOUND {
            Error::new(
                ErrorKind::Unavailable,
                "the Windows Time service is not installed",
            )
        } else {
            Error::new(
                ErrorKind::Mutation,
                "Windows rejected the automatic-time change",
            )
        });
    }
    // SAFETY: `value` is a NUL-terminated UTF-16 buffer that outlives the call.
    let bytes = unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
    // SAFETY: `key` is open with KEY_SET_VALUE.
    let status = unsafe { RegSetValueExW(key, w!("Type"), None, REG_SZ, Some(bytes)) };
    // SAFETY: `key` was opened above and is closed exactly once.
    let _ = unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS {
        return Err(if status == ERROR_ACCESS_DENIED {
            admin_error("automatic time")
        } else {
            Error::new(
                ErrorKind::Mutation,
                "Windows rejected the automatic-time change",
            )
        });
    }
    // The registry change is the setting itself; the service only rereads
    // it when told to (or at its next start), so a failed notification is
    // not a failed change.
    notify_time_service(enabled);
    Ok(())
}

pub(crate) fn system_set_timezone(timezone: &str) -> Result<(), Error> {
    let zones = enumerate_zones();
    let by_key = iana_to_windows(timezone).and_then(|windows_id| {
        zones
            .iter()
            .find(|zone| wide_to_string(&zone.TimeZoneKeyName).eq_ignore_ascii_case(&windows_id))
    });
    // The list Settings offers comes from the forward mapping, so a zone it
    // offered always matches one entry here even when the reverse lookup
    // has no answer.
    let zone = by_key
        .or_else(|| {
            zones.iter().find(|zone| {
                windows_to_iana(&wide_to_string(&zone.TimeZoneKeyName)).as_deref() == Some(timezone)
            })
        })
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidTimezone,
                "the time zone is not known to Windows",
            )
        })?;
    let mut zone = *zone;
    zone.DynamicDaylightTimeDisabled = false;
    enable_privilege(SE_TIME_ZONE_NAME).map_err(|_| admin_error("the time zone"))?;
    // SAFETY: `zone` is a complete entry returned by Windows itself.
    unsafe { SetDynamicTimeZoneInformation(&zone) }.map_err(|error| {
        if is_permission_error(&error) {
            admin_error("the time zone")
        } else {
            Error::new(ErrorKind::Mutation, "Windows rejected the time-zone change")
        }
    })
}

pub(crate) fn system_set_time(time_usec: u64) -> Result<(), Error> {
    let ticks = unix_usec_to_filetime(time_usec).ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidTime,
            "the requested system time is outside the Windows clock range",
        )
    })?;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut system_time = SYSTEMTIME::default();
    // SAFETY: both pointers reference valid local structures.
    unsafe { FileTimeToSystemTime(&file_time, &mut system_time) }.map_err(|_| {
        Error::new(
            ErrorKind::InvalidTime,
            "the requested system time is outside the Windows clock range",
        )
    })?;
    enable_privilege(SE_SYSTEMTIME_NAME).map_err(|_| admin_error("the clock"))?;
    // SAFETY: `system_time` is a valid UTC SYSTEMTIME.
    unsafe { SetSystemTime(&system_time) }.map_err(|error| {
        if is_permission_error(&error) {
            admin_error("the clock")
        } else {
            Error::new(ErrorKind::Mutation, "Windows rejected the clock change")
        }
    })
}

pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let (ready_tx, ready_rx) = async_channel::bounded(1);
    let thread_sender = sender.clone();
    std::thread::Builder::new()
        .name("lulo-time-watch".into())
        .spawn(move || run_watch_window(thread_sender, ready_tx))
        .map_err(|_| Error::new(ErrorKind::Unavailable, "could not start the clock watcher"))?;
    let window = match ready_rx.recv().await {
        Ok(Ok(window)) => window,
        Ok(Err(error)) => {
            let _ = sender.try_send(WatchEvent::Unavailable);
            return Err(error);
        }
        Err(_) => {
            let _ = sender.try_send(WatchEvent::Unavailable);
            return Err(Error::new(
                ErrorKind::Unavailable,
                "the clock watcher stopped",
            ));
        }
    };
    // Closes the window (ending its thread) however this future ends,
    // including when it is dropped while waiting.
    let _window = CloseOnDrop(window);
    // Publish only once the window exists, so the consumer's first snapshot
    // cannot miss a change that happens while the watcher starts.
    if sender.send(WatchEvent::Changed).await.is_ok() {
        sender.closed().await;
    }
    Ok(())
}

struct CloseOnDrop(isize);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        // SAFETY: posting to a window owned by another thread is allowed; if
        // it is already gone the call simply fails.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.0 as *mut c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

thread_local! {
    static WATCH_SENDER: RefCell<Option<async_channel::Sender<WatchEvent>>> =
        const { RefCell::new(None) };
}

fn run_watch_window(
    sender: async_channel::Sender<WatchEvent>,
    ready: async_channel::Sender<Result<isize, Error>>,
) {
    WATCH_SENDER.with(|slot| *slot.borrow_mut() = Some(sender));
    let window = match create_watch_window() {
        Ok(window) => window,
        Err(error) => {
            let _ = ready.try_send(Err(error));
            WATCH_SENDER.with(|slot| slot.borrow_mut().take());
            return;
        }
    };
    if ready.try_send(Ok(window.0 as isize)).is_err() {
        // SAFETY: the window belongs to this thread.
        let _ = unsafe { DestroyWindow(window) };
    }
    let mut message = MSG::default();
    // GetMessageW returns 0 for WM_QUIT and -1 on failure; both end the loop.
    // SAFETY: `message` is a valid, writable MSG.
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        // SAFETY: `message` was filled by GetMessageW.
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    WATCH_SENDER.with(|slot| slot.borrow_mut().take());
}

fn create_watch_window() -> Result<HWND, Error> {
    let unavailable = || Error::new(ErrorKind::Unavailable, "could not create the clock watcher");
    let class_name = w!("LuloTimeWatcher");
    // SAFETY: a null name returns this executable's module handle.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|_| unavailable())?
        .into();
    let class = WNDCLASSW {
        lpfnWndProc: Some(watch_window_proc),
        hInstance: instance,
        lpszClassName: class_name,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised and its strings are 'static.
    // A second watcher in the same process reuses the class registered by
    // the first; it is identical.
    if unsafe { RegisterClassW(&class) } == 0
        && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err(unavailable());
    }
    // A hidden top-level window, not a message-only one: Windows broadcasts
    // clock and time-zone changes only to top-level windows.
    // SAFETY: the class was registered above with this instance.
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name,
            w!("Lulo time watcher"),
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
    .map_err(|_| unavailable())
}

unsafe extern "system" fn watch_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMECHANGE => notify_watch(),
        WM_SETTINGCHANGE if lparam.0 != 0 => {
            // SAFETY: for WM_SETTINGCHANGE a non-null lParam is a
            // NUL-terminated UTF-16 section name.
            let section = unsafe { PCWSTR(lparam.0 as *const u16).to_string() };
            if section.is_ok_and(|section| setting_section_is_time(&section)) {
                notify_watch();
            }
        }
        WM_DESTROY => {
            // SAFETY: called on the thread that owns the message loop.
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {}
    }
    // SAFETY: forwards the message unchanged to the default handler.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn notify_watch() {
    WATCH_SENDER.with(|slot| {
        if let Some(sender) = slot.borrow().as_ref() {
            let _ = sender.try_send(WatchEvent::Changed);
        }
    });
}

fn setting_section_is_time(section: &str) -> bool {
    section.eq_ignore_ascii_case("intl") || section.eq_ignore_ascii_case("TimeZone")
}

fn enumerate_zones() -> Vec<DYNAMIC_TIME_ZONE_INFORMATION> {
    let mut zones = Vec::new();
    for index in 0..MAX_ENUMERATED_ZONES {
        let mut zone = DYNAMIC_TIME_ZONE_INFORMATION::default();
        // SAFETY: `zone` is a valid, writable structure.
        if unsafe { EnumDynamicTimeZoneInformation(index, &mut zone) } != ERROR_SUCCESS.0 {
            break;
        }
        zones.push(zone);
    }
    zones
}

type WindowsToIana =
    unsafe extern "C" fn(*const u16, i32, *const std::ffi::c_char, *mut u16, i32, *mut i32) -> i32;
type IanaToWindows = unsafe extern "C" fn(*const u16, i32, *mut u16, i32, *mut i32) -> i32;

struct Icu {
    windows_to_iana: WindowsToIana,
    iana_to_windows: Option<IanaToWindows>,
}

/// The ICU that Windows 10 1903 and later ship in System32. Loaded at run
/// time so older systems fall back to the built-in table.
fn icu() -> Option<&'static Icu> {
    static ICU: OnceLock<Option<Icu>> = OnceLock::new();
    ICU.get_or_init(|| {
        // SAFETY: loads a system DLL by name from System32 only.
        let module =
            unsafe { LoadLibraryExW(w!("icu.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
        // SAFETY: the module stays loaded for the life of the process (it is
        // never freed), and both exports have the C signatures declared above.
        unsafe {
            let windows_to_iana = GetProcAddress(module, s!("ucal_getTimeZoneIDForWindowsID"))?;
            let iana_to_windows = GetProcAddress(module, s!("ucal_getWindowsTimeZoneID"));
            Some(Icu {
                windows_to_iana: std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    WindowsToIana,
                >(windows_to_iana),
                iana_to_windows: iana_to_windows.map(|function| {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, IanaToWindows>(
                        function,
                    )
                }),
            })
        }
    })
    .as_ref()
}

fn windows_to_iana(windows_id: &str) -> Option<String> {
    if windows_id.is_empty() {
        return None;
    }
    if let Some(icu) = icu() {
        let input = windows_id.encode_utf16().collect::<Vec<_>>();
        let mut output = [0_u16; 128];
        let mut status = 0_i32;
        // SAFETY: input/output buffers and lengths match; the region is a
        // NUL-terminated C string ("001" is CLDR's "world" default).
        let length = unsafe {
            (icu.windows_to_iana)(
                input.as_ptr(),
                input.len() as i32,
                c"001".as_ptr(),
                output.as_mut_ptr(),
                output.len() as i32,
                &mut status,
            )
        };
        // ICU failures are positive codes; negative values are warnings.
        if status <= 0 && length > 0 && (length as usize) <= output.len() {
            if let Ok(zone) = String::from_utf16(&output[..length as usize]) {
                return Some(current_iana_name(&zone).to_string());
            }
        }
    }
    builtin_windows_to_iana(windows_id).map(str::to_string)
}

fn iana_to_windows(timezone: &str) -> Option<String> {
    if let Some(function) = icu().and_then(|icu| icu.iana_to_windows) {
        let input = timezone.encode_utf16().collect::<Vec<_>>();
        let mut output = [0_u16; 128];
        let mut status = 0_i32;
        // SAFETY: input/output buffers and lengths match.
        let length = unsafe {
            function(
                input.as_ptr(),
                input.len() as i32,
                output.as_mut_ptr(),
                output.len() as i32,
                &mut status,
            )
        };
        if status <= 0 && length > 0 && (length as usize) <= output.len() {
            if let Ok(windows_id) = String::from_utf16(&output[..length as usize]) {
                return Some(windows_id);
            }
        }
    }
    builtin_iana_to_windows(timezone).map(str::to_string)
}

/// CLDR's Windows mapping still uses some superseded IANA names; Settings
/// shows the current ones.
fn current_iana_name(zone: &str) -> &str {
    match zone {
        "Asia/Calcutta" => "Asia/Kolkata",
        "Asia/Katmandu" => "Asia/Kathmandu",
        "Asia/Saigon" => "Asia/Ho_Chi_Minh",
        "Asia/Rangoon" => "Asia/Yangon",
        "Europe/Kiev" => "Europe/Kyiv",
        "America/Godthab" => "America/Nuuk",
        "America/Buenos_Aires" => "America/Argentina/Buenos_Aires",
        "America/Indianapolis" => "America/Indiana/Indianapolis",
        "Atlantic/Faeroe" => "Atlantic/Faroe",
        "Pacific/Enderbury" => "Pacific/Kanton",
        other => other,
    }
}

/// Windows zone id → IANA name (CLDR "001" mapping, current IANA names),
/// used only when the system ICU is missing.
const BUILTIN_ZONES: &[(&str, &str)] = &[
    ("Dateline Standard Time", "Etc/GMT+12"),
    ("UTC-11", "Etc/GMT+11"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Pacific Standard Time (Mexico)", "America/Tijuana"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Mountain Standard Time (Mexico)", "America/Mazatlan"),
    ("Mountain Standard Time", "America/Denver"),
    ("Central America Standard Time", "America/Guatemala"),
    ("Central Standard Time", "America/Chicago"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Canada Central Standard Time", "America/Regina"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Eastern Standard Time", "America/New_York"),
    ("US Eastern Standard Time", "America/Indiana/Indianapolis"),
    ("Venezuela Standard Time", "America/Caracas"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("SA Western Standard Time", "America/La_Paz"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("SA Eastern Standard Time", "America/Cayenne"),
    ("Argentina Standard Time", "America/Argentina/Buenos_Aires"),
    ("Greenland Standard Time", "America/Nuuk"),
    ("UTC-02", "Etc/GMT+2"),
    ("Azores Standard Time", "Atlantic/Azores"),
    ("UTC", "Etc/UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("Morocco Standard Time", "Africa/Casablanca"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("FLE Standard Time", "Europe/Kyiv"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Jordan Standard Time", "Asia/Amman"),
    ("Middle East Standard Time", "Asia/Beirut"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("E. Africa Standard Time", "Africa/Nairobi"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Azerbaijan Standard Time", "Asia/Baku"),
    ("Georgian Standard Time", "Asia/Tbilisi"),
    ("Caucasus Standard Time", "Asia/Yerevan"),
    ("Afghanistan Standard Time", "Asia/Kabul"),
    ("Ekaterinburg Standard Time", "Asia/Yekaterinburg"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("West Asia Standard Time", "Asia/Tashkent"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Sri Lanka Standard Time", "Asia/Colombo"),
    ("Nepal Standard Time", "Asia/Kathmandu"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("Myanmar Standard Time", "Asia/Yangon"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("North Asia Standard Time", "Asia/Krasnoyarsk"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("Cen. Australia Standard Time", "Australia/Adelaide"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("West Pacific Standard Time", "Pacific/Port_Moresby"),
    ("Tasmania Standard Time", "Australia/Hobart"),
    ("Vladivostok Standard Time", "Asia/Vladivostok"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("UTC+12", "Etc/GMT-12"),
    ("Fiji Standard Time", "Pacific/Fiji"),
    ("Tonga Standard Time", "Pacific/Tongatapu"),
];

fn builtin_windows_to_iana(windows_id: &str) -> Option<&'static str> {
    BUILTIN_ZONES
        .iter()
        .find(|(windows, _)| windows.eq_ignore_ascii_case(windows_id))
        .map(|(_, iana)| *iana)
}

fn builtin_iana_to_windows(timezone: &str) -> Option<&'static str> {
    let timezone = current_iana_name(timezone);
    BUILTIN_ZONES
        .iter()
        .find(|(_, iana)| *iana == timezone)
        .map(|(windows, _)| *windows)
}

/// W32Time "Type": NoSync turns synchronization off; NTP, NT5DS (domain
/// hierarchy) and AllSync all keep the clock set automatically.
fn sync_type_enabled(value: &str) -> bool {
    !value.trim().eq_ignore_ascii_case("NoSync")
}

/// Windows keeps the hardware clock in local time unless this value is set.
fn real_time_is_universal() -> bool {
    let mut value = 0_u64;
    let mut size = std::mem::size_of::<u64>() as u32;
    // SAFETY: `value` is an 8-byte buffer and `size` states its length.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            TIME_ZONE_INFORMATION_KEY,
            w!("RealTimeIsUniversal"),
            RRF_RT_REG_DWORD | RRF_RT_REG_QWORD,
            None,
            Some((&mut value as *mut u64).cast::<c_void>()),
            Some(&mut size as *mut u32),
        )
    };
    status == ERROR_SUCCESS && value != 0
}

fn read_registry_string(key: PCWSTR, name: PCWSTR) -> Option<String> {
    let mut buffer = [0_u16; 128];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: `buffer` is writable and `size` states its length in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            Some(&mut size as *mut u32),
        )
    };
    (status == ERROR_SUCCESS).then(|| wide_to_string(&buffer))
}

fn notify_time_service(enabled: bool) {
    // SAFETY: plain service-manager calls; every handle opened here is
    // closed before returning.
    unsafe {
        let Ok(manager) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) else {
            return;
        };
        if let Ok(service) = OpenServiceW(
            manager,
            w!("W32Time"),
            SERVICE_PAUSE_CONTINUE | SERVICE_START,
        ) {
            let mut status = SERVICE_STATUS::default();
            if let Err(error) = ControlService(service, SERVICE_CONTROL_PARAMCHANGE, &mut status) {
                // The service is trigger-started and often stopped; start it
                // so turning automatic time on takes effect now.
                if enabled
                    && error.code()
                        == windows::core::HRESULT::from_win32(ERROR_SERVICE_NOT_ACTIVE.0)
                {
                    let _ = StartServiceW(service, None);
                }
            }
            let _ = CloseServiceHandle(service);
        }
        let _ = CloseServiceHandle(manager);
    }
}

struct Token(HANDLE);

impl Token {
    fn open(access: TOKEN_ACCESS_MASK) -> Option<Self> {
        let mut handle = HANDLE::default();
        // SAFETY: the pseudo-handle of the current process needs no closing;
        // `handle` receives a token that `Drop` closes.
        unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut handle) }.ok()?;
        Some(Self(handle))
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcessToken and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn process_is_elevated() -> bool {
    let Some(token) = Token::open(TOKEN_QUERY) else {
        return false;
    };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut length = 0_u32;
    // SAFETY: the buffer is a TOKEN_ELEVATION of the stated size.
    let result = unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
    };
    result.is_ok() && elevation.TokenIsElevated != 0
}

/// Enables a privilege the token already holds. Fails when the account does
/// not hold it at all (for example the system-time privilege for a
/// standard user).
fn enable_privilege(name: PCWSTR) -> Result<(), ()> {
    let token = Token::open(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY).ok_or(())?;
    let mut luid = LUID::default();
    // SAFETY: `name` is a constant privilege name; `luid` is writable.
    unsafe { LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid) }.map_err(|_| ())?;
    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    // SAFETY: `privileges` is a valid one-entry TOKEN_PRIVILEGES.
    unsafe {
        AdjustTokenPrivileges(
            token.0,
            false,
            Some(&privileges as *const TOKEN_PRIVILEGES),
            0,
            None,
            None,
        )
    }
    .map_err(|_| ())?;
    // AdjustTokenPrivileges succeeds even when it enabled nothing; only the
    // thread's last error says whether the account holds the privilege.
    // SAFETY: reads this thread's last error, set by the call above.
    if unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
        return Err(());
    }
    Ok(())
}

fn is_permission_error(error: &windows::core::Error) -> bool {
    let code = error.code();
    code == windows::core::HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
        || code == windows::core::HRESULT::from_win32(ERROR_PRIVILEGE_NOT_HELD.0)
}

fn admin_error(what: &str) -> Error {
    Error::new(
        ErrorKind::Authorization,
        format!("changing {what} on Windows requires an administrator"),
    )
}

fn now_usec() -> Result<u64, Error> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::new(ErrorKind::Protocol, "the system clock predates 1970"))?;
    u64::try_from(elapsed.as_micros())
        .map_err(|_| Error::new(ErrorKind::Protocol, "the system clock is out of range"))
}

fn unix_usec_to_filetime(time_usec: u64) -> Option<u64> {
    time_usec
        .checked_mul(10)?
        .checked_add(FILETIME_UNIX_EPOCH)
        // FileTimeToSystemTime rejects values with the top bit set.
        .filter(|ticks| *ticks <= i64::MAX as u64)
}

fn wide_to_string(wide: &[u16]) -> String {
    let end = wide
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_table_maps_common_zones_both_ways() {
        assert!(BUILTIN_ZONES.len() >= 40);
        assert_eq!(
            builtin_windows_to_iana("India Standard Time"),
            Some("Asia/Kolkata")
        );
        assert_eq!(
            builtin_iana_to_windows("Asia/Calcutta"),
            Some("India Standard Time")
        );
        for (windows_id, iana) in BUILTIN_ZONES {
            assert!(rmac_time::validate_timezone_syntax(iana).is_ok(), "{iana}");
            assert_eq!(builtin_iana_to_windows(iana), Some(*windows_id));
        }
    }

    #[test]
    fn superseded_cldr_names_become_current() {
        assert_eq!(current_iana_name("Asia/Calcutta"), "Asia/Kolkata");
        assert_eq!(current_iana_name("Europe/Kiev"), "Europe/Kyiv");
        assert_eq!(current_iana_name("Europe/Paris"), "Europe/Paris");
    }

    #[test]
    fn sync_types_and_setting_sections() {
        assert!(sync_type_enabled("NTP"));
        assert!(sync_type_enabled("AllSync"));
        assert!(sync_type_enabled("NT5DS"));
        assert!(!sync_type_enabled("NoSync"));
        assert!(setting_section_is_time("intl"));
        assert!(setting_section_is_time("timezone"));
        assert!(!setting_section_is_time("WindowMetrics"));
    }

    #[test]
    fn unix_time_converts_to_filetime_ticks() {
        assert_eq!(unix_usec_to_filetime(0), Some(FILETIME_UNIX_EPOCH));
        assert_eq!(
            unix_usec_to_filetime(1_000_000),
            Some(FILETIME_UNIX_EPOCH + 10_000_000)
        );
        assert_eq!(unix_usec_to_filetime(u64::MAX), None);
        assert_eq!(
            wide_to_string(&[b'U' as u16, b'T' as u16, b'C' as u16, 0, 7]),
            "UTC"
        );
    }
}
