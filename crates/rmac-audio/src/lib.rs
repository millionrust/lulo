//! Cross-platform system audio state and controls.

use std::process::Command;

#[cfg(any(not(target_os = "macos"), test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod model;
mod notification;
#[cfg(test)]
mod tests;

#[cfg(any(not(target_os = "macos"), test))]
use linux::*;
#[cfg(target_os = "macos")]
use macos::*;
pub use model::*;
pub use notification::*;

pub fn snapshot() -> Result<Snapshot, Error> {
    system_snapshot()
}

pub fn default_device(kind: DeviceKind) -> Result<DefaultDevice, Error> {
    system_default_device(kind)
}

pub fn set_volume(kind: DeviceKind, volume: u8) -> Result<(), Error> {
    system_set_volume(kind, volume.min(100))
}

pub fn set_muted(kind: DeviceKind, muted: bool) -> Result<(), Error> {
    system_set_muted(kind, muted)
}

pub fn set_default_device(kind: DeviceKind, device: &Device) -> Result<Snapshot, Error> {
    system_set_default_device(kind, device)
}

pub fn set_profile(device: &HardwareDevice, profile: &Profile) -> Result<Snapshot, Error> {
    system_set_profile(device, profile)
}

pub fn set_route(kind: DeviceKind, device: &Device, route: &Route) -> Result<Snapshot, Error> {
    system_set_route(kind, device, route)
}

pub fn set_balance(device: &Device, value: i8) -> Result<Snapshot, Error> {
    system_set_balance(device, value.clamp(-100, 100))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    system_watch(sender).await
}

fn command(
    program: &'static str,
    arguments: &[&str],
    operation: &'static str,
) -> Result<String, Error> {
    use std::process::Stdio;

    const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
    let stdout_reader = std::thread::spawn(move || drain_bounded(stdout, OUTPUT_LIMIT));
    let stderr_reader = std::thread::spawn(move || drain_bounded(stderr, OUTPUT_LIMIT));
    let status = child.wait();
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| Error::new(operation, "the stdout reader stopped unexpectedly"))?
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| Error::new(operation, "the stderr reader stopped unexpectedly"))?
        .map_err(|error| Error::new(operation, error.to_string()))?;
    let status = status.map_err(|error| Error::new(operation, error.to_string()))?;
    if stdout_truncated || stderr_truncated {
        return Err(Error::new(
            operation,
            "the service response exceeded the 4 MiB safety limit",
        ));
    }
    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(Error::new(
            operation,
            if detail.is_empty() {
                format!("{program} exited with {status}")
            } else {
                detail
            },
        ));
    }
    Ok(String::from_utf8_lossy(&stdout).trim().to_string())
}

fn drain_bounded(mut reader: impl std::io::Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut captured = Vec::with_capacity(limit.min(8192));
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok((captured, truncated));
        }
        let remaining = limit.saturating_sub(captured.len());
        let keep = read.min(remaining);
        captured.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
}

fn sort_devices(devices: &mut [Device]) {
    devices.sort_by(|left, right| {
        right
            .is_default
            .cmp(&left.is_default)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
}
