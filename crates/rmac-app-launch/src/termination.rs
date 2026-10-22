//! Bounded application-process termination through stable Linux identities.

#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const MAX_APPLICATION_PROCESSES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationKind {
    Quit,
    ForceQuit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationError {
    InvalidIdentity,
    Missing,
    Unsupported,
    Rejected,
}

impl std::fmt::Display for TerminationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "the application process identity is invalid",
            Self::Missing => "the application is no longer running",
            Self::Unsupported => "application termination is not supported on this system",
            Self::Rejected => "the operating system rejected application termination",
        })
    }
}

impl std::error::Error for TerminationError {}

/// Terminate every compositor-authoritative process for one application.
///
/// Linux opens every pidfd before sending any signal. This prevents a PID from
/// being silently retargeted after validation and avoids partially terminating
/// an application merely because a later identity cannot be opened.
pub async fn terminate_application(
    mut pids: Vec<u32>,
    kind: TerminationKind,
) -> Result<(), TerminationError> {
    normalize(&mut pids)?;
    blocking::unblock(move || terminate_blocking(&pids, kind)).await
}

fn normalize(pids: &mut Vec<u32>) -> Result<(), TerminationError> {
    if pids.is_empty() || pids.len() > MAX_APPLICATION_PROCESSES || pids.contains(&0) {
        return Err(TerminationError::InvalidIdentity);
    }
    pids.sort_unstable();
    pids.dedup();
    Ok(())
}

#[cfg(target_os = "linux")]
fn terminate_blocking(pids: &[u32], kind: TerminationKind) -> Result<(), TerminationError> {
    let handles = pids
        .iter()
        .copied()
        .map(ProcessHandle::open)
        .collect::<Result<Vec<_>, _>>()?;
    for handle in handles {
        handle.send(kind)?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn terminate_blocking(_pids: &[u32], _kind: TerminationKind) -> Result<(), TerminationError> {
    Err(TerminationError::Unsupported)
}

/// Windows: every process is opened before any is ended, as on Linux.
/// Force Quit ends them (`TerminateProcess`); Quit asks each one's windows
/// to close (`WM_CLOSE`), as their close buttons would, so they may ask to
/// save.
#[cfg(windows)]
fn terminate_blocking(pids: &[u32], kind: TerminationKind) -> Result<(), TerminationError> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };

    match kind {
        TerminationKind::ForceQuit => {
            let mut handles: Vec<HANDLE> = Vec::with_capacity(pids.len());
            for pid in pids {
                // SAFETY: opens a process for termination only.
                match unsafe { OpenProcess(PROCESS_TERMINATE, false, *pid) } {
                    Ok(handle) => handles.push(handle),
                    Err(_) => {
                        for handle in handles {
                            // SAFETY: closes the handles opened above.
                            let _ = unsafe { CloseHandle(handle) };
                        }
                        return Err(TerminationError::Missing);
                    }
                }
            }
            let mut failed = false;
            for handle in handles {
                // SAFETY: ends a process this call opened, then closes it.
                unsafe {
                    failed |= TerminateProcess(handle, 1).is_err();
                    let _ = CloseHandle(handle);
                }
            }
            if failed {
                Err(TerminationError::Rejected)
            } else {
                Ok(())
            }
        }
        TerminationKind::Quit => {
            unsafe extern "system" fn close(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
                // SAFETY: `lparam` is the PID list passed below, alive for
                // the whole enumeration.
                let pids = unsafe { &*(lparam.0 as *const Vec<u32>) };
                let mut pid = 0u32;
                // SAFETY: reads properties of the window being enumerated
                // and posts it WM_CLOSE.
                unsafe {
                    GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
                    if pids.contains(&pid) && IsWindowVisible(hwnd).as_bool() {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
                true.into()
            }
            let pids = pids.to_vec();
            // SAFETY: the callback only reads `pids`, which outlives the call.
            let _ = unsafe { EnumWindows(Some(close), LPARAM(&pids as *const Vec<u32> as isize)) };
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
struct ProcessHandle {
    pidfd: OwnedFd,
}

#[cfg(target_os = "linux")]
impl ProcessHandle {
    fn open(pid: u32) -> Result<Self, TerminationError> {
        let pid = libc::pid_t::try_from(pid).map_err(|_| TerminationError::InvalidIdentity)?;
        // SAFETY: pidfd_open accepts one numeric PID and zero flags. A
        // non-negative result is a newly owned descriptor.
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if raw < 0 {
            return Err(classify(std::io::Error::last_os_error()));
        }
        let raw = i32::try_from(raw).map_err(|_| TerminationError::Rejected)?;
        // SAFETY: the successful pidfd_open result is uniquely owned here.
        let pidfd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Self { pidfd })
    }

    fn send(self, kind: TerminationKind) -> Result<(), TerminationError> {
        let signal = match kind {
            TerminationKind::Quit => libc::SIGTERM,
            TerminationKind::ForceQuit => libc::SIGKILL,
        };
        // SAFETY: the pidfd remains live for the syscall, null siginfo is
        // permitted, and pidfd_send_signal requires zero flags.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.pidfd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(classify(std::io::Error::last_os_error()))
        }
    }
}

#[cfg(target_os = "linux")]
fn classify(error: std::io::Error) -> TerminationError {
    match error.raw_os_error() {
        Some(libc::ESRCH) => TerminationError::Missing,
        Some(libc::ENOSYS) | Some(libc::EINVAL) | Some(libc::ENODEV) => {
            TerminationError::Unsupported
        }
        _ => TerminationError::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_identity_set_is_bounded_sorted_and_unique() {
        let mut pids = vec![19, 7, 19];
        normalize(&mut pids).unwrap();
        assert_eq!(pids, [7, 19]);

        assert_eq!(
            normalize(&mut Vec::new()),
            Err(TerminationError::InvalidIdentity)
        );
        assert_eq!(
            normalize(&mut vec![0]),
            Err(TerminationError::InvalidIdentity)
        );
        assert_eq!(
            normalize(&mut vec![1; MAX_APPLICATION_PROCESSES + 1]),
            Err(TerminationError::InvalidIdentity)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_pidfd_errors_fail_closed() {
        assert_eq!(
            classify(std::io::Error::from_raw_os_error(libc::ESRCH)),
            TerminationError::Missing
        );
        assert_eq!(
            classify(std::io::Error::from_raw_os_error(libc::ENOSYS)),
            TerminationError::Unsupported
        );
        assert_eq!(
            classify(std::io::Error::from_raw_os_error(libc::EPERM)),
            TerminationError::Rejected
        );
    }
}
