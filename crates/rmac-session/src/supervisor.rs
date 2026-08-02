use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rmac_storage::{atomic_write, Failure};
use serde::Serialize;

use crate::model::SAFE_MODE_VERSION;
use crate::{
    CommandRunner, ComponentHealth, Error, Operation, ProcessRunner, SafeModeState, SessionHealth,
    StatePaths, COMPONENT_UNITS,
};

pub struct Supervisor<R = ProcessRunner> {
    paths: StatePaths,
    runner: R,
}

impl Supervisor<ProcessRunner> {
    pub fn from_environment() -> Result<Self, Error> {
        Ok(Self::new(StatePaths::from_environment()?, ProcessRunner))
    }
}

impl<R: CommandRunner> Supervisor<R> {
    pub fn new(paths: StatePaths, runner: R) -> Self {
        Self { paths, runner }
    }

    pub fn snapshot(&self) -> Result<SessionHealth, Error> {
        let mut components = Vec::with_capacity(COMPONENT_UNITS.len());
        for unit in COMPONENT_UNITS {
            components.push(self.component_health(unit)?);
        }
        Ok(SessionHealth {
            observed_at_unix_ms: now_unix_ms(),
            safe_mode: self.load_safe_mode()?,
            components,
        })
    }

    pub fn component_health(&self, unit: &str) -> Result<ComponentHealth, Error> {
        validate_component_unit(unit, &self.paths.health)?;
        let output = self
            .runner
            .output(
                "systemctl",
                &[
                    "--user",
                    "show",
                    unit,
                    "--no-pager",
                    "--property=Id,LoadState,ActiveState,SubState,Result,NRestarts,MainPID,ExecMainStatus",
                ],
            )
            .map_err(|error| Failure::from_io(Operation::RunSystemctl, &self.paths.health, error))?;
        if !output.status.success() && output.stdout.is_empty() {
            return Err(Failure::message_with_kind(
                Operation::RunSystemctl,
                &self.paths.health,
                io::ErrorKind::NotConnected,
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        let text = String::from_utf8(output.stdout).map_err(|error| {
            Failure::message(
                Operation::ParseSystemctl,
                &self.paths.health,
                error.to_string(),
            )
        })?;
        parse_component_health(&text, unit).map_err(|detail| {
            Failure::message(Operation::ParseSystemctl, &self.paths.health, detail)
        })
    }

    pub fn write_health(&self) -> Result<SessionHealth, Error> {
        let snapshot = self.snapshot()?;
        write_json(&self.paths.health, &snapshot, Operation::WriteHealth)?;
        Ok(snapshot)
    }

    pub fn observe_failure(&self, unit: &str) -> Result<bool, Error> {
        let health = self.component_health(unit)?;
        if !health.exhausted_restart_budget() {
            self.write_health()?;
            return Ok(false);
        }
        let state = SafeModeState {
            version: SAFE_MODE_VERSION,
            entered_at_unix_ms: now_unix_ms(),
            trigger_unit: unit.to_owned(),
            observed_restarts: health.restarts,
            reason: if health.result == "start-limit-hit" {
                "systemd start limit was reached".into()
            } else {
                "component exhausted the bounded restart budget".into()
            },
        };
        write_json(&self.paths.safe_mode, &state, Operation::WriteSafeMode)?;
        Ok(true)
    }

    pub fn load_safe_mode(&self) -> Result<Option<SafeModeState>, Error> {
        let bytes = match std::fs::read(&self.paths.safe_mode) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(Failure::from_io(
                    Operation::ReadSafeMode,
                    &self.paths.safe_mode,
                    error,
                ));
            }
        };
        let state: SafeModeState = serde_json::from_slice(&bytes).map_err(|error| {
            Failure::message(
                Operation::ReadSafeMode,
                &self.paths.safe_mode,
                error.to_string(),
            )
        })?;
        if state.version != SAFE_MODE_VERSION {
            return Err(Failure::message(
                Operation::ReadSafeMode,
                &self.paths.safe_mode,
                format!("unsupported safe-mode version {}", state.version),
            ));
        }
        validate_component_unit(&state.trigger_unit, &self.paths.safe_mode)?;
        Ok(Some(state))
    }

    pub fn clear_safe_mode(&self) -> Result<(), Error> {
        match std::fs::remove_file(&self.paths.safe_mode) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Failure::from_io(
                Operation::ClearSafeMode,
                &self.paths.safe_mode,
                error,
            )),
        }
    }
}

pub fn parse_component_health(text: &str, expected_unit: &str) -> Result<ComponentHealth, String> {
    let mut fields = std::collections::BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("malformed systemctl property: {line}"))?;
        fields.insert(key, value);
    }
    let unit = required(&fields, "Id")?;
    if unit != expected_unit {
        return Err(format!(
            "systemctl returned {unit} while querying {expected_unit}"
        ));
    }
    Ok(ComponentHealth {
        unit: unit.to_owned(),
        load_state: required(&fields, "LoadState")?.to_owned(),
        active_state: required(&fields, "ActiveState")?.to_owned(),
        sub_state: required(&fields, "SubState")?.to_owned(),
        result: required(&fields, "Result")?.to_owned(),
        restarts: parse_number(&fields, "NRestarts")?,
        main_pid: nonzero_number(&fields, "MainPID")?,
        exit_status: optional_number(&fields, "ExecMainStatus")?,
    })
}

fn required<'a>(
    fields: &'a std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<&'a str, String> {
    fields
        .get(key)
        .copied()
        .ok_or_else(|| format!("systemctl omitted {key}"))
}

fn parse_number<T: std::str::FromStr>(
    fields: &std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<T, String> {
    required(fields, key)?
        .parse()
        .map_err(|_| format!("systemctl returned an invalid {key}"))
}

fn nonzero_number(
    fields: &std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<Option<u32>, String> {
    Ok(match parse_number(fields, key)? {
        0 => None,
        value => Some(value),
    })
}

fn optional_number(
    fields: &std::collections::BTreeMap<&str, &str>,
    key: &str,
) -> Result<Option<i32>, String> {
    let value = required(fields, key)?;
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse()
            .map(Some)
            .map_err(|_| format!("systemctl returned an invalid {key}"))
    }
}

fn validate_component_unit(unit: &str, path: &Path) -> Result<(), Error> {
    if COMPONENT_UNITS.contains(&unit) {
        Ok(())
    } else {
        Err(Failure::message(
            Operation::ParseSystemctl,
            path,
            format!("unrecognized rmac component unit {unit}"),
        ))
    }
}

fn write_json(path: &Path, value: &impl Serialize, operation: Operation) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| {
        Failure::message(Operation::ResolvePath, path, "state path has no parent")
    })?;
    std::fs::create_dir_all(parent).map_err(|error| Failure::from_io(operation, parent, error))?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| Failure::message(operation, path, error.to_string()))?;
    atomic_write(path, &bytes).map_err(|error| Failure::from_io(operation, path, error))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
