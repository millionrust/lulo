//! Narrow system-action helper for cues that must outlive the invoking shell.

use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let Some(action) = arguments.next() else {
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        return ExitCode::from(2);
    }
    match action.as_str() {
        "screenshot-screen" => screenshot_screen(),
        "login" => {
            let _ = rmac_sound::play_blocking(rmac_sound::Cue::Boot);
            ExitCode::SUCCESS
        }
        "unlock" => {
            let _ = rmac_sound::play_blocking(rmac_sound::Cue::Unlock);
            ExitCode::SUCCESS
        }
        _ => ExitCode::from(2),
    }
}

fn screenshot_screen() -> ExitCode {
    let succeeded = Command::new("/usr/bin/niri")
        .args(["msg", "action", "screenshot-screen"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !succeeded {
        return ExitCode::FAILURE;
    }
    let _ = rmac_sound::play_blocking(rmac_sound::Cue::Screenshot);
    ExitCode::SUCCESS
}
