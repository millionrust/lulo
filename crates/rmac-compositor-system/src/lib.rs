//! The session's windows for the shell, through whichever backend the
//! platform has: niri's IPC on Lulo OS, Win32 on Windows (ADR 0023, "Phase 3
//! revised: shared shell views").
//!
//! Every function here has the same name and meaning as in
//! `rmac-compositor-niri`, which it re-exports unchanged on Unix, so the
//! shell's views call one API and Lulo OS behaves exactly as before.

pub use rmac_compositor as domain;

#[cfg(unix)]
pub use rmac_compositor_niri::{
    action_capabilities, execute, execute_action, minimize_request, minimize_window_in, snapshot,
    watch, Error, MinimizeError,
};

/// Whether `error` only says no compositor is reachable (niri's socket is
/// not set, as in a nested test session): callers report a disconnected
/// compositor and keep their other sources running.
#[cfg(unix)]
pub fn is_unavailable(error: &Error) -> bool {
    matches!(error, Error::MissingSocketPath)
}

#[cfg(windows)]
mod win32;

#[cfg(windows)]
pub use win32::{
    action_capabilities, execute, execute_action, is_unavailable, minimize_request,
    minimize_window_in, refresh, snapshot, watch, Error, MinimizeError,
};
