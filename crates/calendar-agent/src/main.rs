//! `rmac-calendar-agent`: entry point for the systemd user service. See
//! `crates/calendar-agent/src/lib.rs` and ADR 0022 §6/§7.
#![cfg_attr(windows, windows_subsystem = "windows")]

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

/// Windows: a Task Scheduler task (or Calendar) runs this with `--check`;
/// there is no window, so the program has no console either.
#[cfg(windows)]
fn main() -> ExitCode {
    match rmac_calendar_agent::windows::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rmac-calendar-agent: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
fn main() -> ExitCode {
    eprintln!("rmac-calendar-agent: calendar reminders require Linux");
    ExitCode::FAILURE
}
