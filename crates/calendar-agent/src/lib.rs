//! `calendar-agent`: a small, no-GPUI background agent that fires Calendar's
//! reminders even while Calendar itself is closed (ADR 0022 §6/§7,
//! `docs/design/calendar-mail.md` CAL-7).
//!
//! `alarms`, `state` and `engine` are pure and portable -- they are the
//! part unit tests exercise with a fake clock. `linux` is the Linux-only
//! runtime glue (EDS reading through `rmac-calendar-eds`/`rmac-calendar-
//! runtime`, one `CLOCK_REALTIME` timerfd, and notification posting through
//! the existing freedesktop `Notifications` service); it is verified on the
//! reference laptop, not by this crate's test suite.

pub mod alarms;
pub mod engine;
pub mod state;

#[cfg(target_os = "linux")]
pub mod linux;

use std::io;
use std::path::PathBuf;

/// `ConditionPathExists=` in `rmac-calendar-agent.service`: present once
/// Calendar has seen at least one calendar, so the unit only ever runs on a
/// machine that actually has something to remind about.
pub const MARKER_FILE: &str = "lulo/calendar/agent-enabled";
pub const UNIT_NAME: &str = "rmac-calendar-agent.service";

pub fn marker_path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|root| root.join(MARKER_FILE))
}

/// Called by Calendar (`crates/calendar`) the first time it loads at least
/// one calendar: creates the marker file if it is not already there and
/// starts the agent unit. Idempotent and best-effort -- a failure here just
/// means the unit starts on the next normal session start instead (it is
/// also in `rmac-session.target`'s `Wants=`, gated by the same marker).
#[cfg(target_os = "linux")]
pub fn ensure_enabled() -> io::Result<()> {
    let marker = marker_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no state directory"))?;
    if let Some(parent) = marker.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    match rmac_storage::write_new_private(&marker, b"") {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "start", UNIT_NAME])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_enabled() -> io::Result<()> {
    Ok(())
}
