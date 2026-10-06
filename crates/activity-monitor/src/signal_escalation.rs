//! Administrator-privileged signal delivery for a process owned by another
//! user (MON-10/MON-14: "polkit for other users' processes"). Linux's
//! `pidfd_send_signal` already fails closed with `EPERM` when the signal's
//! target is owned by someone else; this module is the one honest way
//! forward from there — `pkexec`'s default `org.freedesktop.policykit.
//! pkexec.run-program` action lets any local, active-session user run one
//! command as root after authenticating, with no bespoke polkit rule
//! needed. When `pkexec` itself is not installed, this reports that
//! plainly rather than pretending the signal was delivered.

use std::process::{Command, Output};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EscalationOutcome {
    /// `pkexec kill -s <signal> <pid>` exited zero.
    Delivered,
    /// The administrator helper ran but the signal was not delivered —
    /// authentication was declined/cancelled, or `kill` itself failed
    /// (e.g. the process had already exited).
    Denied,
    /// No PolicyKit authentication agent/`pkexec` binary is available on
    /// this system.
    NoHelper,
    /// `pkexec` could not even be started for a reason other than "not
    /// found" (carries the OS error text for the feedback detail).
    Error(String),
}

/// Pure classification of the already-run command's result, kept separate
/// from `escalate` so it is testable without actually spawning a process.
pub(crate) fn classify(result: std::io::Result<Output>) -> EscalationOutcome {
    match result {
        Ok(output) if output.status.success() => EscalationOutcome::Delivered,
        Ok(_) => EscalationOutcome::Denied,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => EscalationOutcome::NoHelper,
        Err(error) => EscalationOutcome::Error(error.to_string()),
    }
}

/// Run `pkexec kill -s <signal_number> <pid>`. Blocking — the caller must
/// run this off the UI thread, since `pkexec` may show an authentication
/// prompt and wait on it.
pub(crate) fn escalate(pid: u32, signal_number: i32) -> EscalationOutcome {
    let result = Command::new("pkexec")
        .arg("kill")
        .arg("-s")
        .arg(signal_number.to_string())
        .arg(pid.to_string())
        .output();
    classify(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn missing_pkexec_is_reported_as_no_helper() {
        let error = io::Error::new(io::ErrorKind::NotFound, "no such file");
        assert_eq!(classify(Err(error)), EscalationOutcome::NoHelper);
    }

    #[test]
    fn other_spawn_failures_carry_their_message() {
        let error = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        match classify(Err(error)) {
            EscalationOutcome::Error(message) => assert!(message.contains("denied")),
            other => panic!("expected Error, got {other:?}"),
        }
    }
}
