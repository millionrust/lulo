use std::process::{Command, ExitCode};
use std::thread;
use std::time::Duration;

use rmac_session::{Supervisor, COMPONENT_UNITS};

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
    let supervisor = Supervisor::from_environment()?;
    match arguments.as_slice() {
        [command] if command == "status" => {
            println!("{}", serde_json::to_string_pretty(&supervisor.snapshot()?)?);
        }
        [command] if command == "write-health" => {
            supervisor.write_health()?;
        }
        [command] if command == "monitor" => loop {
            if let Err(error) = supervisor.write_health() {
                eprintln!("{error}");
            }
            thread::sleep(Duration::from_secs(5));
        },
        [command, unit] if command == "observe-failure" => {
            if supervisor.observe_failure(unit)? {
                systemctl(&["stop", "rmac-session.target"])?;
                systemctl(&["start", "rmac-safe-mode.target"])?;
            }
        }
        [command] if command == "clear-safe-mode" => {
            supervisor.clear_safe_mode()?;
            let mut reset = vec!["reset-failed"];
            reset.extend(COMPONENT_UNITS);
            systemctl(&reset)?;
            systemctl(&["stop", "rmac-safe-mode.target"])?;
            systemctl(&["start", "rmac-session.target"])?;
        }
        _ => {
            return Err("usage: rmac-session-supervisor <status|write-health|monitor|observe-failure UNIT|clear-safe-mode>".into());
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
