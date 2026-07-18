//! GNOME interface text scaling for GTK applications.
//!
//! GNOME's `org.gnome.desktop.interface` GSettings schema is the authority.
//! Mutations are accepted only when the key is writable and the value read
//! back from the same authority matches the requested factor.

use std::fmt;
use std::io::{self, Read as _};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SCHEMA: &str = "org.gnome.desktop.interface";
const TEXT_SCALE_KEY: &str = "text-scaling-factor";
const MIN_FACTOR: f64 = 1.0;
const MAX_FACTOR: f64 = 2.0;
const READBACK_EPSILON: f64 = 0.001;
const MAX_COMMAND_OUTPUT_BYTES: usize = 4 * 1024;
const MAX_ERROR_BYTES: usize = 512;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
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
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
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

struct CommandOutput {
    success: bool,
    stdout: String,
}

trait Runner {
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

fn snapshot_with(runner: &impl Runner) -> Result<Snapshot, Error> {
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

fn command_error(operation: &'static str, error: io::Error) -> Error {
    let detail = match error.kind() {
        io::ErrorKind::NotFound => "gsettings is not installed",
        io::ErrorKind::TimedOut => "gsettings did not finish before the bounded deadline",
        io::ErrorKind::InvalidData => "gsettings returned invalid or excessive output",
        io::ErrorKind::PermissionDenied => "permission was denied while running gsettings",
        _ => "gsettings could not be run",
    };
    Error::new(operation, detail)
}

fn bounded_command_output(command: &mut Command) -> io::Result<CommandOutput> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing stderr pipe"))?;
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr));
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(PROCESS_POLL_INTERVAL),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "gsettings command timed out",
                ));
            }
        }
    };
    let (stdout, stdout_excessive) = stdout_reader
        .join()
        .map_err(|_| io::Error::other("stdout reader failed"))??;
    let (_, stderr_excessive) = stderr_reader
        .join()
        .map_err(|_| io::Error::other("stderr reader failed"))??;
    if stdout_excessive || stderr_excessive {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "gsettings output exceeded the limit",
        ));
    }
    let stdout = String::from_utf8(stdout)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "stdout is not UTF-8"))?;
    Ok(CommandOutput {
        success: status.success(),
        stdout,
    })
}

fn drain_bounded(mut reader: impl io::Read) -> io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_COMMAND_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let excessive = bytes.len() > MAX_COMMAND_OUTPUT_BYTES;
    bytes.truncate(MAX_COMMAND_OUTPUT_BYTES);
    // Keep draining so a child with excessive output cannot block on a full pipe.
    io::copy(&mut reader, &mut io::sink())?;
    Ok((bytes, excessive))
}

fn monitor_once(sender: &async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let mut child = Command::new("gsettings")
        .args(["monitor", SCHEMA, TEXT_SCALE_KEY])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| command_error("watch GTK text scaling", error))?;
    let mut stdout = child.stdout.take().ok_or_else(|| {
        Error::new(
            "watch GTK text scaling",
            "gsettings monitor did not provide an output stream",
        )
    })?;
    let _ = sender.try_send(WatchEvent::Available);
    let event_sender = sender.clone();
    let (reader_done_tx, reader_done_rx) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut buffer = [0_u8; 1024];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {
                    let _ = event_sender.try_send(WatchEvent::Changed);
                }
                Err(_) => break,
            }
        }
        let _ = reader_done_tx.send(());
    });
    loop {
        if sender.is_closed() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Ok(());
        }
        if reader_done_rx.try_recv().is_ok() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(Error::new(
                "watch GTK text scaling",
                "the gsettings monitor stream ended",
            ));
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                let _ = reader.join();
                return Err(Error::new(
                    "watch GTK text scaling",
                    "the gsettings monitor process ended",
                ));
            }
            Ok(None) => std::thread::sleep(PROCESS_POLL_INTERVAL),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(Error::new(
                    "watch GTK text scaling",
                    "the gsettings monitor could not be inspected",
                ));
            }
        }
    }
}

fn bounded_text(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut end = normalized.len().min(MAX_ERROR_BYTES);
    while !normalized.is_char_boundary(end) {
        end -= 1;
    }
    normalized[..end].trim().to_string()
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
            success("1.0\n"),
            success("true\n"),
            success(""),
            success("1.3\n"),
            success("true\n"),
        ]);
        let snapshot = set_text_scale_with(&runner, 1.3).unwrap();
        assert_eq!(snapshot.factor, 1.3);
        let calls = runner.calls.borrow();
        assert_eq!(calls[4], ["set", SCHEMA, TEXT_SCALE_KEY, "1.30"]);
    }

    #[test]
    fn rejects_a_readback_mismatch() {
        let runner = FakeRunner::new(vec![
            success("1.0\n"),
            success("true\n"),
            success("1.0\n"),
            success("true\n"),
            success(""),
            success("1.2\n"),
            success("true\n"),
        ]);
        let error = set_text_scale_with(&runner, 1.3).unwrap_err();
        assert!(error.to_string().contains("authority reports 1.20"));
    }

    #[test]
    fn refuses_a_value_or_policy_changed_during_preflight() {
        let runner = FakeRunner::new(vec![
            success("1.0\n"),
            success("true\n"),
            success("1.1\n"),
            success("true\n"),
        ]);
        let error = set_text_scale_with(&runner, 1.3).unwrap_err();
        assert!(error.to_string().contains("changed before save"));
        assert_eq!(runner.calls.borrow().len(), 4);
    }

    #[test]
    fn a_matching_value_is_a_noop() {
        let runner = FakeRunner::new(vec![success("1.3\n"), success("true\n")]);
        assert_eq!(set_text_scale_with(&runner, 1.3).unwrap().factor, 1.3);
        assert_eq!(runner.calls.borrow().len(), 2);
    }

    #[test]
    fn errors_are_bounded_and_control_normalized() {
        let error = Error::new("test", format!("{}\nprivate", "x".repeat(600)));
        assert!(error.to_string().len() <= MAX_ERROR_BYTES + "test: ".len());
        assert!(!error.to_string().contains('\n'));
    }
}
