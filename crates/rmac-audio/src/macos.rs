//! macOS development audio backend and fixture parsers.

use super::*;

#[cfg(target_os = "macos")]
pub(super) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
    sender
        .send(WatchEvent::Unavailable)
        .await
        .map_err(|_| Error::new("watch audio changes", "the event consumer closed"))
}

#[cfg(target_os = "macos")]
pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
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
    let has_output = outputs.iter().any(|device| device.is_default);
    let has_input = inputs.iter().any(|device| device.is_default);
    Ok(Snapshot {
        available: true,
        has_output,
        has_input,
        can_set_default: false,
        can_mute_input: false,
        configuration_available: false,
        configuration_error: None,
        output,
        input,
        outputs,
        inputs,
        hardware_devices: Vec::new(),
    })
}

#[cfg(target_os = "macos")]
pub(super) fn system_set_volume(kind: DeviceKind, volume: u8) -> Result<(), Error> {
    let script = match kind {
        DeviceKind::Output => format!("set volume output volume {volume}"),
        DeviceKind::Input => format!("set volume input volume {volume}"),
    };
    command("osascript", &["-e", &script], "change audio volume")?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn system_set_muted(kind: DeviceKind, muted: bool) -> Result<(), Error> {
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
pub(super) fn system_set_default_device(_: DeviceKind, _: &Device) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change default audio device",
        "macOS does not expose device selection through the scripting adapter",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn system_set_profile(_: &HardwareDevice, _: &Profile) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change audio profile",
        "macOS does not expose profiles through the scripting adapter",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn system_set_route(_: DeviceKind, _: &Device, _: &Route) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change audio route",
        "macOS does not expose routes through the scripting adapter",
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn system_set_balance(_: &Device, _: i8) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change output balance",
        "macOS does not expose balance through the scripting adapter",
    ))
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn parse_macos_audio_devices(output: &str) -> (Vec<Device>, Vec<Device>) {
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
                    routes: Vec::new(),
                    balance: None,
                    authority_name: name.clone(),
                    authority_device_id: None,
                    authority_route_device: None,
                });
            }
            if has_input {
                inputs.push(Device {
                    id: name.clone(),
                    name: name.clone(),
                    is_default: default_input,
                    routes: Vec::new(),
                    balance: None,
                    authority_name: name,
                    authority_device_id: None,
                    authority_route_device: None,
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
pub(super) fn parse_macos_volume_settings(output: &str) -> Option<(Level, Level)> {
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
