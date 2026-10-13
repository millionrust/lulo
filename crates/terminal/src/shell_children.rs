//! Whether a shell on Windows is running a program of its own.
//!
//! Terminal asks before it closes a window or tab only when something other
//! than the shell is running, as on the Mac. Unix reads the PTY's foreground
//! process group; Windows' pseudo console has none, so every close asked
//! "Do you want to terminate running processes in this window?" even at an
//! idle prompt (found by the robot, scripts/robot/windows). There, the shell
//! is busy when it has a child process other than its console host.
//!
//! One process snapshot per close request: nothing polls.

/// Console hosts the pseudo console starts beside the shell: not jobs.
#[cfg(any(windows, test))]
const CONSOLE_HOSTS: &[&str] = &["conhost.exe", "openconsole.exe"];

/// Whether `processes` (pid, parent pid, executable name) holds a child of
/// `shell` that is a program the user started.
#[cfg(any(windows, test))]
pub(crate) fn has_job(shell: u32, processes: &[(u32, u32, String)]) -> bool {
    processes.iter().any(|(pid, parent, name)| {
        *parent == shell
            && *pid != shell
            && !CONSOLE_HOSTS
                .iter()
                .any(|host| name.eq_ignore_ascii_case(host))
    })
}

/// Whether the shell `shell` has a job running now. `None` when the process
/// list cannot be read (the caller then asks, the safe answer).
#[cfg(windows)]
pub(crate) fn shell_has_job(shell: u32) -> Option<bool> {
    Some(has_job(shell, &snapshot()?))
}

#[cfg(windows)]
fn snapshot() -> Option<Vec<(u32, u32, String)>> {
    use std::ffi::c_void;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ProcessEntry32W {
        size: u32,
        usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        priority_class_base: i32,
        flags: u32,
        exe_file: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    // SAFETY: plain Toolhelp calls; the handle is closed below and the entry
    // is a correctly sized, zeroed PROCESSENTRY32W.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot as isize == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut processes = Vec::new();
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let length = entry
                .exe_file
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(entry.exe_file.len());
            processes.push((
                entry.process_id,
                entry.parent_process_id,
                String::from_utf16_lossy(&entry.exe_file[..length]),
            ));
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        Some(processes)
    }
}

#[cfg(test)]
mod tests {
    use super::has_job;

    fn list(entries: &[(u32, u32, &str)]) -> Vec<(u32, u32, String)> {
        entries
            .iter()
            .map(|(pid, parent, name)| (*pid, *parent, (*name).to_owned()))
            .collect()
    }

    #[test]
    fn an_idle_shell_has_no_job() {
        let processes = list(&[
            (100, 50, "rmac-terminal.exe"),
            (200, 100, "powershell.exe"),
            (201, 200, "conhost.exe"),
            (202, 100, "OpenConsole.exe"),
        ]);
        assert!(!has_job(200, &processes));
    }

    #[test]
    fn a_program_the_shell_started_is_a_job() {
        let processes = list(&[
            (200, 100, "powershell.exe"),
            (201, 200, "conhost.exe"),
            (300, 200, "python.exe"),
        ]);
        assert!(has_job(200, &processes));
        // Another shell's children do not count.
        assert!(!has_job(400, &processes));
    }
}
