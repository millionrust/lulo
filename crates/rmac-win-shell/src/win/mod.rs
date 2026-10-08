//! The Win32 side of the Lulo layer.

pub mod appbar;
pub mod backdrop;
pub mod catalog;
pub mod desktop;
pub mod desktop_files;
pub mod events;
pub mod folders;
pub mod icons;
pub mod launch;
pub mod memory;
pub mod menubar_server;
pub mod power;
pub mod recycle;
pub mod registry;
pub mod session;
pub mod status;
pub mod surface;
pub mod taskbar;
pub mod wallpaper;
pub mod windows_list;

use std::sync::OnceLock;

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
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("LULO_SHELL_TRACE").is_some_and(|value| value == "1")) {
        eprintln!("lulo-shell: {}", message());
    }
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
