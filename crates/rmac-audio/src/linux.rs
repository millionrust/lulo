//! Linux PipeWire service backend and parsers.

use super::*;

#[cfg(not(target_os = "macos"))]
pub(super) const WATCH_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(not(target_os = "macos"))]
pub(super) const WATCH_QUIET_PERIOD: std::time::Duration = std::time::Duration::from_millis(75);
#[cfg(not(target_os = "macos"))]
pub(super) const WATCH_MAX_COALESCE: std::time::Duration = std::time::Duration::from_millis(250);
#[cfg(not(target_os = "macos"))]
pub(super) const MUTATION_VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(not(target_os = "macos"))]
pub(super) const MUTATION_VERIFY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);

#[cfg(not(target_os = "macos"))]
pub(super) async fn system_watch(sender: async_channel::Sender<WatchEvent>) -> Result<(), Error> {
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

#[cfg(not(target_os = "macos"))]
pub(super) async fn watch_once(
    sender: &async_channel::Sender<WatchEvent>,
    unavailable_reported: &mut bool,
) -> Result<(), Error> {
    use std::process::Stdio;

    use futures_lite::io::AsyncReadExt as _;

    // `pw-dump --monitor` is the machine-readable PipeWire graph monitor: it
    // prints the full graph once, then a JSON array of the objects that
    // changed on every later state change. Only changes to the objects a
    // snapshot reads trigger a re-read; `pw-dump` clients (every snapshot
    // is one) coming and going do not, or watchers would feed each other.
    let mut monitor = std::process::Command::new("pw-dump");
    monitor
        .arg("--monitor")
        .arg("--no-colors")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // `kill_on_drop` covers a watcher that stops; binding covers a process
    // that exits without dropping it, so the monitor never outlives it.
    rmac_process::bind_to_parent(&mut monitor);
    let mut command = async_process::Command::from(monitor);
    command.kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| Error::new("start the PipeWire monitor", error.to_string()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::new("start the PipeWire monitor", "stdout was not captured"))?;
    let mut buffer = [0_u8; 8192];
    let mut changes = crate::MonitorChanges::default();
    let mut absorb = |bytes: &[u8]| -> Result<bool, Error> {
        changes
            .feed(bytes)
            .map_err(|error| Error::new("read PipeWire changes", error))
    };

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
        if !absorb(&buffer[..read])? {
            continue;
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
                    absorb(&buffer[..read])?;
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
pub(super) async fn monitor_status_error(mut child: async_process::Child) -> Result<(), Error> {
    let status = child
        .status()
        .await
        .map_err(|error| Error::new("wait for the PipeWire monitor", error.to_string()))?;
    Err(Error::new(
        "watch PipeWire changes",
        format!("pw-dump --monitor exited with {status}"),
    ))
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn publish_changed(
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
pub(super) async fn publish_unavailable(
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

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum RouteDirection {
    Output,
    Input,
}

#[cfg(any(not(target_os = "macos"), test))]
type ActiveRoutes = std::collections::HashMap<(RouteDirection, i32), i32>;

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Debug)]
pub(super) struct GraphNode {
    pub(super) authority_name: String,
    pub(super) description: String,
    pub(super) kind: DeviceKind,
    pub(super) device_id: Option<String>,
    pub(super) route_device: Option<i32>,
    pub(super) balance: Option<Balance>,
    pub(super) level: Option<Level>,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Debug)]
pub(super) struct GraphRoute {
    pub(super) route: Route,
    pub(super) direction: RouteDirection,
    pub(super) device_indexes: Vec<i32>,
    pub(super) profile_indexes: Vec<i32>,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Debug)]
pub(super) struct GraphHardwareDevice {
    pub(super) device: HardwareDevice,
    pub(super) active_profile: i32,
    pub(super) routes: Vec<GraphRoute>,
    pub(super) active_routes: ActiveRoutes,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Debug, Default)]
pub(super) struct GraphMetadata {
    pub(super) nodes: std::collections::HashMap<String, GraphNode>,
    pub(super) hardware: Vec<GraphHardwareDevice>,
    pub(super) capabilities_rejected: bool,
    /// `node.name` of the node named by the `default` metadata object's
    /// `default.audio.sink` key, if any and unambiguous.
    pub(super) default_sink: Option<String>,
    /// `node.name` of the node named by the `default` metadata object's
    /// `default.audio.source` key, if any and unambiguous.
    pub(super) default_source: Option<String>,
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_snapshot() -> Result<Snapshot, Error> {
    let graph = read_graph_metadata("read PipeWire state")?;
    let mut outputs = graph_devices(&graph, DeviceKind::Output);
    let mut inputs = graph_devices(&graph, DeviceKind::Input);
    apply_graph_metadata(&mut outputs, &graph, DeviceKind::Output);
    apply_graph_metadata(&mut inputs, &graph, DeviceKind::Input);
    sort_devices(&mut outputs);
    sort_devices(&mut inputs);
    let has_output = default_device_id(&outputs).is_some();
    let has_input = default_device_id(&inputs).is_some();
    let output = default_level(&graph, DeviceKind::Output, "read output volume")?;
    let input = default_level(&graph, DeviceKind::Input, "read input volume")?;
    let configuration_available = !graph.capabilities_rejected;
    let configuration_error = graph.capabilities_rejected.then(|| {
        "Some audio port, device profile, or default-device data was rejected because PipeWire \
         returned an ambiguous response."
            .to_string()
    });
    let mut hardware_devices = graph
        .hardware
        .into_iter()
        .map(|hardware| hardware.device)
        .collect::<Vec<_>>();
    hardware_devices.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(Snapshot {
        available: true,
        has_output,
        has_input,
        can_set_default: true,
        can_mute_input: has_input,
        configuration_available,
        configuration_error,
        output,
        input,
        outputs,
        inputs,
        hardware_devices,
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_default_device(kind: DeviceKind) -> Result<DefaultDevice, Error> {
    let operation = match kind {
        DeviceKind::Output => "read default output device",
        DeviceKind::Input => "read default input device",
    };
    let graph = read_graph_metadata(operation)?;
    let node = default_node(&graph, kind)
        .ok_or_else(|| Error::new(operation, "PipeWire has no default device configured"))?;
    let level = node.level.ok_or_else(|| {
        Error::new(
            operation,
            "PipeWire did not report a usable volume for the default device",
        )
    })?;
    Ok(DefaultDevice {
        name: node.description.clone(),
        level,
    })
}

/// The graph node currently named by the PipeWire `default` metadata object
/// for `kind`, if the default is set and points at a node that is present.
#[cfg(not(target_os = "macos"))]
fn default_node(graph: &GraphMetadata, kind: DeviceKind) -> Option<&GraphNode> {
    let default_name = match kind {
        DeviceKind::Output => graph.default_sink.as_deref(),
        DeviceKind::Input => graph.default_source.as_deref(),
    }?;
    graph
        .nodes
        .values()
        .find(|node| node.kind == kind && node.authority_name == default_name)
}

/// The volume/mute level of the current default device for `kind`, read
/// straight from its `pw-dump` node Props. No default device is not an
/// error (`Level::default()`); a default device with unreadable Props is.
#[cfg(not(target_os = "macos"))]
fn default_level(
    graph: &GraphMetadata,
    kind: DeviceKind,
    operation: &'static str,
) -> Result<Level, Error> {
    match default_node(graph, kind) {
        None => Ok(Level::default()),
        Some(node) => node.level.ok_or_else(|| {
            Error::new(
                operation,
                "PipeWire did not report a usable volume for the default device",
            )
        }),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn machine_devices(kind: DeviceKind) -> Result<Vec<Device>, Error> {
    let operation = match kind {
        DeviceKind::Output => "read PipeWire output devices",
        DeviceKind::Input => "read PipeWire input devices",
    };
    let graph = read_graph_metadata(operation)?;
    let mut devices = graph_devices(&graph, kind);
    apply_graph_metadata(&mut devices, &graph, kind);
    sort_devices(&mut devices);
    Ok(devices)
}

/// The recovery hint appended to a failed mutation `Error`'s detail: mutations
/// only check the mutating command's exit status (never its text), so this
/// is the one place callers get an actionable next step when that exit
/// status is non-zero.
#[cfg(not(target_os = "macos"))]
const PIPEWIRE_MUTATION_HINT: &str =
    "check that PipeWire and WirePlumber are running for this session (`wpctl status`)";

/// Runs a PipeWire/WirePlumber CLI mutation, checking only its exit status.
/// On failure, the command's stderr detail is kept but annotated with
/// [`PIPEWIRE_MUTATION_HINT`] so the caller has a next step, not just a raw
/// tool error.
#[cfg(not(target_os = "macos"))]
fn pipewire_mutation(
    program: &'static str,
    arguments: &[&str],
    operation: &'static str,
) -> Result<(), Error> {
    command(program, arguments, operation)
        .map(|_| ())
        .map_err(|error| {
            Error::new(
                operation,
                format!("{}; {PIPEWIRE_MUTATION_HINT}", error.detail()),
            )
        })
}

#[cfg(not(target_os = "macos"))]
fn wpctl_mutation(arguments: &[&str], operation: &'static str) -> Result<(), Error> {
    pipewire_mutation("wpctl", arguments, operation)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_set_volume(kind: DeviceKind, volume: u8) -> Result<(), Error> {
    let target = wpctl_default_target(kind);
    let value = format!("{:.2}", f32::from(volume) / 100.0);
    wpctl_mutation(
        &["set-volume", target, &value],
        match kind {
            DeviceKind::Output => "change output volume",
            DeviceKind::Input => "change input volume",
        },
    )
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_set_muted(kind: DeviceKind, muted: bool) -> Result<(), Error> {
    wpctl_mutation(
        &[
            "set-mute",
            wpctl_default_target(kind),
            if muted { "1" } else { "0" },
        ],
        match kind {
            DeviceKind::Output => "change output mute",
            DeviceKind::Input => "change input mute",
        },
    )
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_set_default_device(
    kind: DeviceKind,
    expected: &Device,
) -> Result<Snapshot, Error> {
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
    wpctl_mutation(
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
pub(super) fn system_set_profile(
    expected_device: &HardwareDevice,
    expected_profile: &Profile,
) -> Result<Snapshot, Error> {
    if expected_device
        .id
        .parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
        .is_none()
        || expected_profile.index < 0
    {
        return Err(Error::new(
            "change audio profile",
            "invalid PipeWire device or profile identity",
        ));
    }
    let graph = read_graph_metadata("read audio profiles")?;
    let current_device = exact_hardware_device(&graph, expected_device, "change audio profile")?;
    let current_profile = current_device
        .device
        .profiles
        .iter()
        .find(|profile| {
            profile.index == expected_profile.index
                && profile.authority_name == expected_profile.authority_name
        })
        .ok_or_else(|| {
            Error::new(
                "change audio profile",
                "the selected profile is no longer advertised",
            )
        })?;
    if !current_profile.availability.can_select() {
        return Err(Error::new(
            "change audio profile",
            "the selected profile is currently unavailable",
        ));
    }
    if current_profile.is_active {
        return system_snapshot();
    }
    let index = expected_profile.index.to_string();
    wpctl_mutation(
        &["set-profile", &expected_device.id, &index],
        "change audio profile",
    )?;

    let deadline = std::time::Instant::now() + MUTATION_VERIFY_TIMEOUT;
    loop {
        let graph = match read_graph_metadata("verify the audio profile") {
            Ok(graph) => graph,
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {
                std::thread::sleep(MUTATION_VERIFY_INTERVAL);
                continue;
            }
        };
        match graph
            .hardware
            .iter()
            .find(|device| device.device.id == expected_device.id)
        {
            Some(device) if device.device.authority_name != expected_device.authority_name => {
                return Err(Error::new(
                    "verify the audio profile",
                    "the PipeWire device identity changed after the request",
                ));
            }
            Some(device)
                if device.device.profiles.iter().any(|profile| {
                    profile.index == expected_profile.index
                        && profile.authority_name == expected_profile.authority_name
                        && profile.is_active
                }) =>
            {
                let snapshot = system_snapshot()?;
                if snapshot.hardware_devices.iter().any(|device| {
                    device.id == expected_device.id
                        && device.authority_name == expected_device.authority_name
                        && device.profiles.iter().any(|profile| {
                            profile.index == expected_profile.index
                                && profile.authority_name == expected_profile.authority_name
                                && profile.is_active
                        })
                }) {
                    return Ok(snapshot);
                }
            }
            _ => {}
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                "verify the audio profile",
                "PipeWire did not confirm the requested profile within three seconds",
            ));
        }
        std::thread::sleep(MUTATION_VERIFY_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_set_route(
    kind: DeviceKind,
    expected_device: &Device,
    expected_route: &Route,
) -> Result<Snapshot, Error> {
    if expected_device
        .id
        .parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
        .is_none()
        || expected_route.index < 0
    {
        return Err(Error::new(
            "change audio route",
            "invalid PipeWire node or route identity",
        ));
    }
    let current = exact_routed_device(kind, expected_device, "change audio route")?;
    let route = exact_route(&current, expected_route, "change audio route")?;
    if !route.availability.can_select() {
        return Err(Error::new(
            "change audio route",
            "the selected route is currently unavailable",
        ));
    }
    if route.is_active {
        return system_snapshot();
    }
    let index = expected_route.index.to_string();
    wpctl_mutation(
        &["set-route", &expected_device.id, &index],
        "change audio route",
    )?;

    let deadline = std::time::Instant::now() + MUTATION_VERIFY_TIMEOUT;
    loop {
        match exact_routed_device(kind, expected_device, "verify the audio route") {
            Ok(device) => match exact_route(&device, expected_route, "verify the audio route") {
                Ok(route) if route.is_active => {
                    let snapshot = system_snapshot()?;
                    let devices = match kind {
                        DeviceKind::Output => &snapshot.outputs,
                        DeviceKind::Input => &snapshot.inputs,
                    };
                    if devices.iter().any(|device| {
                        device.id == expected_device.id
                            && device.authority_name == expected_device.authority_name
                            && device.authority_device_id == expected_device.authority_device_id
                            && device.authority_route_device
                                == expected_device.authority_route_device
                            && device.routes.iter().any(|route| {
                                route.index == expected_route.index
                                    && route.authority_name == expected_route.authority_name
                                    && route.is_active
                            })
                    }) {
                        return Ok(snapshot);
                    }
                }
                Ok(_) | Err(_) => {}
            },
            Err(error) => {
                if std::time::Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                "verify the audio route",
                "PipeWire did not confirm the requested route within three seconds",
            ));
        }
        std::thread::sleep(MUTATION_VERIFY_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn system_set_balance(expected_device: &Device, value: i8) -> Result<Snapshot, Error> {
    if expected_device.balance.is_none() {
        return Err(Error::new(
            "change output balance",
            "the selected node did not advertise writable stereo channels",
        ));
    }
    let current =
        exact_routed_device(DeviceKind::Output, expected_device, "change output balance")?;
    let current_balance = current.balance.as_ref().ok_or_else(|| {
        Error::new(
            "change output balance",
            "the selected node no longer advertises writable stereo channels",
        )
    })?;
    if current_balance.value == value {
        return system_snapshot();
    }
    let (left, right) = balance_channel_targets(current_balance, value);
    let (first, second) = if current_balance.left_first {
        (left, right)
    } else {
        (right, left)
    };
    let parameter = format!(
        "{{ channelVolumes: [ {}, {} ] }}",
        spa_channel_volume(first),
        spa_channel_volume(second)
    );
    pipewire_mutation(
        "pw-cli",
        &["set-param", &expected_device.id, "Props", &parameter],
        "change output balance",
    )?;

    let deadline = std::time::Instant::now() + MUTATION_VERIFY_TIMEOUT;
    loop {
        match exact_routed_device(DeviceKind::Output, expected_device, "verify output balance") {
            Ok(device)
                if device
                    .balance
                    .as_ref()
                    .is_some_and(|balance| balance.value.abs_diff(value) <= 1) =>
            {
                let snapshot = system_snapshot()?;
                if snapshot.outputs.iter().any(|device| {
                    device.id == expected_device.id
                        && device.authority_name == expected_device.authority_name
                        && device
                            .balance
                            .as_ref()
                            .is_some_and(|balance| balance.value.abs_diff(value) <= 1)
                }) {
                    return Ok(snapshot);
                }
            }
            Ok(_) => {}
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::new(
                "verify output balance",
                "PipeWire did not confirm the requested balance within three seconds",
            ));
        }
        std::thread::sleep(MUTATION_VERIFY_INTERVAL);
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn balance_channel_targets(current: &Balance, value: i8) -> (u32, u32) {
    let maximum = current.left_volume.max(current.right_volume);
    if value >= 0 {
        (
            attenuated_channel_volume(maximum, 100 - u32::from(value as u8)),
            maximum,
        )
    } else {
        (
            maximum,
            attenuated_channel_volume(maximum, 100 - u32::from(value.unsigned_abs())),
        )
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn attenuated_channel_volume(maximum: u32, percent: u32) -> u32 {
    ((u64::from(maximum) * u64::from(percent) + 50) / 100) as u32
}

#[cfg(not(target_os = "macos"))]
pub(super) fn spa_channel_volume(value: u32) -> String {
    format!("{}.{:06}", value / 1_000_000, value % 1_000_000)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn read_graph_metadata(operation: &'static str) -> Result<GraphMetadata, Error> {
    let dump = command("pw-dump", &["--no-colors"], operation)?;
    parse_pw_dump_metadata(&dump).map_err(|error| Error::new(operation, error.detail()))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn exact_hardware_device<'a>(
    graph: &'a GraphMetadata,
    expected: &HardwareDevice,
    operation: &'static str,
) -> Result<&'a GraphHardwareDevice, Error> {
    let current = graph
        .hardware
        .iter()
        .find(|device| device.device.id == expected.id)
        .ok_or_else(|| Error::new(operation, "the selected PipeWire device disappeared"))?;
    if current.device.authority_name != expected.authority_name {
        return Err(Error::new(
            operation,
            "the selected PipeWire device identity changed; refresh Sound before trying again",
        ));
    }
    Ok(current)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn exact_routed_device(
    kind: DeviceKind,
    expected: &Device,
    operation: &'static str,
) -> Result<Device, Error> {
    // `machine_devices` already reads the graph and every node from a single
    // `pw-dump` call and applies that same graph's metadata, so there is no
    // second, separately-timed read to reconcile here.
    let devices = machine_devices(kind)?;
    let current = devices
        .into_iter()
        .find(|device| device.id == expected.id)
        .ok_or_else(|| Error::new(operation, "the selected audio node disappeared"))?;
    if current.authority_name != expected.authority_name
        || current.authority_device_id != expected.authority_device_id
        || current.authority_route_device != expected.authority_route_device
    {
        return Err(Error::new(
            operation,
            "the selected audio route identity changed; refresh Sound before trying again",
        ));
    }
    Ok(current)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn exact_route<'a>(
    device: &'a Device,
    expected: &Route,
    operation: &'static str,
) -> Result<&'a Route, Error> {
    device
        .routes
        .iter()
        .find(|route| {
            route.index == expected.index && route.authority_name == expected.authority_name
        })
        .ok_or_else(|| Error::new(operation, "the selected route is no longer advertised"))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn wpctl_default_target(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Output => "@DEFAULT_AUDIO_SINK@",
        DeviceKind::Input => "@DEFAULT_AUDIO_SOURCE@",
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) const MAX_AUDIO_DEVICES: usize = 256;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) const MAX_AUTHORITY_NAME_BYTES: usize = 512;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) const MAX_DEVICE_LABEL_CHARS: usize = 256;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) const MAX_GRAPH_OBJECTS: usize = 4096;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) const MAX_DEVICE_CAPABILITIES: usize = 128;

/// Builds the machine-readable device list for `kind` straight from the
/// `pw-dump` graph: one [`Device`] per matching node, marked `is_default`
/// against the PipeWire `default` metadata object (see
/// [`GraphMetadata::default_sink`] / [`GraphMetadata::default_source`]).
/// Ports, routes and balance are filled in separately by
/// [`apply_graph_metadata`].
#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn graph_devices(graph: &GraphMetadata, kind: DeviceKind) -> Vec<Device> {
    let default_name = match kind {
        DeviceKind::Output => graph.default_sink.as_deref(),
        DeviceKind::Input => graph.default_source.as_deref(),
    };
    let mut devices = graph
        .nodes
        .iter()
        .filter(|(_, node)| node.kind == kind)
        .map(|(id, node)| Device {
            id: id.clone(),
            name: node.description.clone(),
            is_default: default_name == Some(node.authority_name.as_str()),
            routes: Vec::new(),
            balance: None,
            authority_name: node.authority_name.clone(),
            authority_device_id: None,
            authority_route_device: None,
        })
        .collect::<Vec<_>>();
    sort_devices(&mut devices);
    devices
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_pw_dump_metadata(output: &str) -> Result<GraphMetadata, Error> {
    use serde_json::Value;

    let Value::Array(objects) = serde_json::from_str::<Value>(output).map_err(|_| {
        Error::new(
            "read PipeWire capabilities",
            "pw-dump returned invalid JSON",
        )
    })?
    else {
        return Err(Error::new(
            "read PipeWire capabilities",
            "pw-dump did not return a JSON array",
        ));
    };
    if objects.len() > MAX_GRAPH_OBJECTS {
        return Err(Error::new(
            "read PipeWire capabilities",
            "the PipeWire graph exceeded 4096 objects",
        ));
    }

    let mut graph = GraphMetadata::default();
    let mut ambiguous_nodes = std::collections::HashSet::new();
    let mut sink_count = 0_usize;
    let mut source_count = 0_usize;
    let mut default_objects = Vec::new();
    for object in &objects {
        let Some(id) = json_u32(object.get("id")) else {
            continue;
        };
        match object.get("type").and_then(Value::as_str) {
            Some("PipeWire:Interface:Node") => {
                let Some(props) = object.get("info").and_then(|info| info.get("props")) else {
                    continue;
                };
                let kind = match props.get("media.class").and_then(Value::as_str) {
                    Some("Audio/Sink") => DeviceKind::Output,
                    Some("Audio/Source") => DeviceKind::Input,
                    _ => continue,
                };
                match kind {
                    DeviceKind::Output => sink_count += 1,
                    DeviceKind::Input => source_count += 1,
                }
                if sink_count > MAX_AUDIO_DEVICES || source_count > MAX_AUDIO_DEVICES {
                    graph.capabilities_rejected = true;
                    continue;
                }
                let Some(authority_name) = props
                    .get("node.name")
                    .and_then(Value::as_str)
                    .and_then(bounded_authority_name)
                else {
                    continue;
                };
                let description = ["node.description", "node.nick", "node.name"]
                    .into_iter()
                    .find_map(|key| props.get(key).and_then(Value::as_str))
                    .map(bounded_label)
                    .filter(|label| !label.is_empty())
                    .unwrap_or_else(|| bounded_label(&authority_name));
                let node = GraphNode {
                    authority_name,
                    description,
                    kind,
                    device_id: json_u32(props.get("device.id")).map(|id| id.to_string()),
                    route_device: json_i32(props.get("card.profile.device")),
                    balance: parse_node_balance(object),
                    level: parse_node_level(object),
                };
                let id = id.to_string();
                if graph.nodes.insert(id.clone(), node).is_some() {
                    ambiguous_nodes.insert(id);
                }
            }
            Some("PipeWire:Interface:Device") => {
                let Some(props) = object.get("info").and_then(|info| info.get("props")) else {
                    continue;
                };
                if props.get("media.class").and_then(Value::as_str) != Some("Audio/Device") {
                    continue;
                }
                if graph.hardware.len() == MAX_AUDIO_DEVICES {
                    graph.capabilities_rejected = true;
                    continue;
                }
                let has_profiles = object
                    .get("info")
                    .and_then(|info| info.get("params"))
                    .and_then(|params| params.get("EnumProfile"))
                    .is_some();
                if has_profiles {
                    if let Some(device) = parse_graph_hardware_device(id, props, object) {
                        graph.hardware.push(device);
                    } else {
                        graph.capabilities_rejected = true;
                    }
                }
            }
            Some("PipeWire:Interface:Metadata") => {
                let Some(props) = object.get("props") else {
                    continue;
                };
                if props.get("metadata.name").and_then(Value::as_str) != Some("default") {
                    continue;
                }
                default_objects.push(object);
            }
            _ => {}
        }
    }
    for id in ambiguous_nodes {
        graph.nodes.remove(&id);
    }
    match default_objects.as_slice() {
        [] => {}
        [only] => {
            if let Some(entries) = only.get("metadata").and_then(Value::as_array) {
                graph.default_sink = default_node_name(entries, "default.audio.sink");
                graph.default_source = default_node_name(entries, "default.audio.source");
            }
        }
        // PipeWire only ever publishes one "default" metadata object; more
        // than one is an ambiguous response we cannot safely pick between.
        _ => graph.capabilities_rejected = true,
    }
    let mut id_counts = std::collections::HashMap::new();
    let mut name_counts = std::collections::HashMap::new();
    for hardware in &graph.hardware {
        *id_counts
            .entry(hardware.device.id.clone())
            .or_insert(0_usize) += 1;
        *name_counts
            .entry(hardware.device.authority_name.clone())
            .or_insert(0_usize) += 1;
    }
    let before = graph.hardware.len();
    graph.hardware.retain(|hardware| {
        id_counts.get(&hardware.device.id) == Some(&1)
            && name_counts.get(&hardware.device.authority_name) == Some(&1)
    });
    graph.capabilities_rejected |= graph.hardware.len() != before;
    Ok(graph)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn bounded_label(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DEVICE_LABEL_CHARS)
        .collect()
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn bounded_authority_name(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= MAX_AUTHORITY_NAME_BYTES
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn json_u32(value: Option<&serde_json::Value>) -> Option<u32> {
    let value = value?;
    let parsed = value
        .as_u64()
        .or_else(|| value.as_str()?.parse::<u64>().ok())?;
    (parsed > 0 && parsed <= u64::from(u32::MAX)).then_some(parsed as u32)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn json_i32(value: Option<&serde_json::Value>) -> Option<i32> {
    let value = value?;
    let parsed = value
        .as_i64()
        .or_else(|| value.as_str()?.parse::<i64>().ok())?;
    (0..=i64::from(i32::MAX))
        .contains(&parsed)
        .then_some(parsed as i32)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_availability(value: Option<&serde_json::Value>) -> Option<Availability> {
    match value.and_then(serde_json::Value::as_str) {
        Some("yes") => Some(Availability::Available),
        Some("no") => Some(Availability::Unavailable),
        Some("unknown") | None => Some(Availability::Unknown),
        _ => None,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_i32_array(value: Option<&serde_json::Value>) -> Option<Vec<i32>> {
    let values = value?.as_array()?;
    if values.len() > MAX_DEVICE_CAPABILITIES {
        return None;
    }
    values.iter().map(|value| json_i32(Some(value))).collect()
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_graph_hardware_device(
    id: u32,
    props: &serde_json::Value,
    object: &serde_json::Value,
) -> Option<GraphHardwareDevice> {
    let authority_name = props
        .get("device.name")
        .and_then(serde_json::Value::as_str)
        .and_then(bounded_authority_name)?;
    let name = ["device.description", "device.nick", "device.name"]
        .into_iter()
        .find_map(|key| props.get(key).and_then(serde_json::Value::as_str))
        .map(bounded_label)
        .filter(|label| !label.is_empty())?;
    let params = object.get("info")?.get("params")?;
    let (mut profiles, active_profile) = parse_profiles(params)?;
    let (routes, active_routes) = parse_routes(params, active_profile)?;
    profiles.sort_by(|left, right| {
        right
            .is_active
            .cmp(&left.is_active)
            .then_with(|| {
                right
                    .availability
                    .can_select()
                    .cmp(&left.availability.can_select())
            })
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.index.cmp(&right.index))
    });
    Some(GraphHardwareDevice {
        device: HardwareDevice {
            id: id.to_string(),
            name,
            profiles,
            authority_name,
        },
        active_profile,
        routes,
        active_routes,
    })
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_profiles(params: &serde_json::Value) -> Option<(Vec<Profile>, i32)> {
    let enumerated = params.get("EnumProfile")?.as_array()?;
    let active = params.get("Profile")?.as_array()?;
    if enumerated.is_empty() || enumerated.len() > MAX_DEVICE_CAPABILITIES || active.len() != 1 {
        return None;
    }
    let active_index = json_i32(active[0].get("index"))?;
    let active_name = active[0]
        .get("name")
        .and_then(serde_json::Value::as_str)
        .and_then(bounded_authority_name)?;
    let mut seen_indexes = std::collections::HashSet::new();
    let mut seen_names = std::collections::HashSet::new();
    let mut profiles = Vec::with_capacity(enumerated.len());
    for value in enumerated {
        let index = json_i32(value.get("index"))?;
        let authority_name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .and_then(bounded_authority_name)?;
        if !seen_indexes.insert(index) || !seen_names.insert(authority_name.clone()) {
            return None;
        }
        let name = value
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(bounded_label)
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| bounded_label(&authority_name));
        profiles.push(Profile {
            index,
            name,
            availability: parse_availability(value.get("available"))?,
            is_active: index == active_index && authority_name == active_name,
            authority_name,
        });
    }
    profiles
        .iter()
        .any(|profile| profile.is_active)
        .then_some((profiles, active_index))
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_routes(
    params: &serde_json::Value,
    active_profile: i32,
) -> Option<(Vec<GraphRoute>, ActiveRoutes)> {
    let Some(enumerated) = params.get("EnumRoute") else {
        return Some((Vec::new(), std::collections::HashMap::new()));
    };
    let enumerated = enumerated.as_array()?;
    if enumerated.len() > MAX_DEVICE_CAPABILITIES {
        return None;
    }
    let mut routes = Vec::with_capacity(enumerated.len());
    let mut identities = std::collections::HashSet::new();
    for value in enumerated {
        let index = json_i32(value.get("index"))?;
        let direction = parse_route_direction(value.get("direction"))?;
        let authority_name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .and_then(bounded_authority_name)?;
        if !identities.insert((direction, index)) {
            return None;
        }
        let name = value
            .get("description")
            .and_then(serde_json::Value::as_str)
            .map(bounded_label)
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| bounded_label(&authority_name));
        routes.push(GraphRoute {
            route: Route {
                index,
                name,
                availability: parse_availability(value.get("available"))?,
                is_active: false,
                authority_name,
            },
            direction,
            device_indexes: parse_i32_array(value.get("devices"))?,
            profile_indexes: parse_i32_array(value.get("profiles"))?,
        });
    }

    let mut active_routes = std::collections::HashMap::new();
    let active = match params.get("Route") {
        Some(value) => value.as_array()?.clone(),
        None => Vec::new(),
    };
    if active.len() > MAX_DEVICE_CAPABILITIES {
        return None;
    }
    for value in active {
        let index = json_i32(value.get("index"))?;
        let direction = parse_route_direction(value.get("direction"))?;
        let device = json_i32(value.get("device"))?;
        if json_i32(value.get("profile"))? != active_profile {
            return None;
        }
        if active_routes.insert((direction, device), index).is_some()
            || !routes.iter().any(|route| {
                route.direction == direction
                    && route.route.index == index
                    && route.device_indexes.contains(&device)
            })
        {
            return None;
        }
    }
    Some((routes, active_routes))
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_route_direction(value: Option<&serde_json::Value>) -> Option<RouteDirection> {
    match value.and_then(serde_json::Value::as_str) {
        Some("Output") => Some(RouteDirection::Output),
        Some("Input") => Some(RouteDirection::Input),
        _ => None,
    }
}

/// Finds the single Props entry in a node's `params.Props` array that
/// carries `channelVolumes`. Real `pw-dump` output for an ALSA-backed node
/// has *two* Props entries — the volume/mute one and a second, ALSA-route
/// specific one (`device`, `deviceName`, …) with no `channelVolumes` field —
/// so this cannot assume the array has exactly one entry; it must instead
/// find the one entry that advertises `channelVolumes`, and reject the node
/// as unreadable if that is not exactly one entry.
#[cfg(any(not(target_os = "macos"), test))]
fn volume_props(params: &serde_json::Value) -> Option<&serde_json::Value> {
    let mut matches = params.get("Props")?.as_array()?.iter().filter(|entry| {
        entry
            .get("channelVolumes")
            .and_then(serde_json::Value::as_array)
            .is_some()
    });
    let found = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(found)
}

/// A node's live volume/mute, read from its Props `channelVolumes` (a
/// linear scale) and `mute`. `channelVolumes` is averaged across channels
/// and cube-rooted to match the perceptual/cubic scale WirePlumber tools
/// such as `wpctl` display (e.g. a raw `channelVolumes` of `8e-6` is the
/// `wpctl`-displayed `0.02`, since `0.02.powi(3) == 8e-6`).
#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_node_level(object: &serde_json::Value) -> Option<Level> {
    let params = object.get("info")?.get("params")?;
    let props = volume_props(params)?;
    let channel_volumes = props.get("channelVolumes")?.as_array()?;
    if channel_volumes.is_empty() || channel_volumes.len() > MAX_DEVICE_CAPABILITIES {
        return None;
    }
    let mut sum = 0.0_f64;
    for value in channel_volumes {
        let value = value.as_f64()?;
        if !value.is_finite() || !(0.0..=10.0).contains(&value) {
            return None;
        }
        sum += value;
    }
    let average_linear = sum / channel_volumes.len() as f64;
    let display = average_linear.max(0.0).cbrt();
    let volume = (display * 100.0).round().clamp(0.0, 100.0) as u8;
    let muted = props.get("mute")?.as_bool()?;
    Some(Level { volume, muted })
}

/// Reads a `default.audio.sink`/`default.audio.source`-style entry from a
/// PipeWire `default` metadata object's `metadata` array and returns the
/// node name it points at, if any and well-formed. The value is normally
/// already a decoded `{"name": "..."}` object, but metadata values are
/// generically typed as JSON-encoded strings, so a raw string is also
/// accepted and decoded the same way.
#[cfg(any(not(target_os = "macos"), test))]
fn default_node_name(entries: &[serde_json::Value], key: &str) -> Option<String> {
    let entry = entries
        .iter()
        .find(|entry| entry.get("key").and_then(serde_json::Value::as_str) == Some(key))?;
    let value = entry.get("value")?;
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            let decoded = serde_json::from_str::<serde_json::Value>(value.as_str()?).ok()?;
            decoded.get("name")?.as_str().map(str::to_owned)
        })?;
    bounded_authority_name(&name)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn parse_node_balance(object: &serde_json::Value) -> Option<Balance> {
    let permissions = object.get("permissions")?.as_array()?;
    let writable = permissions
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|permission| permission == "w");
    let executable = permissions
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|permission| permission == "x");
    if !writable || !executable {
        return None;
    }
    let params = object.get("info")?.get("params")?;
    let prop_info = params.get("PropInfo")?.as_array()?;
    let channel_volumes_advertised = prop_info.iter().any(|property| {
        property.get("id").and_then(serde_json::Value::as_str) == Some("channelVolumes")
            && property
                .get("container")
                .and_then(serde_json::Value::as_str)
                == Some("Array")
    });
    if !channel_volumes_advertised {
        return None;
    }
    let props = volume_props(params)?;
    let channel_map = props.get("channelMap")?.as_array()?;
    let channel_volumes = props.get("channelVolumes")?.as_array()?;
    if channel_map.len() != 2 || channel_volumes.len() != 2 {
        return None;
    }
    let first = channel_map[0].as_str()?;
    let second = channel_map[1].as_str()?;
    let left_first = match (first, second) {
        ("FL", "FR") => true,
        ("FR", "FL") => false,
        _ => return None,
    };
    let first_volume = scaled_channel_volume(&channel_volumes[0])?;
    let second_volume = scaled_channel_volume(&channel_volumes[1])?;
    let (left_volume, right_volume) = if left_first {
        (first_volume, second_volume)
    } else {
        (second_volume, first_volume)
    };
    let value = balance_percent(left_volume, right_volume)?;
    Some(Balance {
        value,
        left_volume,
        right_volume,
        left_first,
    })
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn scaled_channel_volume(value: &serde_json::Value) -> Option<u32> {
    const SCALE: f64 = 1_000_000.0;
    let value = value.as_f64()?;
    (value.is_finite() && (0.0..=10.0).contains(&value)).then_some((value * SCALE).round() as u32)
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn balance_percent(left: u32, right: u32) -> Option<i8> {
    const MIN_ADJUSTABLE_VOLUME: u32 = 10_000;
    let maximum = left.max(right);
    if maximum < MIN_ADJUSTABLE_VOLUME {
        return None;
    }
    let difference = i64::from(right) - i64::from(left);
    Some(
        ((difference * 100) as f64 / f64::from(maximum))
            .round()
            .clamp(-100.0, 100.0) as i8,
    )
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn apply_graph_metadata(
    devices: &mut [Device],
    graph: &GraphMetadata,
    kind: DeviceKind,
) {
    for device in devices {
        let Some(node) = graph
            .nodes
            .get(&device.id)
            .filter(|node| node.kind == kind && node.authority_name == device.authority_name)
        else {
            continue;
        };
        device.name.clone_from(&node.description);
        if kind == DeviceKind::Output {
            device.balance.clone_from(&node.balance);
        }
        let (Some(device_id), Some(route_device)) = (&node.device_id, node.route_device) else {
            continue;
        };
        let Some(hardware) = graph
            .hardware
            .iter()
            .find(|hardware| hardware.device.id == *device_id)
        else {
            continue;
        };
        let direction = match kind {
            DeviceKind::Output => RouteDirection::Output,
            DeviceKind::Input => RouteDirection::Input,
        };
        device.routes = hardware
            .routes
            .iter()
            .filter(|route| {
                route.direction == direction
                    && route.device_indexes.contains(&route_device)
                    && route.profile_indexes.contains(&hardware.active_profile)
            })
            .map(|route| {
                let mut route = route.route.clone();
                route.is_active = hardware
                    .active_routes
                    .get(&(direction, route_device))
                    .is_some_and(|active| *active == route.index);
                route
            })
            .collect();
        device.routes.sort_by(|left, right| {
            right
                .is_active
                .cmp(&left.is_active)
                .then_with(|| {
                    right
                        .availability
                        .can_select()
                        .cmp(&left.availability.can_select())
                })
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.index.cmp(&right.index))
        });
        device.authority_device_id = Some(device_id.clone());
        device.authority_route_device = Some(route_device);
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn default_device_id(devices: &[Device]) -> Option<&str> {
    devices
        .iter()
        .find(|device| device.is_default)
        .map(|device| device.id.as_str())
}
