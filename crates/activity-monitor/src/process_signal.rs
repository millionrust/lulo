//! Process signal delivery bound to a kernel identity on Linux.

#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SignalKind {
    Terminate,
    Kill,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SignalOutcome {
    Delivered,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Missing,
    Unsupported,
    Rejected,
}

/// A handle opened before the confirmation-time identity refresh.
///
/// Linux uses a pidfd so a later PID reuse cannot retarget the signal. Other
/// development platforms retain the existing `sysinfo` signal path after the
/// same complete identity preflight.
pub(crate) struct ProcessHandle {
    #[cfg(target_os = "linux")]
    pidfd: OwnedFd,
}

impl ProcessHandle {
    #[cfg(target_os = "linux")]
    pub(crate) fn open(pid: u32) -> Result<Self, SignalOutcome> {
        let pid = libc::pid_t::try_from(pid).map_err(|_| SignalOutcome::Rejected)?;
        // SAFETY: pidfd_open takes a numeric PID and flags. Flags are required
        // to be zero, and a successful return is a newly owned file descriptor.
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if raw < 0 {
            return Err(classify_pidfd_error(std::io::Error::last_os_error()));
        }
        let raw = i32::try_from(raw).map_err(|_| SignalOutcome::Rejected)?;
        // SAFETY: a non-negative pidfd_open return is a new descriptor owned by
        // this call. OwnedFd closes it exactly once.
        let pidfd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Self { pidfd })
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn open(_pid: u32) -> Result<Self, SignalOutcome> {
        Ok(Self {})
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn send(
        self,
        kind: SignalKind,
        _fallback: impl FnOnce() -> Option<bool>,
    ) -> SignalOutcome {
        let signal = match kind {
            SignalKind::Terminate => libc::SIGTERM,
            SignalKind::Kill => libc::SIGKILL,
        };
        // SAFETY: `self.pidfd` is live for the duration of the syscall,
        // `siginfo` is null as permitted by pidfd_send_signal, and flags are
        // required to be zero.
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
            SignalOutcome::Delivered
        } else {
            classify_pidfd_error(std::io::Error::last_os_error())
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn send(
        self,
        _kind: SignalKind,
        fallback: impl FnOnce() -> Option<bool>,
    ) -> SignalOutcome {
        match fallback() {
            Some(true) => SignalOutcome::Delivered,
            Some(false) => SignalOutcome::Rejected,
            None => SignalOutcome::Unsupported,
        }
    }
}

#[cfg(target_os = "linux")]
fn classify_pidfd_error(error: std::io::Error) -> SignalOutcome {
    match error.raw_os_error() {
        Some(libc::ESRCH) => SignalOutcome::Missing,
        Some(libc::ENOSYS) | Some(libc::EINVAL) | Some(libc::ENODEV) => SignalOutcome::Unsupported,
        _ => SignalOutcome::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn development_fallback_preserves_every_sysinfo_result() {
        assert_eq!(
            ProcessHandle::open(1)
                .unwrap()
                .send(SignalKind::Terminate, || Some(true)),
            SignalOutcome::Delivered
        );
        assert_eq!(
            ProcessHandle::open(1)
                .unwrap()
                .send(SignalKind::Terminate, || Some(false)),
            SignalOutcome::Rejected
        );
        assert_eq!(
            ProcessHandle::open(1)
                .unwrap()
                .send(SignalKind::Terminate, || None),
            SignalOutcome::Unsupported
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_pidfd_errors_fail_closed() {
        assert_eq!(
            classify_pidfd_error(std::io::Error::from_raw_os_error(libc::ESRCH)),
            SignalOutcome::Missing
        );
        assert_eq!(
            classify_pidfd_error(std::io::Error::from_raw_os_error(libc::ENOSYS)),
            SignalOutcome::Unsupported
        );
        assert_eq!(
            classify_pidfd_error(std::io::Error::from_raw_os_error(libc::ENODEV)),
            SignalOutcome::Unsupported
        );
        assert_eq!(
            classify_pidfd_error(std::io::Error::from_raw_os_error(libc::EPERM)),
            SignalOutcome::Rejected
        );
    }
}
