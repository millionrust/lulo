//! rmac: opt-in tracing of start-up phases and idle wake-ups
//! (docs/decisions/0025-vendor-gpui-windows.md).
//!
//! - `RMAC_GPUI_STARTUP_TRACE=1` prints one `gpui_windows startup:` line per
//!   start-up phase, timed from the moment Windows created the process, so
//!   the numbers line up with `scripts/windows/launch_smoke.py`'s launch time.
//! - `RMAC_GPUI_WAKE_TRACE=1` prints one `gpui_windows wake:` line for every
//!   thing that wakes the process: each message the main thread's loop
//!   retrieves, each task it runs (with the task's spawn site), each
//!   thread-pool task and timer, each vsync-thread tick and each frame.
//!   `launch_smoke.py` groups the lines that fall in its idle window by
//!   source, which names each idle wake-up.
//!
//! Both are read once; with neither set, each call is one relaxed load.

use std::{fmt::Display, sync::OnceLock};

use windows::Win32::{
    Foundation::FILETIME,
    System::{
        SystemInformation::GetSystemTimePreciseAsFileTime,
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
};

fn flag(cell: &'static OnceLock<bool>, name: &str) -> bool {
    *cell.get_or_init(|| std::env::var(name).is_ok_and(|value| value == "1"))
}

pub(crate) fn startup_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    flag(&ENABLED, "RMAC_GPUI_STARTUP_TRACE")
}

pub(crate) fn wake_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    flag(&ENABLED, "RMAC_GPUI_WAKE_TRACE")
}

fn filetime_100ns(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// Milliseconds since Windows created this process.
pub(crate) fn since_process_start_ms() -> f64 {
    static CREATED: OnceLock<u64> = OnceLock::new();
    let created = *CREATED.get_or_init(|| {
        let [mut creation, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
        // SAFETY: the pseudo handle is always valid and the out pointers
        // point at live locals.
        match unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        } {
            Ok(()) => filetime_100ns(creation),
            Err(_) => 0,
        }
    });
    // SAFETY: no preconditions.
    let now = filetime_100ns(unsafe { GetSystemTimePreciseAsFileTime() });
    now.saturating_sub(created) as f64 / 10_000.0
}

/// One start-up phase reached.
pub(crate) fn startup(phase: &str) {
    if startup_enabled() {
        eprintln!(
            "gpui_windows startup: {phase} at {:.1} ms",
            since_process_start_ms()
        );
    }
}

/// rmac: run a thread-pool task and, with the wake trace on, report the CPU
/// time it used as `gpui_windows cpu: <source> <site> <100 ns> at <ms> ms`.
/// The app's own pool tasks and a software rasteriser's workers (WARP on a
/// machine without a GPU) share the system thread pool, so this is what
/// tells them apart (`scripts/windows/launch_smoke.py`). Thread times move
/// a whole scheduler tick at a time, so one task reads 0 or a tick; summed
/// over many tasks they are the true total.
pub(crate) fn pool_task(source: &str, detail: impl Display, run: impl FnOnce()) {
    if !wake_enabled() {
        run();
        return;
    }
    let before = current_thread_cpu_100ns();
    run();
    let used = current_thread_cpu_100ns().saturating_sub(before);
    eprintln!(
        "gpui_windows cpu: {source} {detail} {used} at {:.1} ms",
        since_process_start_ms()
    );
}

fn current_thread_cpu_100ns() -> u64 {
    use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let [mut creation, mut exit, mut kernel, mut user] = [FILETIME::default(); 4];
    // SAFETY: the pseudo handle is always valid and the out pointers point
    // at live locals.
    match unsafe { GetThreadTimes(GetCurrentThread(), &mut creation, &mut exit, &mut kernel, &mut user) } {
        Ok(()) => filetime_100ns(kernel) + filetime_100ns(user),
        Err(_) => 0,
    }
}

/// One wake-up, by source, with a detail such as a message id or a task's
/// spawn site.
pub(crate) fn wake(source: &str, detail: impl Display) {
    if wake_enabled() {
        eprintln!(
            "gpui_windows wake: {source} {detail} at {:.1} ms",
            since_process_start_ms()
        );
    }
}
