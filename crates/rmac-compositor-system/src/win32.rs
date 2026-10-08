//! The desktop's windows on Windows, as `rmac-compositor` describes a
//! session: one output per monitor, one workspace per output (plus the
//! hidden parking workspace, which holds minimised windows as niri's does),
//! the windows Alt+Tab lists, and which one has the keyboard.
//!
//! Nothing polls. One thread owns out-of-context WinEvent hooks and blocks
//! in `GetMessageW`; a hook only wakes the watchers, which read the window
//! list again on a blocking thread and publish a snapshot when it changed.
//! A burst of events before a watcher reads folds into one read.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use async_channel::{Receiver, Sender};
use rmac_compositor as domain;
use windows::core::{w, BOOL, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess,
    QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, BringWindowToTop, DispatchMessageW, EnumWindows, GetAncestor,
    GetClassNameW, GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    IsZoomed, PostMessageW, SetForegroundWindow, SetWindowPos, ShowWindowAsync, TranslateMessage,
    ASFW_ANY, CHILDID_SELF, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_SHOW,
    EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND,
    EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MOVESIZEEND, GA_ROOT, GWL_EXSTYLE, GW_OWNER,
    MONITORINFOF_PRIMARY, MSG, OBJID_WINDOW, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WINEVENT_OUTOFCONTEXT, WM_CLOSE, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
};

/// The workspace holding minimised windows (`rmac_compositor::PARKING_WORKSPACE`).
const PARKING: domain::WorkspaceId = domain::WorkspaceId(0xFFFF);

/// Explorer's own desktop and taskbar windows are not app windows.
const SHELL_CLASSES: [&str; 5] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Windows.UI.Core.CoreWindow",
];

/// Why the window list cannot be followed.
#[derive(Debug)]
pub enum Error {
    /// The WinEvent hooks could not be installed.
    Hooks(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hooks(error) => write!(formatter, "window event hooks failed: {error}"),
        }
    }
}

impl std::error::Error for Error {}

/// Windows always has its window list; nothing is ever merely unreachable.
pub fn is_unavailable(_error: &Error) -> bool {
    false
}

/// Why a window could not be minimised.
#[derive(Debug)]
pub enum MinimizeError {
    Action(domain::ActionError),
}

impl fmt::Display for MinimizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Action(error) => write!(formatter, "could not minimise the window: {error:?}"),
        }
    }
}

impl std::error::Error for MinimizeError {}

/// niri asks the shell to minimise a window through its event stream;
/// Windows minimises windows itself, so no event is such a request.
pub fn minimize_request(_event: &domain::Event) -> Option<domain::WindowId> {
    None
}

/// Minimise `window` (Windows keeps its own picture of it for the taskbar;
/// the Dock shows the app's icon on its minimised tile).
pub async fn minimize_window_in(
    snapshot: domain::Snapshot,
    window: domain::WindowId,
) -> Result<(), MinimizeError> {
    if !snapshot
        .windows
        .iter()
        .any(|candidate| candidate.id == window)
    {
        return Ok(());
    }
    execute_action(&domain::Action::MinimizeWindow { window })
        .await
        .map_err(MinimizeError::Action)
}

pub fn action_capabilities() -> domain::ActionCapabilities {
    domain::ActionCapabilities {
        supported: vec![
            domain::ActionKind::Spawn,
            domain::ActionKind::FocusWindow,
            domain::ActionKind::FocusWorkspace,
            domain::ActionKind::FocusOutput,
            domain::ActionKind::CloseWindow,
            domain::ActionKind::FullscreenWindow,
            domain::ActionKind::FillWindow,
            domain::ActionKind::CenterWindow,
            domain::ActionKind::TileWindow,
            domain::ActionKind::SetWindowFrame,
            domain::ActionKind::MinimizeWindow,
            domain::ActionKind::RestoreWindow,
            domain::ActionKind::MoveWindowBy,
        ],
    }
}

/// One coherent reading of the desktop.
pub async fn snapshot() -> Result<domain::Snapshot, Error> {
    Ok(blocking::unblock(read_shared).await)
}

/// Publish a snapshot now and again whenever the windows change, until
/// `sender` closes.
pub async fn watch(sender: Sender<domain::Event>) -> Result<(), Error> {
    rmac_apps::windows_apps::on_process_apps_changed(refresh);
    let changes = subscribe()?;
    if sender
        .send(domain::Event::ConnectionChanged {
            state: domain::ConnectionState::Connected,
        })
        .await
        .is_err()
    {
        return Ok(());
    }
    let mut published: Option<domain::Snapshot> = None;
    loop {
        let snapshot = blocking::unblock(read_shared).await;
        if published.as_ref() != Some(&snapshot) {
            published = Some(snapshot.clone());
            if sender
                .send(domain::Event::Snapshot { snapshot })
                .await
                .is_err()
            {
                return Ok(());
            }
        }
        let changed = futures_util::FutureExt::fuse(changes.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(changed, closed);
        futures_util::select! {
            changed = changed => if changed.is_err() { return Ok(()) },
            _ = closed => return Ok(()),
        }
    }
}

/// Read the windows again in every watcher now: the shell itself changed
/// something no WinEvent reports (a launch it started).
pub fn refresh() {
    notify_watchers();
}

pub async fn execute(request: domain::ActionRequest) -> domain::ActionResult {
    let result = execute_action(&request.action).await;
    domain::ActionResult {
        id: request.id,
        action: request.action,
        result,
    }
}

pub async fn execute_action(action: &domain::Action) -> Result<(), domain::ActionError> {
    let action = action.clone();
    let result = blocking::unblock(move || run_action(&action)).await;
    notify_watchers();
    result
}

// ---------------------------------------------------------------------------
// Watching

static SUBSCRIBERS: Mutex<Vec<Sender<()>>> = Mutex::new(Vec::new());
/// Counts window events; a snapshot read since the last one is reused, so
/// the bar, the Dock and the desktop following one change read the windows
/// once between them.
static CHANGES: AtomicU64 = AtomicU64::new(1);
static LAST_READ: Mutex<Option<(u64, domain::Snapshot)>> = Mutex::new(None);

fn read_shared() -> domain::Snapshot {
    // Without the hooks nothing would say a reading went stale.
    if !matches!(HOOKS.get_or_init(start_hooks), Ok(())) {
        return read_snapshot();
    }
    let generation = CHANGES.load(Ordering::Acquire);
    if let Some((read_at, snapshot)) = LAST_READ.lock().ok().and_then(|last| last.clone()) {
        if read_at == generation {
            return snapshot;
        }
    }
    let snapshot = read_snapshot();
    if let Ok(mut last) = LAST_READ.lock() {
        *last = Some((generation, snapshot.clone()));
    }
    snapshot
}
static HOOKS: OnceLock<Result<(), String>> = OnceLock::new();

fn subscribe() -> Result<Receiver<()>, Error> {
    HOOKS
        .get_or_init(start_hooks)
        .clone()
        .map_err(Error::Hooks)?;
    // One slot: a change arriving while one is already waiting folds into it.
    let (sender, receiver) = async_channel::bounded(1);
    if let Ok(mut subscribers) = SUBSCRIBERS.lock() {
        subscribers.push(sender);
    }
    Ok(receiver)
}

fn notify_watchers() {
    CHANGES.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut subscribers) = SUBSCRIBERS.lock() {
        subscribers.retain(|subscriber| !subscriber.is_closed());
        for subscriber in subscribers.iter() {
            let _ = subscriber.try_send(());
        }
    }
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    if object != OBJID_WINDOW.0 || child != CHILDID_SELF as i32 || hwnd.is_invalid() {
        return;
    }
    // Only top-level windows are apps' windows; the foreground always is.
    // SAFETY: reads a property of the window the event names.
    if event != EVENT_SYSTEM_FOREGROUND && unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return;
    }
    if matches!(event, EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED) {
        bring_launch_forward(hwnd);
    }
    notify_watchers();
}

/// Apps the user just started (a Dock tile, Spotlight, a desktop icon),
/// whose first window is brought to the front when it appears, and when.
static LAUNCHES: std::sync::Mutex<Vec<(u32, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());
/// How long a started app's first window may take to appear.
const LAUNCH_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);

fn launched(pid: u32) {
    if let Ok(mut launches) = LAUNCHES.lock() {
        launches.retain(|(_, started)| started.elapsed() < LAUNCH_WINDOW);
        launches.push((pid, std::time::Instant::now()));
    }
}

/// The first window of an app the user just started comes to the front,
/// as a launched Mac app's does, even over a window that held the
/// foreground meanwhile (Windows' foreground lock would leave it behind:
/// WIN-OS-45).
fn bring_launch_forward(hwnd: HWND) {
    let pid = process_id(hwnd);
    let pending = LAUNCHES.lock().is_ok_and(|mut launches| {
        launches.retain(|(_, started)| started.elapsed() < LAUNCH_WINDOW);
        launches.iter().any(|(launched, _)| *launched == pid)
    });
    // SAFETY: no arguments.
    if !pending || !is_app_window(hwnd, unsafe { GetCurrentProcessId() }) {
        return;
    }
    if let Ok(mut launches) = LAUNCHES.lock() {
        launches.retain(|(launched, _)| *launched != pid);
    }
    activate(hwnd);
    if std::env::var_os("LULO_SHELL_TRACE").is_some_and(|value| value == "1") {
        // SAFETY: no arguments.
        let front = unsafe { GetForegroundWindow() } == hwnd;
        let exe = rmac_apps::windows_apps::exe_key(&process_path(pid));
        eprintln!("lulo-shell: launched {exe} in front: {front}");
    }
}

fn start_hooks() -> Result<(), String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::Builder::new()
        .name("lulo-window-events".into())
        .spawn(move || {
            let ranges = [
                (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
                (EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZEEND),
                (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
                (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
                (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
            ];
            let mut installed = 0;
            for (first, last) in ranges {
                // SAFETY: an out-of-context hook whose callback only reads
                // window properties and wakes channels; it runs on this
                // thread's message loop below.
                let hook = unsafe {
                    SetWinEventHook(
                        first,
                        last,
                        None,
                        Some(on_event),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                };
                if !hook.is_invalid() {
                    installed += 1;
                }
            }
            if installed == 0 {
                let _ = ready_tx.send(Err("SetWinEventHook refused every hook".into()));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            let mut message = MSG::default();
            // SAFETY: a plain message loop on this thread, which owns the
            // hooks; it blocks while nothing happens.
            unsafe {
                while GetMessageW(&mut message, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        })
        .map_err(|error| error.to_string())?;
    ready_rx
        .recv()
        .unwrap_or_else(|_| Err("the window event thread stopped".into()))
}

// ---------------------------------------------------------------------------
// Reading

fn handle(id: domain::WindowId) -> HWND {
    HWND(id.0 as isize as *mut core::ffi::c_void)
}

fn window_id(hwnd: HWND) -> domain::WindowId {
    domain::WindowId(hwnd.0 as isize as u64)
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 128];
    // SAFETY: the buffer is writable and its length is passed.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

fn title(hwnd: HWND) -> String {
    // SAFETY: reads the caption Windows keeps for the window; for another
    // process's window this sends no message.
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    // SAFETY: as above.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

fn process_id(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: plain out-parameter.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// Whether Alt+Tab would show `hwnd`: visible, top-level, not a tool
/// window, not owned (unless it asks to be shown), not cloaked by DWM (a
/// suspended Store app or a window on another virtual desktop), titled, and
/// not Explorer's desktop or taskbar or one of the shell's own surfaces.
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
    !SHELL_CLASSES.contains(&class_name(hwnd).as_str())
}

/// Executable paths by process, kept while the process has windows.
static PROCESS_PATHS: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);
/// AppUserModelIDs by window, kept while the window lives.
static AUMIDS: Mutex<Option<HashMap<isize, Option<String>>>> = Mutex::new(None);
/// Executable descriptions by path.
static DESCRIPTIONS: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

fn cached<K: std::hash::Hash + Eq, V: Clone>(
    cache: &Mutex<Option<HashMap<K, V>>>,
    key: K,
    read: impl FnOnce() -> V,
) -> V {
    if let Some(value) = cache
        .lock()
        .ok()
        .and_then(|cache| cache.as_ref().and_then(|cache| cache.get(&key).cloned()))
    {
        return value;
    }
    let value = read();
    if let Ok(mut cache) = cache.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(key, value.clone());
    }
    value
}

fn process_path(pid: u32) -> String {
    cached(&PROCESS_PATHS, pid, || {
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
    })
}

thread_local! {
    static COM_READY: bool = {
        // SAFETY: joins this blocking thread to the multithreaded apartment
        // for the shell's per-window property store; it is never left.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok()
    };
}

/// The AppUserModelID set on `hwnd` itself, as the taskbar reads it to
/// group windows. Read once per window.
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
    cached(&AUMIDS, hwnd.0 as isize, || {
        COM_READY.with(|_| ());
        // SAFETY: reads a property of a live window through the shell's own
        // per-window property store; the string is freed after copying.
        unsafe {
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
        }
    })
}

/// `FileDescription` from the executable's version resource.
fn file_description(path: &str) -> Option<String> {
    cached(&DESCRIPTIONS, path.to_owned(), || {
        let path = HSTRING::from(path);
        // SAFETY: the version APIs fill the buffer sized by the first call;
        // the pointers VerQueryValueW returns point into that buffer.
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
            let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
            Some(String::from_utf16_lossy(&text[..end]))
        }
    })
}

struct Monitor {
    handle: isize,
    id: domain::OutputId,
    rect: RECT,
    scale: f64,
    primary: bool,
}

fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        // SAFETY: `lparam` is the `Vec` passed below, alive for the call.
        let monitors = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
        monitors.push(monitor);
        BOOL(1)
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only pushes into `handles`.
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        )
    };
    handles
        .into_iter()
        .filter_map(|monitor| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: a MONITORINFOEXW whose size field says so.
            unsafe {
                GetMonitorInfoW(
                    monitor,
                    &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
                )
            }
            .as_bool()
            .then(|| {
                let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
                // SAFETY: plain out-parameters.
                let _ =
                    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
                let end = info
                    .szDevice
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(info.szDevice.len());
                Monitor {
                    handle: monitor.0 as isize,
                    id: domain::OutputId(String::from_utf16_lossy(&info.szDevice[..end])),
                    rect: info.monitorInfo.rcMonitor,
                    scale: f64::from(dpi_x.max(1)) / 96.0,
                    primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                }
            })
        })
        .collect()
}

fn monitor_of(hwnd: HWND, monitors: &[Monitor]) -> Option<usize> {
    // SAFETY: reads the monitor a live window is on.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    monitors
        .iter()
        .position(|candidate| candidate.handle == monitor.0 as isize)
        .or_else(|| monitors.iter().position(|candidate| candidate.primary))
}

/// The window's visible bounds (without the invisible resize border).
fn frame_bounds(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: a RECT-sized out-parameter.
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    Some(rect)
}

fn top_level_windows() -> Vec<HWND> {
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
    handles
}

fn workspace_for(index: usize) -> domain::WorkspaceId {
    domain::WorkspaceId(index as u64 + 1)
}

fn read_snapshot() -> domain::Snapshot {
    // SAFETY: no arguments.
    let own_pid = unsafe { GetCurrentProcessId() };
    let monitors = monitors();
    // SAFETY: no arguments.
    let foreground = unsafe { GetForegroundWindow() };
    let handles = top_level_windows();
    let count = handles.len() as u64;
    let mut windows = Vec::new();
    let mut alive_pids = Vec::new();
    for (rank, hwnd) in handles.into_iter().enumerate() {
        if !is_app_window(hwnd, own_pid) {
            continue;
        }
        let pid = process_id(hwnd);
        alive_pids.push(pid);
        let exe_path = process_path(pid);
        let exe_key = rmac_apps::windows_apps::exe_key(&exe_path);
        let aumid = if rmac_apps::windows_apps::app_for_exe(&exe_key).is_some() {
            None
        } else {
            window_aumid(hwnd)
        };
        // A Lulo app that linked its menus says which app it is, whatever
        // its executable is called.
        let app_id = rmac_apps::windows_apps::process_app(pid).unwrap_or_else(|| {
            rmac_apps::windows_apps::window_app_id(&exe_path, aumid.as_deref())
        });
        if rmac_apps::windows_apps::app(&app_id).is_none()
            && rmac_apps::windows_apps::display_name_for(&app_id).is_none()
        {
            let name = if app_id == exe_key || exe_key.is_empty() {
                rmac_apps::windows_apps::display_name(
                    &exe_key,
                    file_description(&exe_path).as_deref(),
                )
            } else {
                // A Store app: its window's title is its name.
                title(hwnd)
            };
            rmac_apps::windows_apps::register_display_name(&app_id, &name);
        }
        let monitor = monitor_of(hwnd, &monitors);
        // SAFETY: reads a property of a live window.
        let minimized = unsafe { IsIconic(hwnd) }.as_bool();
        let scale = monitor.map_or(1.0, |index| monitors[index].scale);
        let origin = monitor.map_or((0, 0), |index| {
            (monitors[index].rect.left, monitors[index].rect.top)
        });
        let frame = frame_bounds(hwnd).unwrap_or_default();
        let width = (frame.right - frame.left).max(0);
        let height = (frame.bottom - frame.top).max(0);
        windows.push(domain::Window {
            id: window_id(hwnd),
            title: Some(title(hwnd)),
            app_id: Some(app_id),
            pid: i32::try_from(pid).ok(),
            workspace: Some(if minimized {
                PARKING
            } else {
                workspace_for(monitor.unwrap_or(0))
            }),
            focused: hwnd == foreground,
            floating: true,
            urgent: false,
            // Front-most is the most recently focused.
            focus_timestamp: Some(domain::Timestamp {
                seconds: count - rank as u64,
                nanoseconds: 0,
            }),
            layout: domain::WindowLayout {
                scrolling_position: None,
                tile_size: domain::LogicalSize {
                    width: f64::from(width) / scale,
                    height: f64::from(height) / scale,
                },
                tile_position_in_view: Some(domain::LogicalPoint {
                    x: f64::from(frame.left - origin.0) / scale,
                    y: f64::from(frame.top - origin.1) / scale,
                }),
                window_size: domain::PhysicalSize {
                    width: width as u32,
                    height: height as u32,
                },
                window_offset_in_tile: domain::LogicalPoint::default(),
            },
        });
    }
    prune_caches(&alive_pids, &windows);

    let outputs = monitors
        .iter()
        .map(|monitor| {
            let width = (monitor.rect.right - monitor.rect.left).max(0) as u32;
            let height = (monitor.rect.bottom - monitor.rect.top).max(0) as u32;
            domain::Output {
                id: monitor.id.clone(),
                make: String::new(),
                model: String::new(),
                serial: None,
                physical_size_mm: None,
                modes: vec![domain::OutputMode {
                    physical_size: domain::PhysicalSize { width, height },
                    refresh_millihz: 60_000,
                    preferred: true,
                }],
                current_mode: Some(0),
                custom_mode: false,
                vrr_supported: false,
                vrr_enabled: false,
                logical: Some(domain::LogicalOutput {
                    position: domain::LogicalPoint {
                        x: f64::from(monitor.rect.left) / monitor.scale,
                        y: f64::from(monitor.rect.top) / monitor.scale,
                    },
                    size: domain::LogicalSize {
                        width: f64::from(width) / monitor.scale,
                        height: f64::from(height) / monitor.scale,
                    },
                    scale: monitor.scale,
                    transform: "normal".into(),
                }),
            }
        })
        .collect::<Vec<_>>();

    let foreground_window = windows
        .iter()
        .find(|window| window.focused)
        .map(|window| (window.id, window.workspace));
    let focused_monitor = if foreground.is_invalid() {
        None
    } else {
        monitor_of(foreground, &monitors)
    }
    .or_else(|| monitors.iter().position(|monitor| monitor.primary));
    let mut workspaces = monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| {
            let id = workspace_for(index);
            domain::Workspace {
                id,
                index: 1,
                name: None,
                output: Some(monitor.id.clone()),
                urgent: false,
                active: true,
                focused: Some(index) == focused_monitor,
                active_window: windows
                    .iter()
                    .find(|window| window.workspace == Some(id))
                    .map(|window| window.id),
            }
        })
        .collect::<Vec<_>>();
    workspaces.push(domain::Workspace {
        id: PARKING,
        index: 2,
        name: Some(domain::PARKING_WORKSPACE.to_owned()),
        output: monitors
            .iter()
            .find(|monitor| monitor.primary)
            .or(monitors.first())
            .map(|monitor| monitor.id.clone()),
        urgent: false,
        active: false,
        focused: false,
        active_window: None,
    });

    let focus = match foreground_window {
        Some((window, workspace)) => domain::FocusState {
            target: Some(domain::FocusTarget::Window(window)),
            output: focused_monitor.map(|index| monitors[index].id.clone()),
            workspace,
            window: Some(window),
        },
        // One of the shell's own surfaces (the bar with a menu open, the
        // desktop) has the keyboard, as a layer surface does on niri.
        None if !foreground.is_invalid() && process_id(foreground) == own_pid => {
            domain::FocusState {
                target: Some(domain::FocusTarget::LayerSurface(domain::LayerSurfaceId(
                    "lulo-shell".into(),
                ))),
                output: focused_monitor.map(|index| monitors[index].id.clone()),
                workspace: None,
                window: None,
            }
        }
        None => domain::FocusState {
            target: None,
            output: focused_monitor.map(|index| monitors[index].id.clone()),
            workspace: None,
            window: None,
        },
    };

    domain::Snapshot {
        outputs,
        workspaces,
        windows,
        layer_surfaces: Vec::new(),
        focus,
        activation: None,
        overview_visible: false,
    }
}

fn prune_caches(alive_pids: &[u32], windows: &[domain::Window]) {
    if let Ok(mut paths) = PROCESS_PATHS.lock() {
        if let Some(paths) = paths.as_mut() {
            paths.retain(|pid, _| alive_pids.contains(pid));
        }
    }
    if let Ok(mut ids) = AUMIDS.lock() {
        if let Some(ids) = ids.as_mut() {
            ids.retain(|hwnd, _| windows.iter().any(|window| window.id.0 == *hwnd as u64));
        }
    }
}

// ---------------------------------------------------------------------------
// Acting

fn rejected(message: impl Into<String>) -> domain::ActionError {
    domain::ActionError {
        kind: domain::ActionErrorKind::Rejected,
        message: message.into(),
    }
}

fn unsupported(kind: domain::ActionKind) -> domain::ActionError {
    domain::ActionError {
        kind: domain::ActionErrorKind::Unsupported,
        message: format!("{kind:?} has no Windows equivalent"),
    }
}

fn run_action(action: &domain::Action) -> Result<(), domain::ActionError> {
    match action {
        domain::Action::Spawn { command } => spawn(command.arguments()),
        domain::Action::FocusWindow { window } => {
            activate(handle(*window));
            Ok(())
        }
        // One workspace per output, always shown.
        domain::Action::FocusWorkspace { .. } | domain::Action::FocusOutput { .. } => Ok(()),
        domain::Action::CloseWindow { window } => {
            // SAFETY: posts WM_CLOSE, as the window's close button does; the
            // app decides what to do with it (it may ask to save).
            unsafe { PostMessageW(Some(handle(*window)), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                .map_err(|error| rejected(error.to_string()))
        }
        domain::Action::MinimizeWindow { window } => {
            show(handle(*window), SW_MINIMIZE);
            Ok(())
        }
        domain::Action::RestoreWindow { window, .. } => {
            activate(handle(*window));
            Ok(())
        }
        domain::Action::FullscreenWindow { window, on } => {
            show(handle(*window), if *on { SW_MAXIMIZE } else { SW_RESTORE });
            Ok(())
        }
        domain::Action::FillWindow { window } => {
            let hwnd = handle(*window);
            // SAFETY: reads a property of a live window.
            let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
            show(hwnd, if zoomed { SW_RESTORE } else { SW_MAXIMIZE });
            Ok(())
        }
        domain::Action::CenterWindow { window } => {
            let hwnd = handle(*window);
            let (work, _) = work_area(hwnd).ok_or_else(|| rejected("no monitor"))?;
            let frame = frame_bounds(hwnd).ok_or_else(|| rejected("no frame"))?;
            let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
            let left = work.left + ((work.right - work.left) - width) / 2;
            let top = work.top + ((work.bottom - work.top) - height) / 2;
            place_frame(
                hwnd,
                RECT {
                    left,
                    top,
                    right: left + width,
                    bottom: top + height,
                },
            );
            Ok(())
        }
        domain::Action::TileWindow { window, region } => {
            let (x, y, width, height) = region.frame_percent();
            frame_percent(handle(*window), x, y, width, height)
        }
        domain::Action::SetWindowFrame {
            window,
            x,
            y,
            width,
            height,
        } => frame_percent(handle(*window), x.0, y.0, width.0, height.0),
        domain::Action::MoveWindowBy { window, dx, dy } => {
            let hwnd = handle(*window);
            let (_, scale) = work_area(hwnd).ok_or_else(|| rejected("no monitor"))?;
            let frame = frame_bounds(hwnd).ok_or_else(|| rejected("no frame"))?;
            let (dx, dy) = ((dx.0 * scale).round() as i32, (dy.0 * scale).round() as i32);
            place_frame(
                hwnd,
                RECT {
                    left: frame.left + dx,
                    top: frame.top + dy,
                    right: frame.right + dx,
                    bottom: frame.bottom + dy,
                },
            );
            Ok(())
        }
        other => Err(unsupported(other.kind())),
    }
}

fn spawn(arguments: &[String]) -> Result<(), domain::ActionError> {
    let (program, rest) = arguments
        .split_first()
        .ok_or_else(|| rejected("nothing to start"))?;
    // The user asked for this app: let its first window come to the front.
    // SAFETY: no pointers.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let child = std::process::Command::new(program)
        .args(rest)
        .spawn()
        .map_err(|error| rejected(format!("could not start {program}: {error}")))?;
    // SAFETY: no pointers.
    let _ = unsafe { AllowSetForegroundWindow(child.id()) };
    launched(child.id());
    Ok(())
}

/// `ShowWindow` without waiting on the window's own thread, which may be
/// busy (another app's window).
fn show(hwnd: HWND, command: windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD) {
    // SAFETY: posts a show command to a window another app may own.
    let _ = unsafe { ShowWindowAsync(hwnd, command) };
}

/// Bring `hwnd` to the front, restoring it if it is minimised. The user
/// asked for it (a click in the Dock or the Window menu), so when Windows
/// refuses the plain request because another app holds the foreground, the
/// shell joins that app's input queue for the moment of the switch, as
/// Alt+Tab does, and lets go at once.
fn activate(hwnd: HWND) {
    // SAFETY: acts on a window another app owns, as Alt+Tab does.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
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

/// The work area of the monitor `hwnd` is on (physical pixels) and its scale.
fn work_area(hwnd: HWND) -> Option<(RECT, f64)> {
    // SAFETY: reads the monitor a live window is on.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: a MONITORINFO whose size field says so.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: plain out-parameters.
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    Some((info.rcWork, f64::from(dpi_x.max(1)) / 96.0))
}

/// Put `hwnd`'s visible frame at `percent` of its monitor's work area.
fn frame_percent(
    hwnd: HWND,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), domain::ActionError> {
    let (work, _) = work_area(hwnd).ok_or_else(|| rejected("no monitor"))?;
    let (work_width, work_height) = (
        f64::from(work.right - work.left),
        f64::from(work.bottom - work.top),
    );
    let left = work.left + (work_width * x / 100.0).round() as i32;
    let top = work.top + (work_height * y / 100.0).round() as i32;
    place_frame(
        hwnd,
        RECT {
            left,
            top,
            right: left + (work_width * width / 100.0).round() as i32,
            bottom: top + (work_height * height / 100.0).round() as i32,
        },
    );
    Ok(())
}

/// Move `hwnd` so its visible frame is `target` (physical pixels): the
/// window rectangle is wider than the frame by its invisible resize border.
fn place_frame(hwnd: HWND, target: RECT) {
    // SAFETY: reads a property of a live window.
    if unsafe { IsZoomed(hwnd) }.as_bool() || unsafe { IsIconic(hwnd) }.as_bool() {
        show(hwnd, SW_RESTORE);
    }
    let mut window = RECT::default();
    // SAFETY: a RECT out-parameter.
    let _ = unsafe { GetWindowRect(hwnd, &mut window) };
    let frame = frame_bounds(hwnd).unwrap_or(window);
    let (left, top) = (frame.left - window.left, frame.top - window.top);
    let (right, bottom) = (window.right - frame.right, window.bottom - frame.bottom);
    // SAFETY: positions another app's window without waiting on its thread.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            target.left - left,
            target.top - top,
            (target.right - target.left) + left + right,
            (target.bottom - target.top) + top + bottom,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    };
}
