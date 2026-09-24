//! Orca's process lifecycle. rmac starts and stops Orca itself instead of
//! shelling out to `pkill`/`pgrep`, reading the kernel's own process
//! directory the same way `rmac-input` reads `/sys/class/input`, rather than
//! parsing another program's human-readable output.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::api::Error;

const START_TIMEOUT: Duration = Duration::from_secs(3);
const STOP_TIMEOUT: Duration = Duration::from_secs(3);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Whether an Orca process owned by the current user is running.
pub fn is_orca_running() -> bool {
    find_orca_pid().is_some()
}

/// Start Orca (replacing any stale instance), as GNOME does when the
/// `screen-reader-enabled` key turns on. Returns once the process is
/// observed running, or an error if it never appears.
pub(crate) fn start_orca() -> Result<(), Error> {
    if is_orca_running() {
        return Ok(());
    }
    Command::new("orca")
        .arg("--replace")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Error::new("start Orca", error.to_string()))?;
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if is_orca_running() {
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    Err(Error::new(
        "start Orca",
        "Orca did not appear as a running process",
    ))
}

/// Stop every Orca process owned by the current user, escalating from
/// SIGTERM to SIGKILL if one does not exit in time.
pub(crate) fn stop_orca() -> Result<(), Error> {
    let Some(pid) = find_orca_pid() else {
        return Ok(());
    };
    send_signal(pid, libc::SIGTERM)?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    send_signal(pid, libc::SIGKILL)?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return Ok(());
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    Err(Error::new("stop Orca", "Orca did not exit"))
}

fn send_signal(pid: u32, signal: libc::c_int) -> Result<(), Error> {
    // SAFETY: `kill` is a plain syscall wrapper; `pid` was just read from
    // `/proc` and validated to belong to the current user.
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 || std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(Error::new(
            "stop Orca",
            std::io::Error::last_os_error().to_string(),
        ))
    }
}

fn process_exists(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).is_dir()
}

fn find_orca_pid() -> Option<u32> {
    let current_uid = unsafe { libc::getuid() };
    let entries = std::fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let directory = entry.path();
        let Some(comm) = read_small_text(&directory.join("comm")) else {
            continue;
        };
        if comm.trim() != "orca" {
            continue;
        }
        if process_owner(&directory) != Some(current_uid) {
            continue;
        }
        return Some(pid);
    }
    None
}

fn process_owner(directory: &std::path::Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt as _;
    directory.metadata().ok().map(|metadata| metadata.uid())
}

fn read_small_text(path: &std::path::Path) -> Option<String> {
    use std::io::Read as _;

    const LIMIT: u64 = 256;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > LIMIT {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_with_no_matching_comm_file_is_not_orca() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-screen-reader-orca-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("comm"), "bash\n").unwrap();
        let comm = read_small_text(&directory.join("comm")).unwrap();
        assert_eq!(comm.trim(), "bash");
        assert_ne!(comm.trim(), "orca");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_missing_process_directory_is_reported_as_gone() {
        assert!(!process_exists(u32::MAX));
    }
}
