//! Windows host figures for the bottom panel: the CPU tick split, thread
//! count, file cache and per-disk counters, read with the same calls Task
//! Manager's own numbers come from. Each call is one cheap syscall (or one per
//! fixed drive), made only while the window is active and refreshing.

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, HWND,
    LPARAM, WPARAM,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetDriveTypeW, GetLogicalDrives, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE};
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::Threading::{
    GetProcessTimes, GetSystemTimes, OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};
use windows::Win32::System::IO::DeviceIoControl;

use crate::host_stats::DiskTotals;
use crate::process_signal::SignalOutcome;

const DRIVE_FIXED: u32 = 3;

fn filetime(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

/// `[user, system, idle, nice]` in 100 ns units since boot.
pub(crate) fn cpu_times() -> Option<[u64; 4]> {
    let (mut idle, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: three valid out-pointers to FILETIMEs that live for the call.
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    let idle = filetime(idle);
    let kernel = filetime(kernel);
    Some([filetime(user), kernel.saturating_sub(idle), idle, 0])
}

pub(crate) fn performance_info() -> Option<PERFORMANCE_INFORMATION> {
    let mut info = PERFORMANCE_INFORMATION::default();
    let size = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    // SAFETY: `info` is a valid out-structure of the size passed.
    unsafe { GetPerformanceInfo(&mut info, size) }.ok()?;
    Some(info)
}

/// Disk counters summed over the fixed drives' volumes
/// (`IOCTL_DISK_PERFORMANCE` on `\\.\X:`, which needs no administrator and no
/// access right beyond querying). A volume that refuses is skipped.
pub(crate) fn disk_totals() -> Option<DiskTotals> {
    // SAFETY: plain value-returning call.
    let mask = unsafe { GetLogicalDrives() };
    let mut totals = DiskTotals::default();
    let mut any = false;
    for index in 0..26u32 {
        if mask & (1 << index) == 0 {
            continue;
        }
        let letter = char::from(b'A' + index as u8);
        let root: Vec<u16> = format!("{letter}:\\\0").encode_utf16().collect();
        // SAFETY: `root` is NUL-terminated and outlives the call.
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != DRIVE_FIXED {
            continue;
        }
        let device: Vec<u16> = format!("\\\\.\\{letter}:\0").encode_utf16().collect();
        // SAFETY: `device` is NUL-terminated; access 0 only queries.
        let Ok(handle) = (unsafe {
            CreateFileW(
                PCWSTR(device.as_ptr()),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }) else {
            continue;
        };
        if let Some(counters) = disk_performance(handle) {
            totals.reads = totals.reads.saturating_add(u64::from(counters.ReadCount));
            totals.writes = totals.writes.saturating_add(u64::from(counters.WriteCount));
            totals.bytes_read = totals
                .bytes_read
                .saturating_add(counters.BytesRead.max(0) as u64);
            totals.bytes_written = totals
                .bytes_written
                .saturating_add(counters.BytesWritten.max(0) as u64);
            any = true;
        }
        // SAFETY: the handle was opened above and is closed once.
        let _ = unsafe { CloseHandle(handle) };
    }
    any.then_some(totals)
}

fn disk_performance(handle: HANDLE) -> Option<DISK_PERFORMANCE> {
    let mut counters = DISK_PERFORMANCE::default();
    let mut returned = 0u32;
    // SAFETY: the output buffer is a valid DISK_PERFORMANCE of the size passed.
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_PERFORMANCE,
            None,
            0,
            Some((&mut counters as *mut DISK_PERFORMANCE).cast()),
            std::mem::size_of::<DISK_PERFORMANCE>() as u32,
            Some(&mut returned),
            None,
        )
    }
    .ok()?;
    Some(counters)
}

/// Total CPU seconds (user plus kernel) a process has used, from
/// `GetProcessTimes`; `None` for one that has exited or refuses the query.
pub(crate) fn process_cpu_seconds(pid: u32) -> Option<u64> {
    // SAFETY: plain value call; the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: four valid out-pointers that live for the call.
    let times = unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) };
    // SAFETY: opened above, closed once.
    let _ = unsafe { CloseHandle(handle) };
    times.ok()?;
    Some((filetime(kernel) + filetime(user)) / 10_000_000)
}

/// A process handle held from the confirmation to the signal, closed on drop.
pub(crate) struct OwnedProcess {
    handle: HANDLE,
    pid: u32,
}

impl OwnedProcess {
    pub(crate) fn open(pid: u32) -> Result<Self, SignalOutcome> {
        // SAFETY: a plain value call; the returned handle is owned below.
        let handle = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        };
        match handle {
            Ok(handle) => Ok(Self { handle, pid }),
            Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() => {
                Err(SignalOutcome::Missing)
            }
            Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => {
                Err(SignalOutcome::Rejected)
            }
            Err(_) => Err(SignalOutcome::Rejected),
        }
    }

    /// Force Quit.
    pub(crate) fn terminate(&self) -> SignalOutcome {
        // SAFETY: `self.handle` is live until drop.
        match unsafe { TerminateProcess(self.handle, 1) } {
            Ok(()) => SignalOutcome::Delivered,
            Err(_) => {
                // Already gone counts as missing, anything else as refused.
                // SAFETY: as above; a zero timeout only polls.
                if unsafe { WaitForSingleObject(self.handle, 0) }.0 == 0 {
                    SignalOutcome::Missing
                } else {
                    SignalOutcome::Rejected
                }
            }
        }
    }

    /// Quit: post `WM_CLOSE` to every visible top-level window the process
    /// owns. A process with no window cannot be asked politely, which Force
    /// Quit is for.
    pub(crate) fn close_windows(&self) -> SignalOutcome {
        struct Search {
            pid: u32,
            posted: u32,
        }
        unsafe extern "system" fn visit(window: HWND, param: LPARAM) -> BOOL {
            // SAFETY: `param` is the `Search` passed to EnumWindows below.
            let search = unsafe { &mut *(param.0 as *mut Search) };
            let mut owner = 0u32;
            // SAFETY: valid window handle from EnumWindows and a valid out-pointer.
            unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
            // SAFETY: plain value call.
            if owner == search.pid && unsafe { IsWindowVisible(window) }.as_bool() {
                // SAFETY: posting a message to a window handle is always sound.
                if unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok() {
                    search.posted += 1;
                }
            }
            BOOL(1)
        }
        let mut search = Search {
            pid: self.pid,
            posted: 0,
        };
        // SAFETY: the callback only touches `search`, which outlives the call.
        let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
        if search.posted > 0 {
            SignalOutcome::Delivered
        } else {
            SignalOutcome::Unsupported
        }
    }
}

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        // SAFETY: opened in `open`, closed once here.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_tick_split_reads_and_advances() {
        let first = cpu_times().expect("GetSystemTimes");
        assert!(first[2] > 0, "idle time since boot is never zero");
        let second = cpu_times().unwrap();
        assert!(second[0] + second[1] + second[2] >= first[0] + first[1] + first[2]);
    }

    #[test]
    fn a_missing_process_is_reported_missing_and_ours_can_be_opened() {
        assert!(OwnedProcess::open(std::process::id()).is_ok());
        assert!(matches!(
            OwnedProcess::open(0x7fff_fff0),
            Err(SignalOutcome::Missing)
        ));
    }

    #[test]
    fn force_quit_ends_a_child_process() {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 >nul"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn cmd");
        let process = OwnedProcess::open(child.id()).expect("open child");
        assert_eq!(process.terminate(), SignalOutcome::Delivered);
        let status = child.wait().expect("wait");
        assert!(!status.success());
    }

    #[test]
    fn our_own_cpu_time_reads() {
        assert!(process_cpu_seconds(std::process::id()).is_some());
    }

    #[test]
    fn performance_info_counts_threads_and_cache() {
        let info = performance_info().expect("GetPerformanceInfo");
        assert!(info.ThreadCount > 1);
        assert!(info.PageSize >= 4096);
    }

    #[test]
    fn some_fixed_drive_reports_disk_counters() {
        // The runner's volumes allow the query; skip quietly where one does
        // not (a locked-down machine), the panel then omits the rows.
        if let Some(totals) = disk_totals() {
            assert!(totals.bytes_read > 0 || totals.reads > 0);
        }
    }
}
