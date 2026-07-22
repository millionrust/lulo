//! Cross-platform system audio state and controls.

use std::fmt;
use std::process::Command;

pub const MAX_NOTIFICATION_SOUND_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_NOTIFICATION_PLAYBACK_SECONDS: u64 = 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationSoundFormat {
    OggOpus,
    OggVorbis,
    WavPcm,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum NotificationSound<'a> {
    Default,
    Encoded {
        format: NotificationSoundFormat,
        bytes: &'a [u8],
    },
}

impl fmt::Debug for NotificationSound<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => formatter.write_str("Default"),
            Self::Encoded { format, bytes } => formatter
                .debug_struct("Encoded")
                .field("format", format)
                .field("bytes", &bytes.len())
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationPlaybackErrorKind {
    InvalidSound,
    UnsupportedPlatform,
    Prepare,
    Start,
    Wait,
    Rejected,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationPlaybackError {
    kind: NotificationPlaybackErrorKind,
}

impl NotificationPlaybackError {
    fn new(kind: NotificationPlaybackErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(self) -> NotificationPlaybackErrorKind {
        self.kind
    }
}

impl fmt::Display for NotificationPlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "notification sound playback failed ({:?})",
            self.kind
        )
    }
}

impl std::error::Error for NotificationPlaybackError {}

/// Plays one validated notification cue through the Linux PipeWire session.
/// Encoded bytes are bounded and checked again before crossing the process
/// boundary. Linux playback has an unconditional 15-second wall-clock limit.
pub async fn play_notification_sound(
    sound: NotificationSound<'_>,
) -> Result<(), NotificationPlaybackError> {
    let bytes = prepare_notification_sound(sound)?;
    system_play_notification_sound(&bytes).await
}

fn prepare_notification_sound(
    sound: NotificationSound<'_>,
) -> Result<std::borrow::Cow<'_, [u8]>, NotificationPlaybackError> {
    match sound {
        NotificationSound::Default => Ok(std::borrow::Cow::Borrowed(default_notification_wav())),
        NotificationSound::Encoded { format, bytes } => {
            validate_notification_sound(format, bytes)?;
            Ok(std::borrow::Cow::Borrowed(bytes))
        }
    }
}

fn validate_notification_sound(
    format: NotificationSoundFormat,
    bytes: &[u8],
) -> Result<(), NotificationPlaybackError> {
    let valid_size = !bytes.is_empty() && bytes.len() <= MAX_NOTIFICATION_SOUND_BYTES;
    let valid_format = match format {
        NotificationSoundFormat::WavPcm => {
            bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE"
        }
        NotificationSoundFormat::OggOpus => {
            bytes.starts_with(b"OggS") && header_prefix_contains(bytes, b"OpusHead")
        }
        NotificationSoundFormat::OggVorbis => {
            bytes.starts_with(b"OggS") && header_prefix_contains(bytes, b"\x01vorbis")
        }
    };
    if !valid_size || !valid_format {
        return Err(NotificationPlaybackError::new(
            NotificationPlaybackErrorKind::InvalidSound,
        ));
    }
    Ok(())
}

fn header_prefix_contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes
        .get(..bytes.len().min(256))
        .is_some_and(|prefix| prefix.windows(needle.len()).any(|window| window == needle))
}

/// An original short two-tone rmac cue, encoded as mono 48 kHz PCM WAV.
fn default_notification_wav() -> &'static [u8] {
    static CHIME: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    CHIME.get_or_init(generate_default_notification_wav)
}

fn generate_default_notification_wav() -> Vec<u8> {
    const SAMPLE_RATE: u32 = 48_000;
    const DURATION_MS: u32 = 420;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;
    let sample_count = SAMPLE_RATE * DURATION_MS / 1_000;
    let data_bytes = sample_count * u32::from(BITS_PER_SAMPLE / 8);
    let mut wav = Vec::with_capacity((44 + data_bytes) as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&CHANNELS.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * u32::from(CHANNELS) * 2).to_le_bytes());
    wav.extend_from_slice(&(CHANNELS * 2).to_le_bytes());
    wav.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..sample_count {
        let time = index as f32 / SAMPLE_RATE as f32;
        let attack = (time / 0.008).min(1.0);
        let envelope = attack * (-7.5 * time).exp();
        let lower = (std::f32::consts::TAU * 880.0 * time).sin();
        let upper = (std::f32::consts::TAU * 1_320.0 * time + 0.35).sin();
        let sample = ((lower * 0.66 + upper * 0.34) * envelope * 0.22).clamp(-1.0, 1.0);
        wav.extend_from_slice(&((sample * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    wav
}

#[cfg(target_os = "linux")]
async fn system_play_notification_sound(bytes: &[u8]) -> Result<(), NotificationPlaybackError> {
    use std::fs::File;
    use std::io::{Seek as _, Write as _};
    use std::process::Stdio;
    use std::time::Duration;

    use rustix::fs::{fcntl_add_seals, memfd_create, MemfdFlags, SealFlags};

    let fd = memfd_create(
        "rmac-notification-sound",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING | MemfdFlags::NOEXEC_SEAL,
    )
    .map_err(|_| NotificationPlaybackError::new(NotificationPlaybackErrorKind::Prepare))?;
    let mut file = File::from(fd);
    file.write_all(bytes)
        .and_then(|_| file.flush())
        .and_then(|_| file.seek(std::io::SeekFrom::Start(0)).map(drop))
        .map_err(|_| NotificationPlaybackError::new(NotificationPlaybackErrorKind::Prepare))?;
    fcntl_add_seals(
        &file,
        SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL,
    )
    .map_err(|_| NotificationPlaybackError::new(NotificationPlaybackErrorKind::Prepare))?;

    let mut command = async_process::Command::new("pw-play");
    command
        .arg("--media-category=Playback")
        .arg("--media-role=Notification")
        .arg("--latency=50ms")
        .arg("/proc/self/fd/0")
        .stdin(Stdio::from(file))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|_| NotificationPlaybackError::new(NotificationPlaybackErrorKind::Start))?;

    enum Wait {
        Exited(std::io::Result<std::process::ExitStatus>),
        TimedOut,
    }
    let outcome = futures_lite::future::race(async { Wait::Exited(child.status().await) }, async {
        async_io::Timer::after(Duration::from_secs(MAX_NOTIFICATION_PLAYBACK_SECONDS)).await;
        Wait::TimedOut
    })
    .await;
    match outcome {
        Wait::Exited(Ok(status)) if status.success() => Ok(()),
        Wait::Exited(Ok(_)) => Err(NotificationPlaybackError::new(
            NotificationPlaybackErrorKind::Rejected,
        )),
        Wait::Exited(Err(_)) => Err(NotificationPlaybackError::new(
            NotificationPlaybackErrorKind::Wait,
        )),
        Wait::TimedOut => {
            let _ = child.kill();
            let _ = child.status().await;
            Err(NotificationPlaybackError::new(
                NotificationPlaybackErrorKind::Timeout,
            ))
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn system_play_notification_sound(_: &[u8]) -> Result<(), NotificationPlaybackError> {
    Err(NotificationPlaybackError::new(
        NotificationPlaybackErrorKind::UnsupportedPlatform,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Output,
    Input,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable,
    Unknown,
}

impl Availability {
    pub fn can_select(self) -> bool {
        self != Self::Unavailable
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Balance {
    pub value: i8,
    left_volume: u32,
    right_volume: u32,
    left_first: bool,
}

impl fmt::Debug for Balance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Balance")
            .field("value", &self.value)
            .field("has_channel_authority", &true)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Route {
    pub index: i32,
    pub name: String,
    pub availability: Availability,
    pub is_active: bool,
    authority_name: String,
}

impl fmt::Debug for Route {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Route")
            .field("index", &self.index)
            .field("name", &self.name)
            .field("availability", &self.availability)
            .field("is_active", &self.is_active)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Profile {
    pub index: i32,
    pub name: String,
    pub availability: Availability,
    pub is_active: bool,
    authority_name: String,
}

impl fmt::Debug for Profile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Profile")
            .field("index", &self.index)
            .field("name", &self.name)
            .field("availability", &self.availability)
            .field("is_active", &self.is_active)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HardwareDevice {
    /// Opaque identifier for this device in the currently sampled audio graph.
    pub id: String,
    pub name: String,
    pub profiles: Vec<Profile>,
    authority_name: String,
}

impl fmt::Debug for HardwareDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HardwareDevice")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("profiles", &self.profiles)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Device {
    /// Opaque identifier for this node in the currently sampled audio graph.
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub routes: Vec<Route>,
    pub balance: Option<Balance>,
    authority_name: String,
    authority_device_id: Option<String>,
    authority_route_device: Option<i32>,
}

impl fmt::Debug for Device {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Device")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("is_default", &self.is_default)
            .field("routes", &self.routes)
            .field("balance", &self.balance)
            .field("has_authority_name", &(!self.authority_name.is_empty()))
            .field(
                "has_route_authority",
                &(self.authority_device_id.is_some() && self.authority_route_device.is_some()),
            )
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
    pub has_output: bool,
    pub has_input: bool,
    pub can_set_default: bool,
    pub can_mute_input: bool,
    pub configuration_available: bool,
    pub configuration_error: Option<String>,
    pub output: Level,
    pub input: Level,
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    pub hardware_devices: Vec<HardwareDevice>,
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

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RouteDirection {
    Output,
    Input,
}

#[cfg(any(not(target_os = "macos"), test))]
type ActiveRoutes = std::collections::HashMap<(RouteDirection, i32), i32>;

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Debug)]
struct GraphNode {
    authority_name: String,
    description: String,
    kind: DeviceKind,
    device_id: Option<String>,
    route_device: Option<i32>,
    balance: Option<Balance>,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Debug)]
struct GraphRoute {
    route: Route,
    direction: RouteDirection,
    device_indexes: Vec<i32>,
    profile_indexes: Vec<i32>,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Debug)]
struct GraphHardwareDevice {
    device: HardwareDevice,
    active_profile: i32,
    routes: Vec<GraphRoute>,
    active_routes: ActiveRoutes,
}

#[cfg(any(not(target_os = "macos"), test))]
#[derive(Clone, Debug, Default)]
struct GraphMetadata {
    nodes: std::collections::HashMap<String, GraphNode>,
    hardware: Vec<GraphHardwareDevice>,
    capabilities_rejected: bool,
}

#[cfg(not(target_os = "macos"))]
fn system_snapshot() -> Result<Snapshot, Error> {
    let mut outputs = machine_devices(DeviceKind::Output)?;
    let mut inputs = machine_devices(DeviceKind::Input)?;
    let graph = command("pw-dump", &["--no-colors"], "read PipeWire capabilities")
        .and_then(|dump| parse_pw_dump_metadata(&dump));
    let (graph, mut configuration_error) = match graph {
        Ok(graph) => (Some(graph), None),
        Err(_) => (
            None,
            Some("Audio ports and device profiles could not be read from PipeWire.".into()),
        ),
    };
    if graph
        .as_ref()
        .is_some_and(|graph| graph.capabilities_rejected)
    {
        configuration_error = Some(
            "Some audio port or device profile data was rejected because PipeWire returned an ambiguous response."
                .into(),
        );
    }
    if let Some(graph) = &graph {
        apply_graph_metadata(&mut outputs, graph, DeviceKind::Output);
        apply_graph_metadata(&mut inputs, graph, DeviceKind::Input);
    }
    let output = read_default_level(&outputs, DeviceKind::Output)?;
    let input = read_default_level(&inputs, DeviceKind::Input)?;
    sort_devices(&mut outputs);
    sort_devices(&mut inputs);
    let has_output = default_device_id(&outputs).is_some();
    let has_input = default_device_id(&inputs).is_some();
    let configuration_available = graph.is_some();
    let mut hardware_devices = graph
        .map(|graph| {
            graph
                .hardware
                .into_iter()
                .map(|hardware| hardware.device)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
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
fn read_default_level(devices: &[Device], kind: DeviceKind) -> Result<Level, Error> {
    let Some(id) = default_device_id(devices) else {
        return Ok(Level::default());
    };
    let operation = match kind {
        DeviceKind::Output => "read output volume",
        DeviceKind::Input => "read input volume",
    };
    parse_wpctl_level(&command("wpctl", &["get-volume", id], operation)?)
        .ok_or_else(|| Error::new(operation, "unexpected wpctl response"))
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
fn system_set_profile(
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
    command(
        "wpctl",
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
fn system_set_route(
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
    command(
        "wpctl",
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
fn system_set_balance(expected_device: &Device, value: i8) -> Result<Snapshot, Error> {
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
    command(
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
fn balance_channel_targets(current: &Balance, value: i8) -> (u32, u32) {
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
fn attenuated_channel_volume(maximum: u32, percent: u32) -> u32 {
    ((u64::from(maximum) * u64::from(percent) + 50) / 100) as u32
}

#[cfg(not(target_os = "macos"))]
fn spa_channel_volume(value: u32) -> String {
    format!("{}.{:06}", value / 1_000_000, value % 1_000_000)
}

#[cfg(not(target_os = "macos"))]
fn read_graph_metadata(operation: &'static str) -> Result<GraphMetadata, Error> {
    let dump = command("pw-dump", &["--no-colors"], operation)?;
    parse_pw_dump_metadata(&dump).map_err(|error| Error::new(operation, error.detail))
}

#[cfg(not(target_os = "macos"))]
fn exact_hardware_device<'a>(
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
fn exact_routed_device(
    kind: DeviceKind,
    expected: &Device,
    operation: &'static str,
) -> Result<Device, Error> {
    let mut devices = machine_devices(kind)?;
    let graph = read_graph_metadata(operation)?;
    apply_graph_metadata(&mut devices, &graph, kind);
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
fn exact_route<'a>(
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

#[cfg(target_os = "macos")]
fn system_set_profile(_: &HardwareDevice, _: &Profile) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change audio profile",
        "macOS does not expose profiles through the scripting adapter",
    ))
}

#[cfg(target_os = "macos")]
fn system_set_route(_: DeviceKind, _: &Device, _: &Route) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change audio route",
        "macOS does not expose routes through the scripting adapter",
    ))
}

#[cfg(target_os = "macos")]
fn system_set_balance(_: &Device, _: i8) -> Result<Snapshot, Error> {
    Err(Error::new(
        "change output balance",
        "macOS does not expose balance through the scripting adapter",
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
const MAX_GRAPH_OBJECTS: usize = 4096;
#[cfg(any(not(target_os = "macos"), test))]
const MAX_DEVICE_CAPABILITIES: usize = 128;

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
            routes: Vec::new(),
            balance: None,
            authority_name: authority_name.to_owned(),
            authority_device_id: None,
            authority_route_device: None,
        });
    }
    Ok(devices)
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_pw_dump_metadata(output: &str) -> Result<GraphMetadata, Error> {
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
    for object in &objects {
        let Some(id) = json_u32(object.get("id")) else {
            continue;
        };
        let Some(props) = object.get("info").and_then(|info| info.get("props")) else {
            continue;
        };
        match object.get("type").and_then(Value::as_str) {
            Some("PipeWire:Interface:Node") => {
                let kind = match props.get("media.class").and_then(Value::as_str) {
                    Some("Audio/Sink") => DeviceKind::Output,
                    Some("Audio/Source") => DeviceKind::Input,
                    _ => continue,
                };
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
                };
                let id = id.to_string();
                if graph.nodes.insert(id.clone(), node).is_some() {
                    ambiguous_nodes.insert(id);
                }
            }
            Some("PipeWire:Interface:Device")
                if props.get("media.class").and_then(Value::as_str) == Some("Audio/Device") =>
            {
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
            _ => {}
        }
    }
    for id in ambiguous_nodes {
        graph.nodes.remove(&id);
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
fn bounded_label(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_DEVICE_LABEL_CHARS)
        .collect()
}

#[cfg(any(not(target_os = "macos"), test))]
fn bounded_authority_name(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= MAX_AUTHORITY_NAME_BYTES
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}

#[cfg(any(not(target_os = "macos"), test))]
fn json_u32(value: Option<&serde_json::Value>) -> Option<u32> {
    let value = value?;
    let parsed = value
        .as_u64()
        .or_else(|| value.as_str()?.parse::<u64>().ok())?;
    (parsed > 0 && parsed <= u64::from(u32::MAX)).then_some(parsed as u32)
}

#[cfg(any(not(target_os = "macos"), test))]
fn json_i32(value: Option<&serde_json::Value>) -> Option<i32> {
    let value = value?;
    let parsed = value
        .as_i64()
        .or_else(|| value.as_str()?.parse::<i64>().ok())?;
    (0..=i64::from(i32::MAX))
        .contains(&parsed)
        .then_some(parsed as i32)
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_availability(value: Option<&serde_json::Value>) -> Option<Availability> {
    match value.and_then(serde_json::Value::as_str) {
        Some("yes") => Some(Availability::Available),
        Some("no") => Some(Availability::Unavailable),
        Some("unknown") | None => Some(Availability::Unknown),
        _ => None,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_i32_array(value: Option<&serde_json::Value>) -> Option<Vec<i32>> {
    let values = value?.as_array()?;
    if values.len() > MAX_DEVICE_CAPABILITIES {
        return None;
    }
    values.iter().map(|value| json_i32(Some(value))).collect()
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_graph_hardware_device(
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
fn parse_profiles(params: &serde_json::Value) -> Option<(Vec<Profile>, i32)> {
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
fn parse_routes(
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
fn parse_route_direction(value: Option<&serde_json::Value>) -> Option<RouteDirection> {
    match value.and_then(serde_json::Value::as_str) {
        Some("Output") => Some(RouteDirection::Output),
        Some("Input") => Some(RouteDirection::Input),
        _ => None,
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn parse_node_balance(object: &serde_json::Value) -> Option<Balance> {
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
    let props = params.get("Props")?.as_array()?;
    if props.len() != 1 {
        return None;
    }
    let channel_map = props[0].get("channelMap")?.as_array()?;
    let channel_volumes = props[0].get("channelVolumes")?.as_array()?;
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
fn scaled_channel_volume(value: &serde_json::Value) -> Option<u32> {
    const SCALE: f64 = 1_000_000.0;
    let value = value.as_f64()?;
    (value.is_finite() && (0.0..=10.0).contains(&value)).then_some((value * SCALE).round() as u32)
}

#[cfg(any(not(target_os = "macos"), test))]
fn balance_percent(left: u32, right: u32) -> Option<i8> {
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
fn apply_graph_metadata(devices: &mut [Device], graph: &GraphMetadata, kind: DeviceKind) {
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
    fn original_notification_chime_is_bounded_well_formed_pcm() {
        let wav = default_notification_wav();
        assert!(wav.len() < MAX_NOTIFICATION_SOUND_BYTES);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize,
            wav.len() - 8
        );
        assert_eq!(
            u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
            wav.len() - 44
        );
        assert!(wav[44..].chunks_exact(2).any(|sample| sample != [0, 0]));
        validate_notification_sound(NotificationSoundFormat::WavPcm, wav).unwrap();
    }

    #[test]
    fn encoded_notification_sound_boundary_checks_format_size_and_debug() {
        let opus = [b"OggS".as_slice(), &[0; 24], b"OpusHead"].concat();
        let vorbis = [b"OggS".as_slice(), &[0; 24], b"\x01vorbis"].concat();
        validate_notification_sound(NotificationSoundFormat::OggOpus, &opus).unwrap();
        validate_notification_sound(NotificationSoundFormat::OggVorbis, &vorbis).unwrap();
        assert_eq!(
            validate_notification_sound(NotificationSoundFormat::OggVorbis, &opus),
            Err(NotificationPlaybackError::new(
                NotificationPlaybackErrorKind::InvalidSound
            ))
        );
        assert_eq!(
            validate_notification_sound(
                NotificationSoundFormat::WavPcm,
                &vec![0; MAX_NOTIFICATION_SOUND_BYTES + 1],
            ),
            Err(NotificationPlaybackError::new(
                NotificationPlaybackErrorKind::InvalidSound
            ))
        );
        let debug = format!(
            "{:?}",
            NotificationSound::Encoded {
                format: NotificationSoundFormat::OggOpus,
                bytes: &opus,
            }
        );
        assert!(!debug.contains("OpusHead"));
        assert!(debug.contains(&opus.len().to_string()));
    }

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
            routes: Vec::new(),
            balance: None,
            authority_name: "alsa_output.private-hardware-identity".into(),
            authority_device_id: Some("41".into()),
            authority_route_device: Some(0),
        };
        let output = format!("{device:?}");
        assert!(output.contains("Built-in Audio"));
        assert!(output.contains("has_authority_name: true"));
        assert!(!output.contains("private-hardware-identity"));
    }

    #[test]
    fn pipewire_json_correlates_exact_profiles_routes_and_node_identity() {
        let graph = parse_pw_dump_metadata(
            r#"[
                {"id":52,"type":"PipeWire:Interface:Node","permissions":["r","w","x","m"],"info":{"props":{
                    "media.class":"Audio/Sink","node.name":"alsa_output.analog",
                    "node.description":"Built-in Audio Analog Stereo",
                    "device.id":41,"card.profile.device":4},"params":{
                    "PropInfo":[{"id":"channelVolumes","container":"Array"}],
                    "Props":[{"channelMap":["FL","FR"],"channelVolumes":[0.4,0.8]}]
                }}},
                {"id":53,"type":"PipeWire:Interface:Node","info":{"props":{
                    "media.class":"Stream/Output/Audio","node.description":"Private Stream"}}},
                {"id":41,"type":"PipeWire:Interface:Device","info":{"props":{
                    "media.class":"Audio/Device","device.name":"alsa_card.private",
                    "device.description":"Built-in Audio"},"params":{
                    "EnumProfile":[
                        {"index":0,"name":"off","description":"Off","available":"yes"},
                        {"index":1,"name":"duplex","description":"Analog Stereo Duplex","available":"yes"},
                        {"index":2,"name":"unplugged","description":"Unplugged","available":"no"}
                    ],
                    "Profile":[{"index":1,"name":"duplex"}],
                    "EnumRoute":[
                        {"index":0,"direction":"Output","name":"speaker","description":"Speakers","available":"yes","profiles":[1],"devices":[4]},
                        {"index":1,"direction":"Output","name":"headphones","description":"Headphones","available":"unknown","profiles":[1],"devices":[4]}
                    ],
                    "Route":[{"index":0,"direction":"Output","device":4,"profile":1}]
                }}}
            ]"#,
        )
        .unwrap();
        assert_eq!(graph.hardware.len(), 1);
        assert_eq!(graph.hardware[0].device.name, "Built-in Audio");
        assert_eq!(graph.hardware[0].device.profiles.len(), 3);
        assert!(graph.hardware[0].device.profiles[0].is_active);

        let mut outputs =
            parse_wpctl_list("52\talsa_output.analog\taudio/sink\t*", DeviceKind::Output).unwrap();
        apply_graph_metadata(&mut outputs, &graph, DeviceKind::Output);
        assert_eq!(outputs[0].name, "Built-in Audio Analog Stereo");
        assert_eq!(outputs[0].routes.len(), 2);
        assert!(outputs[0].routes[0].is_active);
        assert_eq!(outputs[0].routes[0].name, "Speakers");
        assert_eq!(outputs[0].authority_device_id.as_deref(), Some("41"));
        assert_eq!(outputs[0].authority_route_device, Some(4));
        assert_eq!(
            outputs[0].balance.as_ref().map(|balance| balance.value),
            Some(50)
        );
        assert!(!graph.nodes.contains_key("53"));
        assert!(parse_pw_dump_metadata("not json").is_err());
    }

    #[test]
    fn route_capabilities_reject_duplicate_indices_and_stale_profiles() {
        let duplicate = serde_json::json!({
            "EnumRoute": [
                {"index": 2, "direction": "Output", "name": "speaker", "devices": [4], "profiles": [1]},
                {"index": 2, "direction": "Output", "name": "headphones", "devices": [4], "profiles": [1]}
            ],
            "Route": []
        });
        assert!(parse_routes(&duplicate, 1).is_none());

        let stale = serde_json::json!({
            "EnumRoute": [
                {"index": 2, "direction": "Output", "name": "speaker", "devices": [4], "profiles": [1]}
            ],
            "Route": [
                {"index": 2, "direction": "Output", "device": 4, "profile": 3}
            ]
        });
        assert!(parse_routes(&stale, 1).is_none());
    }

    #[test]
    fn stereo_balance_preserves_the_louder_channel_without_amplification() {
        let current = Balance {
            value: 50,
            left_volume: 400_000,
            right_volume: 800_000,
            left_first: true,
        };
        assert_eq!(balance_channel_targets(&current, 0), (800_000, 800_000));
        assert_eq!(balance_channel_targets(&current, -25), (800_000, 600_000));
        assert_eq!(balance_channel_targets(&current, 75), (200_000, 800_000));
        assert_eq!(balance_channel_targets(&current, 100), (0, 800_000));
        assert_eq!(balance_channel_targets(&current, -100), (800_000, 0));
    }

    #[test]
    fn balance_requires_writable_exact_front_stereo_channels() {
        let base = serde_json::json!({
            "permissions": ["r", "w", "x"],
            "info": {"params": {
                "PropInfo": [{"id": "channelVolumes", "container": "Array"}],
                "Props": [{"channelMap": ["FL", "FR"], "channelVolumes": [0.5, 0.5]}]
            }}
        });
        assert_eq!(
            parse_node_balance(&base).map(|balance| balance.value),
            Some(0)
        );

        let mut read_only = base.clone();
        read_only["permissions"] = serde_json::json!(["r"]);
        assert!(parse_node_balance(&read_only).is_none());

        let mut surround = base;
        surround["info"]["params"]["Props"][0]["channelMap"] =
            serde_json::json!(["FL", "FR", "FC"]);
        surround["info"]["params"]["Props"][0]["channelVolumes"] =
            serde_json::json!([0.5, 0.5, 0.5]);
        assert!(parse_node_balance(&surround).is_none());
    }

    #[test]
    fn graph_labels_require_the_exact_machine_list_node_name() {
        let graph = parse_pw_dump_metadata(
            r#"[{
                "id":52,"type":"PipeWire:Interface:Node","info":{"props":{
                    "media.class":"Audio/Sink","node.name":"reused.private.node",
                    "node.description":"Wrong Hardware"
                }}
            }]"#,
        )
        .unwrap();
        let mut outputs = parse_wpctl_list(
            "52\tcurrent.private.node\taudio/sink\t*",
            DeviceKind::Output,
        )
        .unwrap();
        apply_graph_metadata(&mut outputs, &graph, DeviceKind::Output);
        assert_eq!(outputs[0].name, "current.private.node");
        assert!(outputs[0].routes.is_empty());
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
