//! Original rmac interface-sound playback and persistent user policy.
//!
//! Playback opens PipeWire only for one cue. The public call is non-blocking,
//! rate-limited, and bounded; no UI thread waits for audio and no idle process
//! holds a stream open.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const RATE_LIMIT: Duration = Duration::from_millis(50);
const MAX_CONCURRENT: usize = 8;
const MAX_SOUND_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_SOUND_ROOT: &str = "/usr/share/rmac/sounds";
const SETTINGS_FILE: &str = "rmac/sound.json";

/// Every original cue shipped by rmac.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cue {
    Alert,
    Error,
    Trash,
    EmptyTrash,
    Screenshot,
    VolumeTick,
    Mount,
    Unmount,
    PowerPlug,
    Lock,
    Unlock,
    Notification,
    DragDrop,
    Boot,
}

impl Cue {
    pub const ALL: [Self; 14] = [
        Self::Alert,
        Self::Error,
        Self::Trash,
        Self::EmptyTrash,
        Self::Screenshot,
        Self::VolumeTick,
        Self::Mount,
        Self::Unmount,
        Self::PowerPlug,
        Self::Lock,
        Self::Unlock,
        Self::Notification,
        Self::DragDrop,
        Self::Boot,
    ];

    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Alert => "alert.wav",
            Self::Error => "error.wav",
            Self::Trash => "trash.wav",
            Self::EmptyTrash => "empty-trash.wav",
            Self::Screenshot => "screenshot.wav",
            Self::VolumeTick => "volume-tick.wav",
            Self::Mount => "mount.wav",
            Self::Unmount => "unmount.wav",
            Self::PowerPlug => "power-plug.wav",
            Self::Lock => "lock.wav",
            Self::Unlock => "unlock.wav",
            Self::Notification => "notification.wav",
            Self::DragDrop => "drag-drop.wav",
            // The generated asset predates the final user-facing "boot" name.
            Self::Boot => "login.wav",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Alert => "Alert",
            Self::Error => "Error",
            Self::Trash => "Move to Trash",
            Self::EmptyTrash => "Empty Trash",
            Self::Screenshot => "Screenshot",
            Self::VolumeTick => "Volume",
            Self::Mount => "Mount",
            Self::Unmount => "Unmount",
            Self::PowerPlug => "Power Connected",
            Self::Lock => "Lock",
            Self::Unlock => "Unlock",
            Self::Notification => "Notification",
            Self::DragDrop => "Drop",
            Self::Boot => "Login",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }

    const fn is_interface_effect(self) -> bool {
        !matches!(self, Self::Alert | Self::Notification | Self::Boot)
    }
}

/// User-visible policy owned by Settings ▸ Sound.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub alert_sound: Cue,
    pub alert_volume: u8,
    pub interface_effects: bool,
    pub volume_feedback: bool,
    pub login_sound: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            alert_sound: Cue::Alert,
            alert_volume: 80,
            interface_effects: true,
            volume_feedback: true,
            login_sound: false,
        }
    }
}

impl Settings {
    pub fn normalized(mut self) -> Self {
        self.alert_volume = self.alert_volume.min(100);
        if !matches!(
            self.alert_sound,
            Cue::Alert | Cue::Error | Cue::Notification
        ) {
            self.alert_sound = Cue::Alert;
        }
        self
    }

    fn allows(&self, cue: Cue) -> bool {
        if cue == Cue::VolumeTick && !self.volume_feedback {
            return false;
        }
        if cue == Cue::Boot && !self.login_sound {
            return false;
        }
        !cue.is_interface_effect() || self.interface_effects
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayDisposition {
    Scheduled,
    RateLimited,
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Resolve,
    Read,
    Parse,
    Save,
}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
}

impl Error {
    const fn new(kind: ErrorKind) -> Self {
        Self { kind }
    }

    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ErrorKind::Resolve => "sound settings location is unavailable",
            ErrorKind::Read => "sound settings could not be read",
            ErrorKind::Parse => "sound settings are invalid",
            ErrorKind::Save => "sound settings could not be saved",
        })
    }
}

impl std::error::Error for Error {}

fn config_path() -> Result<PathBuf, Error> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|root| root.join(SETTINGS_FILE))
        .ok_or_else(|| Error::new(ErrorKind::Resolve))
}

pub fn load_settings() -> Result<Settings, Error> {
    let path = config_path()?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default())
        }
        Err(_) => return Err(Error::new(ErrorKind::Read)),
    };
    if bytes.len() > 64 * 1024 {
        return Err(Error::new(ErrorKind::Parse));
    }
    serde_json::from_slice::<Settings>(&bytes)
        .map(Settings::normalized)
        .map_err(|_| Error::new(ErrorKind::Parse))
}

pub fn save_settings(settings: &Settings) -> Result<(), Error> {
    let path = config_path()?;
    let bytes = serde_json::to_vec(&settings.clone().normalized())
        .map_err(|_| Error::new(ErrorKind::Save))?;
    rmac_storage::atomic_write_private(&path, &bytes).map_err(|_| Error::new(ErrorKind::Save))
}

struct PlaybackState {
    last_played: [Option<Instant>; Cue::ALL.len()],
}

static PLAYBACK: OnceLock<Mutex<PlaybackState>> = OnceLock::new();
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

fn playback() -> &'static Mutex<PlaybackState> {
    PLAYBACK.get_or_init(|| {
        Mutex::new(PlaybackState {
            last_played: [None; Cue::ALL.len()],
        })
    })
}

/// Schedule one cue. The call never waits for policy queries or PipeWire.
pub fn play(cue: Cue) -> PlayDisposition {
    schedule(cue, cue == Cue::Alert, None)
}

/// Play the configured alert selection. Alert sheets bypass Focus, matching
/// the fail-visible alert exception in the feel contract.
pub fn play_alert() -> PlayDisposition {
    let settings = load_settings().unwrap_or_default();
    schedule(settings.alert_sound, true, Some(settings))
}

/// Preview an alert choice from Settings. Focus does not suppress an explicit
/// preview, but output mute still does.
pub fn preview(cue: Cue, volume: u8) -> PlayDisposition {
    let settings = Settings {
        alert_sound: cue,
        alert_volume: volume,
        ..Settings::default()
    }
    .normalized();
    schedule(settings.alert_sound, true, Some(settings))
}

/// Play one cue synchronously from a short-lived non-UI helper process.
/// Session binaries use this only after an external action reports success.
pub fn play_blocking(cue: Cue) -> bool {
    let settings = load_settings().unwrap_or_default().normalized();
    if !settings.allows(cue) || output_muted() {
        return false;
    }
    if cue != Cue::Alert && focus_active() {
        return false;
    }
    play_file(cue, settings.alert_volume)
}

fn schedule(cue: Cue, bypass_focus: bool, settings: Option<Settings>) -> PlayDisposition {
    let now = Instant::now();
    let mut state = playback()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.last_played[cue.index()].is_some_and(|last| now.duration_since(last) < RATE_LIMIT) {
        return PlayDisposition::RateLimited;
    }
    if ACTIVE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            (active < MAX_CONCURRENT).then_some(active + 1)
        })
        .is_err()
    {
        return PlayDisposition::Busy;
    }
    state.last_played[cue.index()] = Some(now);
    drop(state);

    std::thread::spawn(move || {
        let _active = ActivePlayback;
        let settings = settings
            .or_else(|| load_settings().ok())
            .unwrap_or_default()
            .normalized();
        if !settings.allows(cue) || output_muted() {
            return;
        }
        if !bypass_focus && focus_active() {
            return;
        }
        let _ = play_file(cue, settings.alert_volume);
    });
    PlayDisposition::Scheduled
}

struct ActivePlayback;

impl Drop for ActivePlayback {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

fn output_muted() -> bool {
    rmac_audio::snapshot()
        .map(|snapshot| snapshot.output.muted)
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn focus_active() -> bool {
    rmac_focus_linux::client::state()
        .map(|snapshot| snapshot.projection.enabled)
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn focus_active() -> bool {
    false
}

fn sound_root() -> PathBuf {
    std::env::var_os("RMAC_SOUND_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOUND_ROOT))
}

fn reviewed_sound_path(cue: Cue) -> Option<PathBuf> {
    let path = sound_root().join(cue.file_name());
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    (metadata.file_type().is_file() && metadata.len() <= MAX_SOUND_BYTES).then_some(path)
}

#[cfg(target_os = "linux")]
fn play_file(cue: Cue, volume: u8) -> bool {
    let Some(path) = reviewed_sound_path(cue) else {
        return false;
    };
    let volume = format!("{:.2}", f32::from(volume.min(100)) / 100.0);
    Command::new("pw-play")
        .args([
            "--media-type",
            "Audio",
            "--media-category",
            "Playback",
            "--media-role",
            "Notification",
            "--latency",
            "20ms",
            "--volume",
            volume.as_str(),
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(target_os = "linux"))]
fn play_file(_cue: Cue, _volume: u8) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_inventory_is_exact_and_unique() {
        let mut names = Cue::ALL.map(Cue::file_name).to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Cue::ALL.len());
        assert_eq!(Cue::Boot.file_name(), "login.wav");
    }

    #[test]
    fn defaults_match_the_sound_pane_contract() {
        let settings = Settings::default();
        assert_eq!(settings.alert_sound, Cue::Alert);
        assert_eq!(settings.alert_volume, 80);
        assert!(settings.interface_effects);
        assert!(settings.volume_feedback);
        assert!(!settings.login_sound);
    }

    #[test]
    fn invalid_alert_choice_and_volume_are_normalized() {
        let settings = Settings {
            alert_sound: Cue::Trash,
            alert_volume: 200,
            ..Settings::default()
        }
        .normalized();
        assert_eq!(settings.alert_sound, Cue::Alert);
        assert_eq!(settings.alert_volume, 100);
    }

    #[test]
    fn policy_switches_gate_the_expected_cues() {
        let disabled = Settings {
            interface_effects: false,
            volume_feedback: false,
            login_sound: false,
            ..Settings::default()
        };
        assert!(!disabled.allows(Cue::Trash));
        assert!(!disabled.allows(Cue::VolumeTick));
        assert!(!disabled.allows(Cue::Boot));
        assert!(disabled.allows(Cue::Alert));
        assert!(disabled.allows(Cue::Notification));
    }
}
