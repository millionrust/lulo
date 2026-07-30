use std::process::{Command, ExitCode};
use std::thread;
use std::time::Duration;

use rmac_session::{DiagnosticReport, Supervisor, COMPONENT_UNITS};
use rmac_shell_settings::ShellSettingsStore;

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    match arguments.as_slice() {
        [command] if command == "status" => {
            let supervisor = Supervisor::from_environment()?;
            println!("{}", serde_json::to_string_pretty(&supervisor.snapshot()?)?);
        }
        [command] if command == "diagnostics" => {
            let supervisor = Supervisor::from_environment()?;
            let settings = ShellSettingsStore::from_environment()?;
            let report =
                DiagnosticReport::from_health(supervisor.snapshot()?, settings.recovery_state());
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        [command] if command == "restore-last-good-settings" => {
            ShellSettingsStore::from_environment()?.restore_last_good()?;
            println!("Restored the last-known-good shell settings.");
        }
        [command] if command == "write-health" => {
            let supervisor = Supervisor::from_environment()?;
            supervisor.write_health()?;
        }
        [command] if command == "monitor" => {
            let supervisor = Supervisor::from_environment()?;
            loop {
                if let Err(error) = supervisor.write_health() {
                    eprintln!("{error}");
                }
                thread::sleep(Duration::from_secs(5));
            }
        }
        [command, unit] if command == "observe-failure" => {
            let supervisor = Supervisor::from_environment()?;
            if supervisor.observe_failure(unit)? {
                systemctl(&["stop", "rmac-session.target"])?;
                systemctl(&["start", "rmac-safe-mode.target"])?;
            }
        }
        [command] if command == "clear-safe-mode" => {
            let supervisor = Supervisor::from_environment()?;
            supervisor.clear_safe_mode()?;
            let mut reset = vec!["reset-failed"];
            reset.extend(COMPONENT_UNITS);
            systemctl(&reset)?;
            systemctl(&["stop", "rmac-safe-mode.target"])?;
            systemctl(&["start", "rmac-session.target"])?;
        }
        _ => {
            return Err("usage: rmac-session-supervisor <status|diagnostics|restore-last-good-settings|write-health|monitor|observe-failure UNIT|clear-safe-mode>".into());
        }
    }
    Ok(())
}

fn systemctl(arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(arguments)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl --user {} failed", arguments.join(" ")).into())
    }
}
