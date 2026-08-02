//! Bounded external-command execution.

use super::*;

pub(super) fn validate_output_id(output: &str) -> Result<(), Error> {
    if output.trim().is_empty() || output.chars().any(char::is_control) {
        Err(Error::new("address display", "invalid output name"))
    } else {
        Ok(())
    }
}

pub(super) fn command(
    program: &'static str,
    arguments: &[&str],
    operation: &'static str,
) -> Result<String, Error> {
    let mut command = Command::new(program);
    command.args(arguments);
    let output = bounded_command_output(&mut command, operation)?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::new(
            operation,
            if detail.is_empty() {
                format!("{program} exited with {}", output.status)
            } else {
                detail
            },
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(super) struct CommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

pub(super) fn bounded_command_output(
    command: &mut Command,
    operation: &'static str,
) -> Result<CommandOutput, Error> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new(operation, "stdout was not captured"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::new(operation, "stderr was not captured"))?;
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout, MAX_COMMAND_OUTPUT_BYTES));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr, MAX_COMMAND_OUTPUT_BYTES));
    let deadline = std::time::Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(Error::new(
                    operation,
                    "the command did not finish within ten seconds",
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(Error::new(operation, error.to_string()));
            }
        }
    };
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| Error::new(operation, "the stdout reader stopped unexpectedly"))?
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| Error::new(operation, "the stderr reader stopped unexpectedly"))?
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let status = status?;
    if stdout_truncated || stderr_truncated {
        return Err(Error::new(
            operation,
            "the command output exceeded the 4 MiB safety limit",
        ));
    }
    Ok(CommandOutput {
        status,
        stdout,
        stderr,
    })
}

pub(super) fn drain_bounded(
    mut reader: impl std::io::Read,
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut captured = Vec::with_capacity(limit.min(8192));
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..read.min(remaining)]);
        truncated |= read > remaining;
    }
    Ok((captured, truncated))
}
