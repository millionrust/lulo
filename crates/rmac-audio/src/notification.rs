//! Notification cue validation and bounded playback.

use std::fmt;

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
    pub(super) fn new(kind: NotificationPlaybackErrorKind) -> Self {
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

pub(super) fn prepare_notification_sound(
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

pub(super) fn validate_notification_sound(
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

pub(super) fn header_prefix_contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes
        .get(..bytes.len().min(256))
        .is_some_and(|prefix| prefix.windows(needle.len()).any(|window| window == needle))
}

/// An original short two-tone rmac cue, encoded as mono 48 kHz PCM WAV.
pub(super) fn default_notification_wav() -> &'static [u8] {
    static CHIME: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    CHIME.get_or_init(generate_default_notification_wav)
}

pub(super) fn generate_default_notification_wav() -> Vec<u8> {
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
pub(super) async fn system_play_notification_sound(
    bytes: &[u8],
) -> Result<(), NotificationPlaybackError> {
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
pub(super) async fn system_play_notification_sound(
    _: &[u8],
) -> Result<(), NotificationPlaybackError> {
    Err(NotificationPlaybackError::new(
        NotificationPlaybackErrorKind::UnsupportedPlatform,
    ))
}
