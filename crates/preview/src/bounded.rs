//! Run a helper tool (the poppler utilities) with a deadline and an output
//! cap. A crafted PDF must not hang Preview or make it buffer without limit.

use std::ffi::OsString;
use std::io::Read as _;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Longest a poppler tool may run for one request.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(60);
/// Most bytes a tool may write to stdout (a 600 dpi A3 page as PPM is about
/// 140 MiB).
pub const MAX_TOOL_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
/// Most stderr bytes kept for the error message.
const MAX_TOOL_STDERR_BYTES: u64 = 16 * 1024;

#[derive(Debug)]
pub enum RunError {
    /// The tool could not start.
    Start(std::io::Error),
    /// The tool was still running at the deadline and was stopped.
    TimedOut,
    /// The tool wrote more than the cap and was stopped.
    TooLarge,
    /// The tool's output could not be read.
    Read(std::io::Error),
}

pub struct Finished {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn run(
    tool: &str,
    args: Vec<OsString>,
    timeout: Duration,
    max_output: u64,
) -> Result<Finished, RunError> {
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Start)?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        stop(&mut child);
        return Err(RunError::Read(std::io::Error::other(
            "tool output was unavailable",
        )));
    };
    let (done, finished) = mpsc::channel();
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(max_output.saturating_add(1))
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = done.send(());
        result
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.take(MAX_TOOL_STDERR_BYTES).read_to_end(&mut bytes);
        bytes
    });

    // stdout closes when the tool exits or has written more than the cap.
    if finished.recv_timeout(timeout).is_err() {
        stop(&mut child);
        return Err(RunError::TimedOut);
    }
    let stdout = match out.join() {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) => {
            stop(&mut child);
            return Err(RunError::Read(error));
        }
        Err(_) => {
            stop(&mut child);
            return Err(RunError::Read(std::io::Error::other(
                "tool output reader failed",
            )));
        }
    };
    if stdout.len() as u64 > max_output {
        stop(&mut child);
        return Err(RunError::TooLarge);
    }
    // A tool that closed stdout but keeps running is stopped at the deadline.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                stop(&mut child);
                return Err(RunError::TimedOut);
            }
            Err(error) => {
                stop(&mut child);
                return Err(RunError::Read(error));
            }
        }
    };
    let stderr = err.join().unwrap_or_default();
    Ok(Finished {
        status,
        stdout,
        stderr,
    })
}

fn stop(child: &mut Child) {
    // Killing an already-exited child fails harmlessly; wait reaps it.
    if child.kill().is_ok() || child.try_wait().ok().flatten().is_none() {
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn sh(script: &str) -> Vec<OsString> {
        vec!["-c".into(), script.into()]
    }

    #[cfg(unix)]
    #[test]
    fn a_quick_tool_returns_its_output_and_status() {
        let finished = run(
            "/bin/sh",
            sh("printf out; printf err >&2; exit 3"),
            Duration::from_secs(10),
            1024,
        )
        .unwrap();
        assert_eq!(finished.stdout, b"out");
        assert_eq!(finished.stderr, b"err");
        assert_eq!(finished.status.code(), Some(3));
    }

    // Windows has no `/bin/sh`; `bounded::run` itself is plain
    // `std::process::Command`, portable already, so these use
    // `powershell.exe`/`cmd.exe` to exercise the same properties instead
    // of skipping Windows coverage entirely.
    #[cfg(windows)]
    #[test]
    fn a_quick_tool_returns_its_output_and_status() {
        let finished = run(
            "cmd.exe",
            vec!["/C".into(), "echo out&echo err 1>&2&exit 3".into()],
            Duration::from_secs(10),
            1024,
        )
        .unwrap();
        assert!(finished.stdout.starts_with(b"out"), "{:?}", finished.stdout);
        assert!(finished.stderr.starts_with(b"err"), "{:?}", finished.stderr);
        assert_eq!(finished.status.code(), Some(3));
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_tool_is_stopped_at_the_deadline() {
        let started = Instant::now();
        let result = run(
            "/bin/sh",
            sh("exec sleep 30"),
            Duration::from_millis(200),
            1024,
        );
        assert!(matches!(result, Err(RunError::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(windows)]
    #[test]
    fn a_hung_tool_is_stopped_at_the_deadline() {
        let started = Instant::now();
        let result = run(
            "powershell.exe",
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "Start-Sleep -Seconds 30".into(),
            ],
            Duration::from_secs(1),
            1024,
        );
        assert!(matches!(result, Err(RunError::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(25));
    }

    // Unix-only: there is no simple one-liner on Windows that detaches a
    // process's own inherited stdout handle (what `exec >/dev/null` does
    // here) while it keeps running, and the timeout/kill path this also
    // exercises is already covered above.
    #[cfg(unix)]
    #[test]
    fn a_tool_that_closes_stdout_and_hangs_is_stopped() {
        let result = run(
            "/bin/sh",
            sh("exec >/dev/null; exec sleep 30"),
            Duration::from_millis(300),
            1024,
        );
        assert!(matches!(result, Err(RunError::TimedOut)));
    }

    #[cfg(unix)]
    #[test]
    fn output_past_the_cap_is_refused() {
        let result = run(
            "/bin/sh",
            sh("while :; do printf 0123456789; done"),
            Duration::from_secs(10),
            4096,
        );
        assert!(matches!(result, Err(RunError::TooLarge)));
    }

    #[cfg(windows)]
    #[test]
    fn output_past_the_cap_is_refused() {
        let result = run(
            "powershell.exe",
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "while ($true) { [Console]::Out.Write('0123456789') }".into(),
            ],
            Duration::from_secs(10),
            4096,
        );
        assert!(matches!(result, Err(RunError::TooLarge)));
    }

    #[test]
    fn a_missing_tool_reports_not_found() {
        match run(
            "/nonexistent/rmac-tool",
            Vec::new(),
            Duration::from_secs(1),
            1,
        ) {
            Err(RunError::Start(error)) => {
                assert_eq!(error.kind(), std::io::ErrorKind::NotFound)
            }
            _ => panic!("expected a start error"),
        }
    }
}
