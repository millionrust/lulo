//! Event-driven, kernel-backed foreground-job presentation for one PTY.

use std::sync::{Arc, Mutex};

use portable_pty::MasterPty;

const MAX_JOB_NAME_BYTES: usize = 64;

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

#[derive(Debug, Default)]
pub(super) struct ForegroundJobSource {
    #[cfg(unix)]
    fd: Option<OwnedFd>,
}

impl ForegroundJobSource {
    pub(super) fn from_master(master: &dyn MasterPty) -> Self {
        #[cfg(unix)]
        {
            let fd = master.as_raw_fd().and_then(|fd| {
                // SAFETY: `fd` remains owned by `master`; F_DUPFD_CLOEXEC
                // returns a separate descriptor owned by this source.
                let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
                (duplicated >= 0).then(|| unsafe { OwnedFd::from_raw_fd(duplicated) })
            });
            Self { fd }
        }
        #[cfg(not(unix))]
        {
            let _ = master;
            Self::default()
        }
    }

    fn foreground_pid(&self) -> Option<u32> {
        #[cfg(unix)]
        {
            let pid = unsafe { libc::tcgetpgrp(self.fd.as_ref()?.as_raw_fd()) };
            (pid > 0)
                .then_some(pid)
                .and_then(|pid| u32::try_from(pid).ok())
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct JobSnapshot {
    foreground_pid: Option<u32>,
    label: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct SessionJobState {
    snapshot: Arc<Mutex<JobSnapshot>>,
}

impl SessionJobState {
    /// Refresh only when PTY output wakes the existing reader worker. No timer,
    /// process polling loop, command text, or environment content is retained.
    pub(super) fn refresh(&self, source: &ForegroundJobSource, shell_pid: Option<u32>) -> bool {
        let foreground_pid = source.foreground_pid();
        if self
            .snapshot
            .lock()
            .is_ok_and(|snapshot| snapshot.foreground_pid == foreground_pid)
        {
            return false;
        }
        let process_name = foreground_pid.and_then(platform_process_name);
        self.apply_observation(foreground_pid, shell_pid, process_name.as_deref())
    }

    fn apply_observation(
        &self,
        foreground_pid: Option<u32>,
        shell_pid: Option<u32>,
        process_name: Option<&[u8]>,
    ) -> bool {
        let label = foreground_pid
            .filter(|pid| Some(*pid) != shell_pid)
            .and(process_name)
            .and_then(normalize_job_name);
        let Ok(mut snapshot) = self.snapshot.lock() else {
            return false;
        };
        let next = JobSnapshot {
            foreground_pid,
            label,
        };
        if *snapshot == next {
            return false;
        }
        *snapshot = next;
        true
    }

    pub(super) fn label(&self) -> Option<String> {
        self.snapshot.lock().ok()?.label.clone()
    }
}

#[cfg(target_os = "linux")]
fn platform_process_name(pid: u32) -> Option<Vec<u8>> {
    use std::io::Read as _;

    let mut file = std::fs::File::open(format!("/proc/{pid}/comm")).ok()?;
    let mut bytes = Vec::with_capacity(MAX_JOB_NAME_BYTES + 1);
    file.by_ref()
        .take((MAX_JOB_NAME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= MAX_JOB_NAME_BYTES).then_some(bytes)
}

#[cfg(target_os = "macos")]
fn platform_process_name(pid: u32) -> Option<Vec<u8>> {
    let mut bytes = [0u8; MAX_JOB_NAME_BYTES + 1];
    // SAFETY: `bytes` is writable for the exact advertised buffer length.
    let length = unsafe {
        libc::proc_name(
            i32::try_from(pid).ok()?,
            bytes.as_mut_ptr().cast(),
            u32::try_from(bytes.len()).ok()?,
        )
    };
    let length = usize::try_from(length).ok()?;
    (length > 0 && length <= MAX_JOB_NAME_BYTES).then(|| bytes[..length].to_vec())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_process_name(_pid: u32) -> Option<Vec<u8>> {
    None
}

fn normalize_job_name(bytes: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(bytes).ok()?.trim();
    if value.is_empty() || value.len() > MAX_JOB_NAME_BYTES {
        return None;
    }
    let mut normalized = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() || is_directional_control(character) {
            return None;
        }
        normalized.push(character);
    }
    Some(normalized)
}

fn is_directional_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_names_are_utf8_bounded_and_spoof_safe() {
        assert_eq!(normalize_job_name(b"  cargo\n").as_deref(), Some("cargo"));
        assert_eq!(normalize_job_name(b"bad\0name"), None);
        assert_eq!(normalize_job_name("vim\u{202e}".as_bytes()), None);
        assert_eq!(normalize_job_name(&[0xff]), None);
        assert_eq!(normalize_job_name(&[b'a'; MAX_JOB_NAME_BYTES + 1]), None);
    }

    #[test]
    fn exact_sessions_expose_only_their_current_foreground_label() {
        let first = SessionJobState::default();
        let second = first.clone();
        let other = SessionJobState::default();

        assert!(first.apply_observation(Some(40), Some(40), Some(b"bash")));
        assert_eq!(first.label(), None);
        assert!(first.apply_observation(Some(42), Some(40), Some(b"cargo\n")));
        assert_eq!(second.label().as_deref(), Some("cargo"));
        assert_eq!(other.label(), None);
        assert!(!first.apply_observation(Some(42), Some(40), Some(b"cargo")));
        assert!(first.apply_observation(None, Some(40), None));
        assert_eq!(first.label(), None);
    }
}
