use std::fmt;
use std::io;
use std::process::Command;
use std::time::Duration;

use crate::process::bounded_command_output;
use crate::watch::{bounded_text, monitor_once};

pub(crate) const SCHEMA: &str = "org.gnome.desktop.interface";
pub(crate) const TEXT_SCALE_KEY: &str = "text-scaling-factor";
const MIN_FACTOR: f64 = 1.0;
const MAX_FACTOR: f64 = 2.0;
const READBACK_EPSILON: f64 = 0.001;
pub(crate) const MAX_COMMAND_OUTPUT_BYTES: usize = 4 * 1024;
pub(crate) const MAX_ERROR_BYTES: usize = 512;
pub(crate) const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const WATCH_RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Available,
    Changed,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub writable: bool,
    pub factor: f64,
    pub detail: Option<String>,
}

impl Snapshot {
    fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            available: false,
            writable: false,
            factor: 1.0,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    pub(crate) fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: bounded_text(&detail.into()),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) stdout: String,
}

pub(crate) trait Runner {
    fn run(&self, arguments: &[&str]) -> io::Result<CommandOutput>;
}

struct RealRunner;

impl Runner for RealRunner {
    fn run(&self, arguments: &[&str]) -> io::Result<CommandOutput> {
        bounded_command_output(Command::new("gsettings").args(arguments))
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    snapshot_with(&RealRunner)
}

pub fn set_text_scale(factor: f64) -> Result<Snapshot, Error> {
    set_text_scale_with(&RealRunner, factor)
}

/// Follow authoritative GSettings changes without retaining subprocess output.
/// The monitor is restarted after failure and exits once the receiver closes.
pub fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    while !sender.is_closed() {
        if monitor_once(&sender).is_err() {
            let _ = sender.try_send(WatchEvent::Unavailable);
        }
        if !sender.is_closed() {
            std::thread::sleep(WATCH_RECONNECT_DELAY);
        }
    }
    Ok(())
}

pub(crate) fn snapshot_with(runner: &impl Runner) -> Result<Snapshot, Error> {
    let value = match runner.run(&["get", SCHEMA, TEXT_SCALE_KEY]) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Snapshot::unavailable(
                "gsettings is not installed; GTK text scaling is unavailable",
            ));
        }
        Err(error) => return Err(command_error("read GTK text scaling", error)),
    };
    if !value.success {
        return Ok(Snapshot::unavailable(
            "the GNOME interface settings schema is unavailable",
        ));
    }
    let factor = parse_factor(&value.stdout)?;

    let writable = runner
        .run(&["writable", SCHEMA, TEXT_SCALE_KEY])
        .map_err(|error| command_error("check GTK text scaling policy", error))?;
    if !writable.success {
        return Err(Error::new(
            "check GTK text scaling policy",
            "gsettings could not inspect the key",
        ));
    }
    let can_write = match writable.stdout.trim() {
        "true" => true,
        "false" => false,
        _ => {
            return Err(Error::new(
                "check GTK text scaling policy",
                "gsettings returned an invalid writability result",
            ));
        }
    };
    Ok(Snapshot {
        available: true,
        writable: can_write,
        factor,
        detail: (!can_write)
            .then(|| "GTK text scaling is locked by the current GSettings policy".to_string()),
    })
}

pub(crate) fn set_text_scale_with(runner: &impl Runner, factor: f64) -> Result<Snapshot, Error> {
    validate_requested_factor(factor)?;
    let before = snapshot_with(runner)?;
    if !before.available {
        return Err(Error::new(
            "set GTK text scaling",
            before
                .detail
                .unwrap_or_else(|| "the GSettings authority is unavailable".to_string()),
        ));
    }
    if !before.writable {
        return Err(Error::new(
            "set GTK text scaling",
            before
                .detail
                .unwrap_or_else(|| "the GSettings key is read-only".to_string()),
        ));
    }
    if (before.factor - factor).abs() <= READBACK_EPSILON {
        return Ok(before);
    }

    let confirmed = snapshot_with(runner)?;
    if confirmed != before {
        return Err(Error::new(
            "set GTK text scaling",
            "the GSettings value or policy changed before save; refresh and try again",
        ));
    }

    let serialized = format!("{factor:.2}");
    let output = runner
        .run(&["set", SCHEMA, TEXT_SCALE_KEY, &serialized])
        .map_err(|error| command_error("set GTK text scaling", error))?;
    if !output.success {
        return Err(Error::new(
            "set GTK text scaling",
            "gsettings rejected the value",
        ));
    }

    let after = snapshot_with(runner)?;
    if (after.factor - factor).abs() > READBACK_EPSILON {
        return Err(Error::new(
            "confirm GTK text scaling",
            format!(
                "requested {factor:.2}, but the authority reports {:.2}",
                after.factor
            ),
        ));
    }
    Ok(after)
}

fn parse_factor(value: &str) -> Result<f64, Error> {
    let factor = value
        .trim()
        .parse::<f64>()
        .map_err(|_| Error::new("read GTK text scaling", "the factor is not a number"))?;
    if !factor.is_finite() || factor <= 0.0 || factor > 10.0 {
        return Err(Error::new(
            "read GTK text scaling",
            "the authority returned an invalid factor",
        ));
    }
    Ok(factor)
}

fn validate_requested_factor(factor: f64) -> Result<(), Error> {
    if !factor.is_finite() || !(MIN_FACTOR..=MAX_FACTOR).contains(&factor) {
        return Err(Error::new(
            "validate GTK text scaling",
            format!("factor must be between {MIN_FACTOR:.1} and {MAX_FACTOR:.1}"),
        ));
    }
    Ok(())
}

pub(crate) fn command_error(operation: &'static str, error: io::Error) -> Error {
    let detail = match error.kind() {
        io::ErrorKind::NotFound => "gsettings is not installed",
        io::ErrorKind::TimedOut => "gsettings did not finish before the bounded deadline",
        io::ErrorKind::InvalidData => "gsettings returned invalid or excessive output",
        io::ErrorKind::PermissionDenied => "permission was denied while running gsettings",
        _ => "gsettings could not be run",
    };
    Error::new(operation, detail)
}
