//! `rmac-calendar-agent`: entry point for the systemd user service. See
//! `crates/calendar-agent/src/lib.rs` and ADR 0022 §6/§7.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    // systemd's own `ConditionPathExists=` already gates the unit on this
    // marker; checking it again here makes a manual or test invocation
    // behave the same way instead of arming a timer no one asked for.
    let enabled = rmac_calendar_agent::marker_path()
        .map(|path| path.is_file())
        .unwrap_or(false);
    if !enabled {
        return ExitCode::SUCCESS;
    }
    match rmac_calendar_agent::linux::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rmac-calendar-agent: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("rmac-calendar-agent: calendar reminders require Linux");
    ExitCode::FAILURE
}
