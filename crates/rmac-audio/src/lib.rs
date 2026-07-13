//! Cross-platform system audio state and controls.

use std::fmt;
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Output,
    Input,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Device {
    /// Opaque identifier for this node in the currently sampled audio graph.
    pub id: String,
    pub name: String,
    pub is_default: bool,
    authority_name: String,
}

impl fmt::Debug for Device {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Device")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("is_default", &self.is_default)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub volume: u8,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub available: bool,
    pub can_set_default: bool,
    pub can_mute_input: bool,
    pub output: Level,
    pub input: Level,
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
}

#[derive(Debug)]
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
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

pub fn snapshot() -> Result<Snapshot, Error> {
    system_snapshot()
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchEvent {
    Changed,
    Unavailable,
}

pub async fn watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    system_watch(sender).await
}

#[cfg(not(target_os = "macos"))]
const WATCH_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
const WATCH_QUIET_PERIOD: std::time::Duration = std::time::Duration::from_millis(75);
#[cfg(not(target_os = "macos"))]
const WATCH_MAX_COALESCE: std::time::Duration = std::time::Duration::from_millis(250);
#[cfg(not(target_os = "macos"))]
const MUTATION_VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(not(target_os = "macos"))]
const MUTATION_VERIFY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

#[cfg(not(target_os = "macos"))]
async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    let mut unavailable_reported = false;
    loop {
        match watch_once(&sender, &mut unavailable_reported).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {}
            Err(_) if sender.is_closed() => return Ok(()),
            Err(_) => publish_unavailable(&sender, &mut unavailable_reported).await?,
        }
        async_io::Timer::after(WATCH_RECONNECT_DELAY).await;
    }
}

#[cfg(target_os = "macos")]
async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    sender
        .send(WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch audio changes", "the event consumer closed"))
}

#[cfg(not(target_os = "macos"))]
async fn watch_once(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    use std::process::Stdio;

    use futures_lite::io::AsyncReadExt as _;

    let mut command = async_process::Command::new("pw-mon");
    command
        .arg("--color=never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| Error::new("start the PipeWire monitor", error.to_string()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("start the PipeWire monitor", "stdout was not captured"))?;
    let mut buffer = [0_u8; 8192];

    loop {
        let next = futures_util::FutureExt::fuse(stdout.read(&mut buffer));
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(next, closed);
        let read = futures_util::select! {
            read = next => read,
            _ = closed => return Ok(()),
        }
        .map_err(|error| Error::new("read PipeWire changes", error.to_string()))?;
        if read == 0 {
            return monitor_status_error(child).await;
        }

        let flush_deadline = std::time::Instant::now() + WATCH_MAX_COALESCE;
        loop {
            let remaining = flush_deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let next = futures_util::FutureExt::fuse(stdout.read(&mut buffer));
            let quiet = futures_util::FutureExt::fuse(async_io::Timer::after(WATCH_QUIET_PERIOD));
            let maximum = futures_util::FutureExt::fuse(async_io::Timer::after(remaining));
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(next, quiet, maximum, closed);
            futures_util::select! {
                read = next => {
                    let read = read.map_err(|error| Error::new("read PipeWire changes", error.to_string()))?;
                    if read == 0 {
                        return monitor_status_error(child).await;
                    }
                },
                _ = quiet => break,
                _ = maximum => break,
                _ = closed => return Ok(()),
            }
        }
        publish_changed(sender, unavailable_reported).await?;
    }
}

#[cfg(not(target_os = "macos"))]
async fn monitor_status_error(mut child: async_process::Child) -> Result<(), Error> {
    let status = child
        .status()
        .await
        .map_err(|error| Error::new("wait for the PipeWire monitor", error.to_string()))?;
    Err(Error::new(
        "watch PipeWire changes",
        format!("pw-mon exited with {status}"),
    ))
}

#[cfg(not(target_os = "macos"))]
async fn publish_changed(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if *unavailable_reported {
        sender
            .send(WatchEvent::Changed)
            .await
            .map_err(|_| Error::new("publish audio recovery", "the event consumer closed"))?;
    } else {
        let _ = sender.try_send(WatchEvent::Changed);
    }
    *unavailable_reported = false;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn publish_unavailable(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    if !*unavailable_reported {
        sender
            .send(WatchEvent::Unavailable)
            .await
            .map_err(|_| Error::new("publish audio outage", "the event consumer closed"))?;
        *unavailable_reported = true;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    let mut outputs = machine_devices(DeviceKind::Output)?;
    let mut inputs = machine_devices(DeviceKind::Input)?;
    let descriptions = command(
        "pw-dump",
        &["--no-colors"],
        "read PipeWire device descriptions",
    )
    .ok()
    .map(|dump| parse_pw_dump_descriptions(&dump))
    .unwrap_or_default();
    apply_device_descriptions(&mut outputs, &descriptions);
    apply_device_descriptions(&mut inputs, &descriptions);
    let output_id = default_device_id(&outputs).ok_or_else(|| {
        Error::new(
            "read output volume",
            "WirePlumber did not advertise a default output device",
        )
    })?;
    let input_id = default_device_id(&inputs).ok_or_else(|| {
        Error::new(
            "read input volume",
            "WirePlumber did not advertise a default input device",
        )
    })?;
    let output = parse_wpctl_level(&command(
        "wpctl",
        &["get-volume", output_id],
        "read output volume",
    )?)
    .ok_or_else(|| Error::new("read output volume", "unexpected wpctl response"))?;
    let input = parse_wpctl_level(&command(
        "wpctl",
        &["get-volume", input_id],
        "read input volume",
    )?)
    .ok_or_else(|| Error::new("read input volume", "unexpected wpctl response"))?;
    sort_devices(&mut outputs);
    sort_devices(&mut inputs);
    Ok(Snapshot {
        available: true,
        can_set_default: true,
        can_mute_input: true,
        output,
        input,
        outputs,
        inputs,
    })
}

#[cfg(not(target_os = "macos"))]
fn machine_devices(kind: DeviceKind) -> Result<Vec<Device>, Error> {
    let (object_type, operation) = match kind {
        DeviceKind::Output => ("sinks", "read PipeWire output devices"),
        DeviceKind::Input => ("sources", "read PipeWire input devices"),
    };
    parse_wpctl_list(
        &command("wpctl", &["list", "audio", object_type], operation)?,
        kind,
    )
}

#[cfg(not(target_os = "macos"))]
fn system_set_volume(kind: DeviceKind, volume: u8) -> Result<(), Error> {
    let target = wpctl_default_target(kind);
    let value = format!("{:.2}", f32::from(volume) / 100.0);
    command(
        "wpctl",
        &["set-volume", target, &value],
        match kind {
            DeviceKind::Output => "change output volume",
            DeviceKind::Input => "change input volume",
        },
    )?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn system_set_muted(kind: DeviceKind, muted: bool) -> Result<(), Error> {
    command(
        "wpctl",
        &[
            "set-mute",
            wpctl_default_target(kind),
            if muted { "1" } else { "0" },
        ],
        match kind {
            DeviceKind::Output => "change output mute",
            DeviceKind::Input => "change input mute",
        },
    )?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn system_set_default_device(kind: DeviceKind, expected: &Device) -> Result<Snapshot, Error> {
    if expected.id.is_empty()
        || !expected
            .id
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(Error::new("change default audio device", "invalid node id"));
    }
    let devices = machine_devices(kind)?;
    let current = devices
        .iter()
        .find(|device| device.id == expected.id)
        .ok_or_else(|| {
            Error::new(
                "change default audio device",
                "the selected node is no longer available",
            )
        })?;
    if current.authority_name != expected.authority_name {
        return Err(Error::new(
            "change default audio device",
            "the selected node identity changed; refresh Sound before trying again",
        ));
    }
    if current.is_default {
        return system_snapshot();
    }
    command(
        "wpctl",
        &["set-default", &expected.id],
        "change default audio device",
    )?;
    let deadline = std::time::Instant::now() + MUTATION_VERIFY_TIMEOUT;
    loop {
        let devices = machine_devices(kind)?;
        let current = devices
            .iter()
            .find(|device| device.id == expected.id)
            .ok_or_else(|| {
                Error::new(
                    "verify the default audio device",
                    "the selected node disappeared after the change",
                )
            })?;
        if current.authority_name != expected.authority_name {
            return Err(Error::new(
                "verify the default audio device",
                "the selected node identity changed after the request",
            ));
        }
        if current.is_default {
            let snapshot = system_snapshot()?;
            let verified = match kind {
                DeviceKind::Output => &snapshot.outputs,
                DeviceKind::Input => &snapshot.inputs,
            }
            .iter()
            .any(|device| {
                device.id == expected.id
                    && device.authority_name == expected.authority_name
                    && device.is_default
            });
            if verified {
                return Ok(snapshot);
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                "verify the default audio device",
                "WirePlumber did not confirm the requested default within three seconds",
            ));
        }
        std::thread::sleep(MUTATION_VERIFY_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
fn wpctl_default_target(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Output => "@DEFAULT_AUDIO_SINK@",
        DeviceKind::Input => "@DEFAULT_AUDIO_SOURCE@",
    }
}

#[cfg(target_os = "macos")]
fn system_snapshot() -> Result<Snapshot, Error> {
    let profiler = command(
        "system_profiler",
        &["SPAudioDataType"],
        "read audio devices",
    )?;
    let (mut outputs, mut inputs) = parse_macos_audio_devices(&profiler);
    let settings = command(
        "osascript",
        &["-e", "get volume settings"],
        "read audio volume",
    )?;
    let (output, input) = parse_macos_volume_settings(&settings)
        .ok_or_else(|| Error::new("read audio volume", "unexpected osascript response"))?;
    sort_devices(&mut outputs);
    sort_devices(&mut inputs);
    Ok(Snapshot {
        available: true,
        can_set_default: false,
        can_mute_input: false,
        output,
        input,
        outputs,
        inputs,
    })
}

#[cfg(target_os = "macos")]
fn system_set_volume(kind: DeviceKind, volume: u8) -> Result<(), Error> {
    let script = match kind {
        DeviceKind::Output => format!("set volume output volume {volume}"),
        DeviceKind::Input => format!("set volume input volume {volume}"),
    };
    command("osascript", &["-e", &script], "change audio volume")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn system_set_muted(kind: DeviceKind, muted: bool) -> Result<(), Error> {
    if kind == DeviceKind::Input {
        return Err(Error::new(
            "change input mute",
            "macOS does not expose input mute through the scripting adapter",
        ));
    }
    let script = format!("set volume output muted {muted}");
    command("osascript", &["-e", &script], "change output mute")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn system_set_default_device(_: DeviceKind, _: &Device) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change default audio device",
        "macOS does not expose device selection through the scripting adapter",
    ))
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

#[cfg(any(not(target_os = "macos"), test))]
fn parse_wpctl_level(output: &str) -> Option<Level> {
    let volume = output
        .split_whitespace()
        .find_map(|field| field.parse::<f32>().ok())?;
    Some(Level {
        volume: (volume * 100.0).round().clamp(0.0, 100.0) as u8,
        muted: output.contains("[MUTED]"),
    })
}

#[cfg(any(not(target_os = "macos"), test))]
const MAX_AUDIO_DEVICES: usize = 256;
#[cfg(any(not(target_os = "macos"), test))]
const MAX_AUTHORITY_NAME_BYTES: usize = 512;
#[cfg(any(not(target_os = "macos"), test))]
const MAX_DEVICE_LABEL_CHARS: usize = 256;

#[cfg(any(not(target_os = "macos"), test))]
fn parse_wpctl_list(output: &str, kind: DeviceKind) -> Result<Vec<Device>, Error> {
    use std::collections::HashSet;

    let expected_class = match kind {
        DeviceKind::Output => "audio/sink",
        DeviceKind::Input => "audio/source",
    };
    let operation = match kind {
        DeviceKind::Output => "read PipeWire output devices",
        DeviceKind::Input => "read PipeWire input devices",
    };
    let mut seen = HashSet::new();
    let mut devices = Vec::new();
    let mut defaults = 0;
    for raw in output.lines().filter(|line| !line.trim().is_empty()) {
        if devices.len() == MAX_AUDIO_DEVICES {
            return Err(Error::new(
                operation,
                "the device list exceeded 256 entries",
            ));
        }
        let fields = raw.trim_end_matches('\r').split('\t').collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(Error::new(
                operation,
                "wpctl list returned a row without four tab-separated fields",
            ));
        }
        let id = fields[0];
        if id.parse::<u32>().ok().filter(|id| *id > 0).is_none() || !seen.insert(id) {
            return Err(Error::new(
                operation,
                "wpctl list returned an invalid or duplicate object id",
            ));
        }
        let authority_name = fields[1];
        if authority_name.is_empty()
            || authority_name.len() > MAX_AUTHORITY_NAME_BYTES
            || authority_name.chars().any(char::is_control)
        {
            return Err(Error::new(
                operation,
                "wpctl list returned an invalid node name",
            ));
        }
        if fields[2] != expected_class {
            return Err(Error::new(
                operation,
                "wpctl list returned an unexpected media class",
            ));
        }
        let is_default = match fields[3] {
            "*" => true,
            marker if marker.chars().all(char::is_whitespace) => false,
            _ => {
                return Err(Error::new(
                    operation,
                    "wpctl list returned an invalid default marker",
                ));
            }
        };
        defaults += usize::from(is_default);
        if defaults > 1 {
            return Err(Error::new(
                operation,
                "WirePlumber advertised more than one default node",
            ));
        }
        devices.push(Device {
            id: id.to_owned(),
            name: bounded_label(authority_name),
            is_default,
            authority_name: authority_name.to_owned(),
        });
    }
    Ok(devices)
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_pw_dump_descriptions(output: &str) -> std::collections::HashMap<String, String> {
    use serde_json::Value;

    let Ok(Value::Array(objects)) = serde_json::from_str::<Value>(output) else {
        return std::collections::HashMap::new();
    };
    objects
        .into_iter()
        .filter_map(|object| {
            let id = object.get("id")?.as_u64()?;
            if id == 0 || id > u64::from(u32::MAX) {
                return None;
            }
            let object_type = object.get("type")?.as_str()?;
            if object_type != "PipeWire:Interface:Node" {
                return None;
            }
            let props = object.get("info")?.get("props")?;
            let media_class = props.get("media.class")?.as_str()?;
            if media_class != "Audio/Sink" && media_class != "Audio/Source" {
                return None;
            }
            let description = ["node.description", "node.nick", "node.name"]
                .into_iter()
                .find_map(|key| props.get(key).and_then(Value::as_str))?;
            let description = bounded_label(description);
            (!description.is_empty()).then(|| (id.to_string(), description))
        })
        .take(MAX_AUDIO_DEVICES * 2)
        .collect()
}

#[cfg(any(not(target_os = "macos"), test))]
fn bounded_label(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DEVICE_LABEL_CHARS)
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn apply_device_descriptions(
    devices: &mut [Device],
    descriptions: &std::collections::HashMap<String, String>,
) {
    for device in devices {
        if let Some(description) = descriptions.get(&device.id) {
            device.name.clone_from(description);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn default_device_id(devices: &[Device]) -> Option<&str> {
    devices
        .iter()
        .find(|device| device.is_default)
        .map(|device| device.id.as_str())
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_audio_devices(output: &str) -> (Vec<Device>, Vec<Device>) {
    let mut outputs = Vec::new();
    let mut inputs = Vec::new();
    let mut current = None;
    let mut has_output = false;
    let mut has_input = false;
    let mut default_output = false;
    let mut default_input = false;
    let flush = |current: &mut Option<String>,
                 has_output: bool,
                 has_input: bool,
                 default_output: bool,
                 default_input: bool,
                 outputs: &mut Vec<Device>,
                 inputs: &mut Vec<Device>| {
        if let Some(name) = current.take() {
            if has_output {
                outputs.push(Device {
                    id: name.clone(),
                    name: name.clone(),
                    is_default: default_output,
                    authority_name: name.clone(),
                });
            }
            if has_input {
                inputs.push(Device {
                    id: name.clone(),
                    name: name.clone(),
                    is_default: default_input,
                    authority_name: name,
                });
            }
        }
    };
    for raw in output.lines() {
        let indentation = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        if indentation == 8 && line.ends_with(':') {
            flush(
                &mut current,
                has_output,
                has_input,
                default_output,
                default_input,
                &mut outputs,
                &mut inputs,
            );
            current = Some(line.trim_end_matches(':').to_string());
            has_output = false;
            has_input = false;
            default_output = false;
            default_input = false;
        } else if current.is_some() {
            if line.starts_with("Output Source:") {
                has_output = true;
            }
            if line.starts_with("Input Source:") {
                has_input = true;
            }
            if line == "Default Output Device: Yes" {
                has_output = true;
                default_output = true;
            }
            if line == "Default Input Device: Yes" {
                has_input = true;
                default_input = true;
            }
        }
    }
    flush(
        &mut current,
        has_output,
        has_input,
        default_output,
        default_input,
        &mut outputs,
        &mut inputs,
    );
    (outputs, inputs)
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_volume_settings(output: &str) -> Option<(Level, Level)> {
    let field = |name: &str| {
        output.split(',').find_map(|part| {
            part.trim()
                .strip_prefix(name)
                .and_then(|value| value.trim().parse::<u8>().ok())
        })
    };
    let muted = output
        .split(',')
        .find_map(|part| part.trim().strip_prefix("output muted:"))
        .is_some_and(|value| value.trim() == "true");
    Some((
        Level {
            volume: field("output volume:")?.min(100),
            muted,
        },
        Level {
            volume: field("input volume:").unwrap_or(0).min(100),
            muted: false,
        },
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl_level_parses_volume_and_mute() {
        assert_eq!(
            parse_wpctl_level("Volume: 0.72 [MUTED]"),
            Some(Level {
                volume: 72,
                muted: true
            })
        );
        assert_eq!(parse_wpctl_level("Volume: 1.5").unwrap().volume, 100);
    }

    #[test]
    fn machine_readable_wpctl_lists_preserve_exact_identity_and_defaults() {
        let mut outputs = parse_wpctl_list(
            "61\talsa_output.hdmi\taudio/sink\t \n52\talsa_output.analog\taudio/sink\t*",
            DeviceKind::Output,
        )
        .unwrap();
        let mut inputs =
            parse_wpctl_list("53\talsa_input.analog\taudio/source\t*", DeviceKind::Input).unwrap();
        sort_devices(&mut outputs);
        sort_devices(&mut inputs);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].id, "52");
        assert!(outputs[0].is_default);
        assert_eq!(outputs[0].authority_name, "alsa_output.analog");
        assert_eq!(inputs[0].authority_name, "alsa_input.analog");
        assert!(
            parse_wpctl_list("53\talsa_input.analog\taudio/sink\t*", DeviceKind::Input).is_err()
        );
    }

    #[test]
    fn machine_readable_wpctl_list_rejects_ambiguous_or_malformed_identity() {
        for invalid in [
            "52\talsa_output.analog\taudio/sink\t*\n52\talsa_output.hdmi\taudio/sink\t ",
            "52\talsa_output.analog\taudio/sink\t*\n61\talsa_output.hdmi\taudio/sink\t*",
            "52\talsa_output.analog\taudio/sink\t?",
            "0\talsa_output.analog\taudio/sink\t*",
            "52\talsa_output.analog\taudio/sink",
        ] {
            assert!(parse_wpctl_list(invalid, DeviceKind::Output).is_err());
        }
    }

    #[test]
    fn device_debug_output_does_not_disclose_private_authority_name() {
        let device = Device {
            id: "52".into(),
            name: "Built-in Audio".into(),
            is_default: true,
            authority_name: "alsa_output.private-hardware-identity".into(),
        };
        let output = format!("{device:?}");
        assert!(output.contains("Built-in Audio"));
        assert!(output.contains("has_authority_name: true"));
        assert!(!output.contains("private-hardware-identity"));
    }

    #[test]
    fn pipewire_json_adds_only_bounded_friendly_node_descriptions() {
        let descriptions = parse_pw_dump_descriptions(
            r#"[
                {"id":52,"type":"PipeWire:Interface:Node","info":{"props":{
                    "media.class":"Audio/Sink","node.name":"alsa_output.analog",
                    "node.description":"Built-in Audio Analog Stereo"}}},
                {"id":53,"type":"PipeWire:Interface:Node","info":{"props":{
                    "media.class":"Stream/Output/Audio","node.description":"Private Stream"}}}
            ]"#,
        );
        assert_eq!(
            descriptions.get("52").map(String::as_str),
            Some("Built-in Audio Analog Stereo")
        );
        assert!(!descriptions.contains_key("53"));
        assert!(parse_pw_dump_descriptions("not json").is_empty());
    }

    #[test]
    fn command_output_reader_drains_after_the_capture_limit() {
        let input = std::io::Cursor::new(vec![7_u8; 32]);
        let (captured, truncated) = drain_bounded(input, 8).unwrap();
        assert_eq!(captured, vec![7_u8; 8]);
        assert!(truncated);
    }

    #[test]
    fn macos_volume_fixture_preserves_input_and_output() {
        let (output, input) = parse_macos_volume_settings(
            "output volume:67, input volume:44, alert volume:100, output muted:true",
        )
        .unwrap();
        assert_eq!(output.volume, 67);
        assert!(output.muted);
        assert_eq!(input.volume, 44);
    }

    #[test]
    fn macos_device_fixture_identifies_defaults() {
        let (outputs, inputs) = parse_macos_audio_devices(concat!(
            "Audio:\n\n",
            "        MacBook Pro Speakers:\n\n",
            "          Default Output Device: Yes\n",
            "          Output Source: MacBook Pro Speakers\n\n",
            "        MacBook Pro Microphone:\n\n",
            "          Default Input Device: Yes\n",
            "          Input Source: MacBook Pro Microphone",
        ));
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "MacBook Pro Speakers");
        assert!(outputs[0].is_default);
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].is_default);
    }

    #[test]
    fn errors_keep_operation_context() {
        let error = Error::new("read audio devices", "service unavailable");
        assert_eq!(
            error.to_string(),
            "could not read audio devices: service unavailable"
        );
    }
}
