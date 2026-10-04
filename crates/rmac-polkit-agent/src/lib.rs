//! Lulo's polkit authentication agent (`rmac-polkit-agent`, SWU-07).
//!
//! When an app asks for something polkit guards with `auth_admin`
//! (adding a user, turning on Remote Login, installing a package), polkitd
//! calls this agent's `BeginAuthentication`. The agent shows the Mac's
//! password dialog ("System Settings wants to make changes."), hands the
//! password to polkit's own helper (`polkit-agent-helper-1`, see
//! [`helper`]), and returns when the helper has told polkitd the result.
//! It never checks a password itself.
//!
//! Layout: [`dbus`] (registration and the agent interface), [`request`]
//! (one dialog at a time, queueing, cancellation, retries), [`helper`] (the
//! helper's line protocol), [`identity`], [`secret`] and [`text`]; the
//! dialog itself is `ui` (Linux only).

pub mod dbus;
pub mod helper;
pub mod identity;
pub mod request;
pub mod secret;
pub mod text;
#[cfg(target_os = "linux")]
pub mod ui;

/// Keep the password out of core dumps and away from same-user ptrace:
/// the process is made non-dumpable before any secret exists.
#[cfg(target_os = "linux")]
pub fn harden_process() {
    // SAFETY: PR_SET_DUMPABLE takes an integer argument and has no memory
    // effects.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}
