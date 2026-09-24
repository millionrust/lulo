use std::io;
use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::api::{CommandOutput, COMMAND_TIMEOUT, MAX_COMMAND_OUTPUT_BYTES, PROCESS_POLL_INTERVAL};

pub(crate) fn bounded_command_output(command: &mut Command) -> io::Result<CommandOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = rmac_process::bind_to_parent(command).spawn()?;
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
