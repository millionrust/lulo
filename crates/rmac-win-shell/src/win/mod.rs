//! The Win32 side of the Lulo layer that only Windows has.

pub use rmac_shell_layer::windows::{appbar, power, surface};
pub mod catalog;
pub mod desktop;
pub mod events;
pub mod folders;
pub mod icons;
pub mod launch;
pub mod memory;
pub mod registry;
pub mod session;
pub mod taskbar;
pub mod wallpaper;
pub mod wallpaper_layer;


/// `s` as a NUL-terminated UTF-16 string.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// UTF-16 up to the first NUL.
pub fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

pub fn user_name() -> String {
    std::env::var("USERNAME").unwrap_or_default()
}

/// With `LULO_SHELL_TRACE=1`, say what the shell does on stderr, for the
/// Windows CI checks (`scripts/windows/launch_smoke.py --shell`).
pub fn trace(message: impl FnOnce() -> String) {
    rmac_shell_layer::windows::trace::trace(message);
}

/// Milliseconds since this process was created, for the start-up trace.
pub fn process_millis() -> f64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: plain out-parameters on this process's own pseudo-handle.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    }
    .is_ok();
    if !ok {
        return 0.0;
    }
    // SAFETY: returns the time; no pointers kept.
    let now = unsafe { GetSystemTimeAsFileTime() };
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    ticks(now).saturating_sub(ticks(created)) as f64 / 10_000.0
}

/// The window with handle `hwnd`.
pub fn handle(hwnd: isize) -> windows::Win32::Foundation::HWND {
    windows::Win32::Foundation::HWND(hwnd as *mut core::ffi::c_void)
}

/// The executable path of process `pid`, or empty when it cannot be read.
pub fn process_path(pid: u32) -> String {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
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
