//! Change the caller's own login password by driving `passwd(1)` over a
//! private pseudo-terminal, the way GNOME's Users panel does.
//!
//! Going through `passwd` means PAM verifies the current password and
//! applies the system's password-quality rules; no administrator
//! authorisation is involved and nothing runs with more privilege than
//! `passwd`'s own setuid helper. The passwords are written to the
//! terminal only after `passwd` has switched echo off, so they never come
//! back through the transcript; the transcript itself is zeroized.
//!
//! The environment is cleared (`LC_ALL=C` for stable prompts) and the
//! program path is absolute, so `PATH` and locale cannot redirect it.

use std::time::Duration;

use crate::model::Secret;

// The pseudo-terminal conversation with `passwd`, which only Unix has.
// Windows builds of Settings show no password change (the account belongs
// to Windows' own Settings there), and `change_own_password` says so.
#[cfg(unix)]
#[path = "passwd_pty.rs"]
mod pty;
#[cfg(unix)]
pub use pty::change_password_with;

/// The system `passwd`.
pub const PASSWD: &str = "/usr/bin/passwd";
#[cfg_attr(not(unix), allow(dead_code))]
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PasswdError {
    /// PAM rejected the current password.
    WrongPassword,
    /// PAM refused the new password; the message is PAM's own reason
    /// (password-quality rules, minimum age), never a secret.
    Rejected(String),
    /// `passwd` is missing or a pseudo-terminal could not be opened.
    Unavailable,
    /// `passwd` stopped responding.
    TimedOut,
    Failed,
}

impl PasswdError {
    pub fn message(&self) -> String {
        match self {
            PasswdError::WrongPassword => "The old password you entered isn’t correct.".into(),
            PasswdError::Rejected(reason) if !reason.is_empty() => {
                format!("The new password can’t be used. {reason}")
            }
            PasswdError::Rejected(_) => {
                "The new password can’t be used. Choose a different password.".into()
            }
            PasswdError::Unavailable => "Your password can’t be changed on this computer.".into(),
            PasswdError::TimedOut => "Changing the password took too long. Try again.".into(),
            PasswdError::Failed => "Your password couldn’t be changed. Try again.".into(),
        }
    }
}

/// Change the signed-in user's password with the system `passwd`.
pub fn change_own_password(current: &Secret, new: &Secret) -> Result<(), PasswdError> {
    #[cfg(unix)]
    {
        change_password_with(std::path::Path::new(PASSWD), current, new, TIMEOUT)
    }
    #[cfg(not(unix))]
    {
        let _ = (current, new);
        Err(PasswdError::Unavailable)
    }
}
