//! Cross-platform system audio state and controls.

use std::fmt;
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Output,
    Input,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// Stable identifier owned by the platform audio service.
    pub id: String,
    pub name: String,
    pub is_default: bool,
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

pub fn set_default_device(kind: DeviceKind, id: &str) -> Result<(), Error> {
    system_set_default_device(kind, id)
}

#[cfg(not(target_os = "macos"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    let status = command("wpctl", &["status"], "read PipeWire devices")?;
    let (mut outputs, mut inputs) = parse_wpctl_status(&status);
    let output = parse_wpctl_level(&command(
        "wpctl",
        &["get-volume", "@DEFAULT_AUDIO_SINK@"],
        "read output volume",
    )?)
    .ok_or_else(|| Error::new("read output volume", "unexpected wpctl response"))?;
    let input = parse_wpctl_level(&command(
        "wpctl",
        &["get-volume", "@DEFAULT_AUDIO_SOURCE@"],
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
fn system_set_default_device(_: DeviceKind, id: &str) -> Result<(), Error> {
    if id.is_empty() || !id.chars().all(|character| character.is_ascii_digit()) {
        return Err(Error::new("change default audio device", "invalid node id"));
    }
    command("wpctl", &["set-default", id], "change default audio device")?;
    Ok(())
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
fn system_set_default_device(_: DeviceKind, _: &str) -> Result<(), Error> {
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
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| Error::new(operation, error.to_string()))?;
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
fn parse_wpctl_status(output: &str) -> (Vec<Device>, Vec<Device>) {
    let mut kind = None;
    let mut outputs = Vec::new();
    let mut inputs = Vec::new();
    for raw in output.lines() {
        let line = raw
            .trim()
            .trim_start_matches(|character: char| {
                character.is_whitespace() || matches!(character, '│' | '├' | '─' | '└')
            })
            .trim();
        if line == "Sinks:" {
            kind = Some(DeviceKind::Output);
            continue;
        }
        if line == "Sources:" {
            kind = Some(DeviceKind::Input);
            continue;
        }
        if line.ends_with(':') {
            kind = None;
            continue;
        }
        let Some(current_kind) = kind else {
            continue;
        };
        let is_default = line.starts_with('*');
        let line = line.trim_start_matches('*').trim();
        let Some((id, description)) = line.split_once('.') else {
            continue;
        };
        if id.is_empty() || !id.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }
        let name = description
            .split_once(" [vol:")
            .map_or(description, |(name, _)| name)
            .trim();
        if name.is_empty() {
            continue;
        }
        let device = Device {
            id: id.to_string(),
            name: name.to_string(),
            is_default,
        };
        match current_kind {
            DeviceKind::Output => outputs.push(device),
            DeviceKind::Input => inputs.push(device),
        }
    }
    (outputs, inputs)
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
                });
            }
            if has_input {
                inputs.push(Device {
                    id: name.clone(),
                    name,
                    is_default: default_input,
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
    fn wpctl_status_extracts_and_sorts_audio_nodes() {
        let (mut outputs, mut inputs) = parse_wpctl_status(
            "Audio\n\
             ├─ Sinks:\n\
             │      61. HDMI / DisplayPort 2 Output [vol: 0.80]\n\
             │  *   52. Built-in Audio Analog Stereo [vol: 0.72]\n\
             ├─ Sources:\n\
             │  *   53. Built-in Audio Analog Stereo [vol: 1.00]\n\
             ├─ Filters:",
        );
        sort_devices(&mut outputs);
        sort_devices(&mut inputs);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].id, "52");
        assert!(outputs[0].is_default);
        assert_eq!(inputs[0].name, "Built-in Audio Analog Stereo");
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
