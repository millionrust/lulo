//! App windows on the desktop, as Alt+Tab lists them, and what the Dock
//! and the menu bar do to them.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{w, BOOL, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess,
    QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindow,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, IsZoomed, PostMessageW, SetForegroundWindow, ShowWindow, GA_ROOT, GWL_EXSTYLE,
    GW_OWNER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use crate::model::apps;
use crate::model::dock::Running;

use super::from_wide;

/// One app window, front-most first in a list.
#[derive(Clone, Debug)]
pub struct AppWindow {
    pub hwnd: isize,
    pub pid: u32,
    pub exe_path: String,
    pub title: String,
    pub minimized: bool,
    /// The window's own AppUserModelID, when it has one and its
    /// executable is not already a known Lulo app.
    pub aumid: Option<String>,
}

/// Explorer's own desktop and taskbar windows are not app windows.
const SHELL_CLASSES: [&str; 5] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Windows.UI.Core.CoreWindow",
];

pub fn handle(hwnd: isize) -> HWND {
    HWND(hwnd as *mut core::ffi::c_void)
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 128];
    // SAFETY: the buffer is writable and its length is passed.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

pub fn title(hwnd: HWND) -> String {
    // SAFETY: reads the caption Windows keeps for the window.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    // SAFETY: as above.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

pub fn process_id(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: plain out-parameter.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

pub fn own_process_id() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetCurrentProcessId() }
}

/// Whether Alt+Tab would show `hwnd`: visible, top-level, not a tool
/// window, not owned (unless it asks to be shown), not cloaked by DWM (a
/// suspended Store app or a window on another virtual desktop), titled.
fn is_app_window(hwnd: HWND, own_pid: u32) -> bool {
    // SAFETY: reads properties of a window EnumWindows just named.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || GetAncestor(hwnd, GA_ROOT) != hwnd {
            return false;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let app_window = style & WS_EX_APPWINDOW.0 != 0;
        if style & WS_EX_TOOLWINDOW.0 != 0 && !app_window {
            return false;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) && !app_window {
            return false;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return false;
        }
    }
    if process_id(hwnd) == own_pid || title(hwnd).is_empty() {
        return false;
    }
    let class = class_name(hwnd);
    !SHELL_CLASSES.contains(&class.as_str())
}

/// Whether `hwnd` is an app window (see `is_app_window`).
pub fn is_app(hwnd: isize) -> bool {
    is_app_window(handle(hwnd), own_process_id())
}

/// Whether `hwnd` is the desktop itself, where the Mac shows the Finder.
pub fn is_desktop(hwnd: isize) -> bool {
    matches!(class_name(handle(hwnd)).as_str(), "Progman" | "WorkerW")
}

thread_local! {
    static PROCESS_PATHS: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
    static DESCRIPTIONS: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// The executable path of process `pid`, or empty when it cannot be read
/// (an elevated process: Lulo never runs elevated).
pub fn process_path(pid: u32) -> String {
    if let Some(path) = PROCESS_PATHS.with(|paths| paths.borrow().get(&pid).cloned()) {
        return path;
    }
    let path = process_path_uncached(pid);
    PROCESS_PATHS.with(|paths| paths.borrow_mut().insert(pid, path.clone()));
    path
}

/// [`process_path`] without the UI thread's cache, for other threads.
pub fn process_path_uncached(pid: u32) -> String {
    // SAFETY: opens the process for a name query only, and closes it.
    unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .ok()
            .and_then(|process| {
                let mut buffer = vec![0u16; 1024];
                let mut size = buffer.len() as u32;
                let read = QueryFullProcessImageNameW(
                    process,
                    PROCESS_NAME_WIN32,
                    PWSTR(buffer.as_mut_ptr()),
                    &mut size,
                );
                let _ = CloseHandle(process);
                read.ok()
                    .map(|()| String::from_utf16_lossy(&buffer[..size as usize]))
            })
            .unwrap_or_default()
    }
}

/// Every app window, front-most first.
pub fn app_windows() -> Vec<AppWindow> {
    let own_pid = own_process_id();
    let mut handles: Vec<HWND> = Vec::new();
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `Vec` passed below, alive for the call.
        let handles = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        handles.push(hwnd);
        BOOL(1)
    }
    // SAFETY: the callback only pushes into `handles`.
    let _ = unsafe {
        EnumWindows(
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HWND> as isize),
        )
    };
    let windows = handles
        .into_iter()
        .filter(|&hwnd| is_app_window(hwnd, own_pid))
        .map(|hwnd| {
            let pid = process_id(hwnd);
            let exe_path = process_path(pid);
            let aumid = if apps::lulo_app_for_exe(&apps::exe_key(&exe_path)).is_some() {
                None
            } else {
                window_aumid(hwnd)
            };
            AppWindow {
                hwnd: hwnd.0 as isize,
                pid,
                exe_path,
                title: title(hwnd),
                // SAFETY: reads a property of a live window.
                minimized: unsafe { IsIconic(hwnd) }.as_bool(),
                aumid,
            }
        })
        .collect::<Vec<_>>();
    // Forget processes and windows that are gone, so a reused id is
    // looked up afresh.
    PROCESS_PATHS.with(|paths| {
        paths
            .borrow_mut()
            .retain(|pid, _| windows.iter().any(|window| window.pid == *pid))
    });
    AUMIDS.with(|ids| {
        ids.borrow_mut()
            .retain(|hwnd, _| windows.iter().any(|window| window.hwnd == *hwnd))
    });
    windows
}

thread_local! {
    static AUMIDS: RefCell<HashMap<isize, Option<String>>> = RefCell::new(HashMap::new());
}

/// The AppUserModelID set on `hwnd` itself (`System.AppUserModel.ID` in
/// its property store), as the taskbar reads it to group windows. Read
/// once per window.
fn window_aumid(hwnd: HWND) -> Option<String> {
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::System::Com::StructuredStorage::{
        PropVariantClear, PropVariantToStringAlloc,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, SHGetPropertyStoreForWindow,
    };
    // PKEY_AppUserModel_ID.
    const KEY: PROPERTYKEY = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };
    let key = hwnd.0 as isize;
    if let Some(known) = AUMIDS.with(|ids| ids.borrow().get(&key).cloned()) {
        return known;
    }
    // SAFETY: reads a property of a live window through the shell's own
    // per-window property store; the string is freed after copying.
    let aumid = unsafe {
        SHGetPropertyStoreForWindow::<IPropertyStore>(hwnd)
            .ok()
            .and_then(|store| store.GetValue(&KEY).ok())
            .and_then(|mut value| {
                let text = PropVariantToStringAlloc(&value).ok();
                let _ = PropVariantClear(&mut value);
                text
            })
            .map(|text| {
                let copied = String::from_utf16_lossy(text.as_wide());
                CoTaskMemFree(Some(text.0 as *const core::ffi::c_void));
                copied
            })
            .filter(|text| !text.is_empty())
    };
    AUMIDS.with(|ids| ids.borrow_mut().insert(key, aumid.clone()));
    aumid
}

/// The apps that have windows, in front-to-back order of their front-most
/// window. Store apps all run in `ApplicationFrameHost.exe`, so each is
/// told apart by its window title. `pipe_app_id` names the Lulo app a
/// process said it is over the menu pipe.
pub fn running<'a>(
    windows: &[AppWindow],
    pipe_app_id: impl Fn(u32) -> Option<&'a str>,
) -> Vec<Running> {
    let mut running: Vec<Running> = Vec::new();
    for window in windows {
        let key = apps::exe_key(&window.exe_path);
        if key.is_empty() {
            continue;
        }
        let lulo = apps::identify(&key, pipe_app_id(window.pid), window.aumid.as_deref());
        if let Some(app) = lulo {
            // A Lulo app is one tile whatever its executable is called.
            let key = app.exe.to_owned();
            match running.iter_mut().find(|known| known.key == key) {
                Some(known) => known.windows.push(window.hwnd),
                None => running.push(Running {
                    key,
                    exe_path: window.exe_path.clone(),
                    name: app.name.to_owned(),
                    windows: vec![window.hwnd],
                    lulo: Some(app),
                }),
            }
            continue;
        }
        let (key, name) = if key == "applicationframehost.exe" {
            (
                format!("store:{}", window.title.to_lowercase()),
                window.title.clone(),
            )
        } else {
            (key, app_name(&window.exe_path))
        };
        match running.iter_mut().find(|app| app.key == key) {
            Some(app) => app.windows.push(window.hwnd),
            None => running.push(Running {
                key,
                exe_path: window.exe_path.clone(),
                name,
                windows: vec![window.hwnd],
                lulo: None,
            }),
        }
    }
    running
}

/// The app's name as the menu bar shows it: a Lulo app's own name, else
/// the executable's description ("Notepad"), else its file name.
pub fn app_name(exe_path: &str) -> String {
    let key = apps::exe_key(exe_path);
    if let Some(app) = apps::lulo_app_for_exe(&key) {
        return app.name.to_owned();
    }
    let description = DESCRIPTIONS.with(|cache| {
        cache
            .borrow_mut()
            .entry(exe_path.to_owned())
            .or_insert_with(|| file_description(exe_path))
            .clone()
    });
    apps::display_name(&key, description.as_deref())
}

/// `FileDescription` from the executable's version resource.
fn file_description(path: &str) -> Option<String> {
    let path = HSTRING::from(path);
    // SAFETY: the version APIs fill the buffer sized by the first call; the
    // pointers VerQueryValueW returns point into that buffer.
    unsafe {
        let size = GetFileVersionInfoSizeW(&path, None);
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        GetFileVersionInfoW(&path, None, size, data.as_mut_ptr().cast()).ok()?;
        let mut pointer: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut length = 0u32;
        if !VerQueryValueW(
            data.as_ptr().cast(),
            w!("\\VarFileInfo\\Translation"),
            &mut pointer,
            &mut length,
        )
        .as_bool()
            || length < 4
            || pointer.is_null()
        {
            return None;
        }
        let language = *(pointer as *const u16);
        let code_page = *(pointer as *const u16).add(1);
        let query = HSTRING::from(format!(
            "\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription"
        ));
        if !VerQueryValueW(data.as_ptr().cast(), &query, &mut pointer, &mut length).as_bool()
            || length == 0
            || pointer.is_null()
        {
            return None;
        }
        let text = std::slice::from_raw_parts(pointer as *const u16, length as usize);
        Some(from_wide(text))
    }
}

pub fn foreground() -> isize {
    // SAFETY: no arguments.
    unsafe { GetForegroundWindow() }.0 as isize
}

/// Bring `hwnd` to the front, restoring it if it is minimised. The user
/// asked for it (a click on the Dock, a key in Spotlight, a launch they
/// started), so when Windows refuses the plain request because another
/// app holds the foreground, the shell joins that app's input queue for
/// the moment of the switch, as Alt+Tab does, and lets go at once.
pub fn activate(hwnd: isize) {
    let hwnd = handle(hwnd);
    // SAFETY: acts on a window another app owns, as Alt+Tab does; the
    // user's click just reached this process, so the foreground may move.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd {
            return;
        }
        let front = GetForegroundWindow();
        let front_thread = GetWindowThreadProcessId(front, None);
        let own_thread = GetCurrentThreadId();
        let attached = front_thread != 0
            && front_thread != own_thread
            && AttachThreadInput(own_thread, front_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(own_thread, front_thread, false);
        }
    }
}

pub fn minimize(hwnd: isize) {
    // SAFETY: as above.
    let _ = unsafe { ShowWindow(handle(hwnd), SW_MINIMIZE) };
}

/// The Mac's Zoom: maximise, or restore a maximised window.
pub fn zoom(hwnd: isize) {
    let hwnd = handle(hwnd);
    // SAFETY: as above.
    unsafe {
        let command = if IsZoomed(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_MAXIMIZE
        };
        let _ = ShowWindow(hwnd, command);
    }
}

/// Ask a window to close, as its close button does; the app may ask to
/// save first.
pub fn close(hwnd: isize) {
    // SAFETY: posts WM_CLOSE; the app decides what to do with it.
    let _ = unsafe { PostMessageW(Some(handle(hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
}

/// The window's visible bounds (without the invisible resize border).
pub fn frame_bounds(hwnd: isize) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: a RECT-sized out-parameter.
    unsafe {
        DwmGetWindowAttribute(
            handle(hwnd),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    Some(rect)
}
