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
//! Off Lulo does. `lulo-session --restore-windows-desktop` only gives the
//! desktop back (the "Restore Windows taskbar" escape hatch).

use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject, INFINITE,
};

use super::{appbar, launch, registry, taskbar, user_name, windows_list};

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
            appbar::remove(windows_list::handle(hwnd as isize));
            registry::delete_value(value);
        }
    }
    taskbar::restore();
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

fn request_stop() {
    // SAFETY: opens (or creates) the named event and sets it.
    if let Ok(event) = unsafe { CreateEventW(None, false, false, &stop_event_name()) } {
        // SAFETY: as above.
        let _ = unsafe { SetEvent(event) };
    }
}

fn supervise() -> i32 {
    let name = HSTRING::from(format!(r"Local\lulo-session-{}", user_name()));
    // SAFETY: a named mutex held for the life of the process.
    let mutex = unsafe { CreateMutexW(None, true, &name) };
    // SAFETY: reads this thread's last error, set by the call above.
    if mutex.is_err() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // The Lulo layer is on already.
        return 0;
    }
    restore_windows_desktop();
    let shell = launch::install_dir().join("lulo-shell.exe");
    let mut starts: Vec<Instant> = Vec::new();
    loop {
        starts.retain(|start| start.elapsed() < Duration::from_secs(60));
        if starts.len() >= RESTARTS_PER_MINUTE {
            eprintln!("lulo-session: Lulo stopped {RESTARTS_PER_MINUTE} times in a minute; the Windows desktop is back.");
            return 1;
        }
        starts.push(Instant::now());
        let status = std::process::Command::new(&shell).status();
        restore_windows_desktop();
        match status {
            Ok(status) if status.success() || status.code() == Some(ALREADY_RUNNING) => return 0,
            Ok(status) => eprintln!("lulo-session: Lulo stopped ({status}); starting it again"),
            Err(error) => {
                eprintln!("lulo-session: cannot start {}: {error}", shell.display());
                return 1;
            }
        }
    }
}

/// `lulo-session [--stop | --restore-windows-desktop]`.
pub fn run(arguments: &[String]) -> i32 {
    match arguments.first().map(String::as_str) {
        None => supervise(),
        Some("--stop") => {
            request_stop();
            0
        }
        Some("--restore-windows-desktop") => {
            restore_windows_desktop();
            0
        }
        Some(_) => {
            eprintln!("usage: lulo-session [--stop | --restore-windows-desktop]");
            2
        }
    }
}
