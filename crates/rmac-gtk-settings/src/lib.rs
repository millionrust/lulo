//! GNOME interface text scaling for GTK applications.
//!
//! GNOME's `org.gnome.desktop.interface` GSettings schema is the authority.
//! Mutations are accepted only when the key is writable and the value read
//! back from the same authority matches the requested factor.

use std::fmt;
use std::io;
use std::process::Command;

const SCHEMA: &str = "org.gnome.desktop.interface";
const TEXT_SCALE_KEY: &str = "text-scaling-factor";
const MIN_FACTOR: f64 = 1.0;
const MAX_FACTOR: f64 = 2.0;
const READBACK_EPSILON: f64 = 0.001;

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
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

trait Runner {
    fn run(&self, arguments: &[&str]) -> io::Result<CommandOutput>;
}

struct RealRunner;

impl Runner for RealRunner {
    fn run(&self, arguments: &[&str]) -> io::Result<CommandOutput> {
        let output = Command::new("gsettings").args(arguments).output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    snapshot_with(&RealRunner)
}

pub fn set_text_scale(factor: f64) -> Result<Snapshot, Error> {
    set_text_scale_with(&RealRunner, factor)
}

fn snapshot_with(runner: &impl Runner) -> Result<Snapshot, Error> {
    let value = match runner.run(&["get", SCHEMA, TEXT_SCALE_KEY]) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Snapshot::unavailable(
                "gsettings is not installed; GTK text scaling is unavailable",
            ));
        }
        Err(error) => return Err(Error::new("read GTK text scaling", error.to_string())),
    };
    if !value.success {
        return Ok(Snapshot::unavailable(command_failure(
            &value,
            "the GNOME interface settings schema is unavailable",
        )));
    }
    let factor = parse_factor(&value.stdout)?;

    let writable = runner
        .run(&["writable", SCHEMA, TEXT_SCALE_KEY])
        .map_err(|error| Error::new("check GTK text scaling policy", error.to_string()))?;
    if !writable.success {
        return Err(Error::new(
            "check GTK text scaling policy",
            command_failure(&writable, "gsettings could not inspect the key"),
        ));
    }
    let can_write = match writable.stdout.trim() {
        "true" => true,
        "false" => false,
        value => {
            return Err(Error::new(
                "check GTK text scaling policy",
                format!("unexpected writable result {value:?}"),
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

fn set_text_scale_with(runner: &impl Runner, factor: f64) -> Result<Snapshot, Error> {
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

    let serialized = format!("{factor:.2}");
    let output = runner
        .run(&["set", SCHEMA, TEXT_SCALE_KEY, &serialized])
        .map_err(|error| Error::new("set GTK text scaling", error.to_string()))?;
    if !output.success {
        return Err(Error::new(
            "set GTK text scaling",
            command_failure(&output, "gsettings rejected the value"),
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
            format!("the authority returned invalid factor {factor}"),
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

fn command_failure(output: &CommandOutput, fallback: &str) -> String {
    let stderr = output.stderr.trim();
    if stderr.is_empty() {
        fallback.to_string()
    } else {
        stderr.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct FakeRunner {
        outputs: RefCell<VecDeque<io::Result<CommandOutput>>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl FakeRunner {
        fn new(outputs: Vec<io::Result<CommandOutput>>) -> Self {
            Self {
                outputs: RefCell::new(outputs.into()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, arguments: &[&str]) -> io::Result<CommandOutput> {
            self.calls
                .borrow_mut()
                .push(arguments.iter().map(|value| (*value).to_string()).collect());
            self.outputs
                .borrow_mut()
                .pop_front()
                .expect("unexpected gsettings call")
        }
    }

    fn success(stdout: &str) -> io::Result<CommandOutput> {
        Ok(CommandOutput {
            success: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
    }

    #[test]
    fn missing_gsettings_is_an_unavailable_snapshot() {
        let runner = FakeRunner::new(vec![Err(io::Error::new(
            io::ErrorKind::NotFound,
            "missing",
        ))]);
        let snapshot = snapshot_with(&runner).unwrap();
        assert!(!snapshot.available);
        assert!(!snapshot.writable);
        assert!(snapshot.detail.unwrap().contains("not installed"));
    }

    #[test]
    fn reads_a_policy_locked_factor() {
        let runner = FakeRunner::new(vec![success("1.2\n"), success("false\n")]);
        let snapshot = snapshot_with(&runner).unwrap();
        assert!(snapshot.available);
        assert!(!snapshot.writable);
        assert_eq!(snapshot.factor, 1.2);
        assert!(snapshot.detail.unwrap().contains("locked"));
    }

    #[test]
    fn rejects_invalid_requested_factors_before_running_a_command() {
        let runner = FakeRunner::new(Vec::new());
        assert!(set_text_scale_with(&runner, 0.9).is_err());
        assert!(set_text_scale_with(&runner, f64::NAN).is_err());
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn sets_and_confirms_the_authoritative_value() {
        let runner = FakeRunner::new(vec![
            success("1.0\n"),
            success("true\n"),
            success(""),
            success("1.3\n"),
            success("true\n"),
        ]);
        let snapshot = set_text_scale_with(&runner, 1.3).unwrap();
        assert_eq!(snapshot.factor, 1.3);
        let calls = runner.calls.borrow();
        assert_eq!(calls[2], ["set", SCHEMA, TEXT_SCALE_KEY, "1.30"]);
    }

    #[test]
    fn rejects_a_readback_mismatch() {
        let runner = FakeRunner::new(vec![
            success("1.0\n"),
            success("true\n"),
            success(""),
            success("1.2\n"),
            success("true\n"),
        ]);
        let error = set_text_scale_with(&runner, 1.3).unwrap_err();
        assert!(error.to_string().contains("authority reports 1.20"));
    }
}
