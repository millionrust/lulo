//! `lulo-session`: the Lulo layer's switch and watchdog.
//!
//! Started by the user (or at sign-in, when they chose that), it gives back
//! anything an earlier crash left changed, runs `lulo-shell`, and gives the
//! desktop back again whenever the shell stops: the taskbar's own setting,
//! its visibility and the AppBars' strips of the work area. A shell that
//! crashes is started again, at most three times a minute. The session
//! waits on the shell's process handle, so it uses no CPU while Lulo runs.
//!
//! `lulo-session --stop` turns the Lulo layer off, as the Lulo menu's Turn
//! Off Lulo does. `lulo-session --restore-windows-desktop` gives the
//! Windows desktop back (the "Restore Windows taskbar" escape hatch),
//! turning a running Lulo off first. `lulo-session --uninstall`, which the
//! installer runs before it removes Lulo, does that and also undoes the
//! user's own Lulo switches that point at Lulo's files (Use Files for
//! Folders, Start Lulo at Sign-In), so nothing is left pointing at a
//! removed app.
//!
//! `lulo-session --as-shell` is what Windows starts at sign-in when the user
//! turned on Start Lulo at Sign-In (their `Winlogon\Shell`; ADR 0023, "Lulo
//! as the shell"): Explorer never loads. The session then also starts the
//! user's startup apps once the layer is on screen (`startup`), and if the
//! shell stops by itself twice within two minutes it starts Explorer
//! instead, so the user is never left without a desktop. Turn Off Lulo
//! there starts Explorer and gives the next sign-in back to it.
//!
//! `lulo-session --after-install`, which the installer runs once it has
//! finished (packaging/windows/Product.wxs), brings Lulo back for the user:
//! after a fresh install, and after an upgrade unless they had turned Lulo
//! off themselves (`registry::lulo_mode`). An upgrade stops a running Lulo
//! with `--stop` first, which leaves Lulo mode as it was.

use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, OpenMutexW, ReleaseMutex, SetEvent,
    WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE, SYNCHRONIZATION_SYNCHRONIZE,
};

use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};

use super::{appbar, desktop, folders, handle, launch, registry, taskbar, user_name};

/// What `lulo-shell` exits with when another Lulo layer already runs.
const ALREADY_RUNNING: i32 = 3;
const RESTARTS_PER_MINUTE: usize = 3;

/// Give the desktop back: remove AppBars a crashed shell left registered
/// and restore the taskbar's recorded setting. Safe to repeat.
pub fn restore_windows_desktop() {
    for value in ["BarWindow", "DockWindow"] {
        if let Some(hwnd) = registry::get_dword(value) {
            // Windows looks the AppBar up by its handle, which is all a
            // dead window leaves behind.
            appbar::remove(handle(hwnd as isize));
            registry::delete_value(value);
        }
    }
    taskbar::restore();
    desktop::restore();
}

/// The event `lulo-session --stop` sets to turn the Lulo layer off.
fn stop_event_name() -> HSTRING {
    HSTRING::from(format!(r"Local\lulo-shell-stop-{}", user_name()))
}

/// Call `on_stop` (from a thread that blocks until then) when
/// `lulo-session --stop` asks the Lulo layer to turn off.
pub fn on_stop_request(on_stop: impl FnOnce() + Send + 'static) {
    // SAFETY: an auto-reset named event, kept for the life of the process.
    let Ok(event) = (unsafe { CreateEventW(None, false, false, &stop_event_name()) }) else {
        return;
    };
    let event = event.0 as usize;
    let _ = std::thread::Builder::new()
        .name("lulo-stop-request".into())
        .spawn(move || {
            let event = windows::Win32::Foundation::HANDLE(event as *mut core::ffi::c_void);
            // SAFETY: waits on the event created above.
            unsafe { WaitForSingleObject(event, INFINITE) };
            on_stop();
        });
}

/// The switch that asks the running shell to open Apps
/// (`lulo-shell.exe --app-drawer`, the Exec of Apps' desktop entry).
pub const APP_DRAWER_SWITCH: &str = "--app-drawer";

fn app_drawer_event_name() -> HSTRING {
    HSTRING::from(format!(r"Local\lulo-shell-apps-{}", user_name()))
}

/// Call `on_open` (from a thread that waits on a named event, so nothing
/// polls) each time `lulo-shell --app-drawer` asks for Apps.
pub fn on_app_drawer_request(on_open: impl Fn() + Send + 'static) {
    // SAFETY: an auto-reset named event, kept for the life of the process.
    let Ok(event) = (unsafe { CreateEventW(None, false, false, &app_drawer_event_name()) }) else {
        return;
    };
    let event = event.0 as usize;
    let _ = std::thread::Builder::new()
        .name("lulo-apps-request".into())
        .spawn(move || loop {
            let event = windows::Win32::Foundation::HANDLE(event as *mut core::ffi::c_void);
            // SAFETY: waits on the event created above.
            unsafe { WaitForSingleObject(event, INFINITE) };
            on_open();
        });
}

/// `lulo-shell --app-drawer`: ask the running shell to open Apps.
pub fn request_app_drawer() -> i32 {
    // SAFETY: opens the running shell's named event and sets it.
    match unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &app_drawer_event_name()) } {
        Ok(event) => {
            // SAFETY: as above; the handle is closed straight after.
            unsafe {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
            0
        }
        // No Lulo running.
        Err(_) => 1,
    }
}

fn request_stop() {
    // SAFETY: opens (or creates) the named event and sets it.
    if let Ok(event) = unsafe { CreateEventW(None, false, false, &stop_event_name()) } {
        // SAFETY: as above.
        let _ = unsafe { SetEvent(event) };
    }
}

fn session_mutex_name() -> HSTRING {
    HSTRING::from(format!(r"Local\lulo-session-{}", user_name()))
}

/// Ask a running Lulo to turn off, and wait (at most `seconds`) until its
/// session has stopped, so nothing hides the desktop again afterwards.
fn stop_and_wait(seconds: u32) {
    // SAFETY: opens the running session's mutex, if there is one, only to
    // wait on it; it is closed again below.
    let Ok(mutex) =
        (unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &session_mutex_name()) })
    else {
        // No Lulo running.
        return;
    };
    request_stop();
    // SAFETY: waits on the handle opened above. The session holds the
    // mutex for its whole life, so the wait ends when it exits (the mutex
    // is then abandoned or released) or after the time limit.
    let waited = unsafe { WaitForSingleObject(mutex, seconds * 1000) };
    // SAFETY: gives back the mutex if the wait took it, then closes it.
    unsafe {
        if waited.0 == 0 || waited.0 == 0x80 {
            let _ = ReleaseMutex(mutex);
        }
        let _ = CloseHandle(mutex);
    }
}

/// The event `lulo-shell` sets once its bar, Dock and desktop are on
/// screen (the session starts the startup apps then).
pub fn ready_event_name() -> HSTRING {
    HSTRING::from(format!(r"Local\lulo-shell-ready-{}", user_name()))
}

/// Say the layer is ready (`lulo-shell`, once).
pub fn signal_ready() {
    // SAFETY: opens (or creates) the named event and sets it; the handle
    // is closed straight after.
    if let Ok(event) = unsafe { CreateEventW(None, true, false, &ready_event_name()) } {
        // SAFETY: as above.
        unsafe {
            let _ = SetEvent(event);
            let _ = CloseHandle(event);
        }
    }
}

/// As the shell: start the startup apps once the layer is on screen (or
/// after a minute at most), on a thread of their own.
fn start_startup_apps_when_ready() {
    // SAFETY: a manual-reset named event, waited on by the thread below.
    let Ok(event) = (unsafe { CreateEventW(None, true, false, &ready_event_name()) }) else {
        return;
    };
    let event = event.0 as usize;
    let _ = std::thread::Builder::new()
        .name("lulo-startup-apps".into())
        .spawn(move || {
            let event = windows::Win32::Foundation::HANDLE(event as *mut core::ffi::c_void);
            // SAFETY: waits on, then closes, the event created above.
            unsafe {
                WaitForSingleObject(event, 60_000);
                let _ = CloseHandle(event);
            }
            super::startup::run_all();
        });
}

/// Explorer, as the desktop when Lulo cannot be (or is turned off as the
/// shell). With no shell window left, Explorer starts as the shell.
fn start_explorer() {
    // Signing out: nothing should start now.
    // SAFETY: no pointers.
    if unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) } != 0 {
        return;
    }
    let windows = std::env::var_os("WINDIR").map_or_else(
        || std::path::PathBuf::from(r"C:\Windows"),
        std::path::PathBuf::from,
    );
    if let Err(error) = std::process::Command::new(windows.join("explorer.exe")).spawn() {
        eprintln!("lulo-session: could not start Explorer: {error}");
    }
}

fn supervise(as_shell: bool) -> i32 {
    let name = session_mutex_name();
    // SAFETY: a named mutex held for the life of the process.
    let mutex = unsafe { CreateMutexW(None, true, &name) };
    // SAFETY: reads this thread's last error, set by the call above.
    if mutex.is_err() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // The Lulo layer is on already.
        return 0;
    }
    let _ = registry::set_lulo_mode(true);
    registry::record_session_start();
    restore_windows_desktop();
    let shell = launch::install_dir().join("lulo-shell.exe");
    if as_shell {
        start_startup_apps_when_ready();
    }
    let mut starts: Vec<Instant> = Vec::new();
    let mut watchdog = crate::model::startup::Watchdog::default();
    loop {
        starts.retain(|start| start.elapsed() < Duration::from_secs(60));
        if starts.len() >= RESTARTS_PER_MINUTE {
            eprintln!("lulo-session: Lulo stopped {RESTARTS_PER_MINUTE} times in a minute; the Windows desktop is back.");
            if as_shell {
                start_explorer();
            }
            return 1;
        }
        starts.push(Instant::now());
        let mut command = std::process::Command::new(&shell);
        if as_shell {
            command.env(AS_SHELL_ENV, "1");
        }
        let status = command.status();
        restore_windows_desktop();
        match status {
            Ok(status) if status.success() => {
                // Turned off by the user (not stopped for an upgrade):
                // Explorer now, and at the next sign-in.
                if as_shell && registry::lulo_mode() == Some(false) {
                    let _ = registry::set_starts_at_sign_in(None);
                    start_explorer();
                }
                return 0;
            }
            Ok(status) if status.code() == Some(ALREADY_RUNNING) => return 0,
            Ok(status) => {
                if as_shell && watchdog.failed(Instant::now()) {
                    eprintln!("lulo-session: Lulo stopped twice ({status}); starting Explorer.");
                    start_explorer();
                    return 1;
                }
                eprintln!("lulo-session: Lulo stopped ({status}); starting it again");
            }
            Err(error) => {
                eprintln!("lulo-session: cannot start {}: {error}", shell.display());
                if as_shell {
                    start_explorer();
                }
                return 1;
            }
        }
    }
}

/// What `lulo-session --as-shell` tells `lulo-shell`: it is the shell, so
/// it keeps the work area and the tray itself.
pub const AS_SHELL_ENV: &str = "LULO_AS_SHELL";

/// After an install or upgrade: start the Lulo layer in its own process,
/// detached from the installer (which ends straight after), unless the user
/// had turned Lulo off. Returns at once.
fn start_after_install() -> i32 {
    if registry::lulo_mode() == Some(false) {
        return 0;
    }
    let Ok(session) = std::env::current_exe() else {
        return 1;
    };
    // Lulo is the user's shell: back as the shell, with the tray and the
    // work area its own.
    let as_shell =
        registry::user_shell().is_some_and(|shell| crate::model::startup::is_lulo_shell(&shell));
    use std::os::windows::process::CommandExt as _;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    // Out of msiexec's job if it has one, so the layer outlives the
    // install; a job that forbids breaking away still lets it start.
    let arguments: &[&str] = if as_shell {
        &[crate::model::startup::AS_SHELL_SWITCH]
    } else {
        &[]
    };
    let started = std::process::Command::new(&session)
        .args(arguments)
        .creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
        .or_else(|_| {
            std::process::Command::new(&session)
                .args(arguments)
                .creation_flags(flags)
                .spawn()
        });
    match started {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("lulo-session: cannot start Lulo after installing: {error}");
            1
        }
    }
}

/// `lulo-session [--stop | --restore-windows-desktop | --uninstall | --after-install]`.
pub fn run(arguments: &[String]) -> i32 {
    match arguments.first().map(String::as_str) {
        None => supervise(false),
        Some(crate::model::startup::AS_SHELL_SWITCH) => supervise(true),
        Some("--stop") => {
            request_stop();
            0
        }
        Some("--restore-windows-desktop") => {
            let _ = registry::set_lulo_mode(false);
            stop_and_wait(10);
            restore_windows_desktop();
            0
        }
        Some("--uninstall") => {
            stop_and_wait(10);
            restore_windows_desktop();
            folders::turn_off();
            let _ = registry::set_starts_at_sign_in(None);
            registry::forget_lulo_mode();
            // Last: restoring the desktop above reads the AppBars' handles
            // from these settings.
            registry::forget_shell_settings();
            0
        }
        Some("--after-install") => start_after_install(),
        Some(_) => {
            eprintln!(
                "usage: lulo-session [--as-shell | --stop | --restore-windows-desktop | --uninstall | --after-install]"
            );
            2
        }
    }
}
