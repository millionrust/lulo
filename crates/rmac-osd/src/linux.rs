use super::{Command, Error, Kind, Operation, Presentation};
use std::fs;
use std::io;
use std::os::unix::fs::{
    DirBuilderExt as _, FileTypeExt as _, MetadataExt as _, PermissionsExt as _,
};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const AUDIO_STEP: u8 = 6;
const MAX_BACKLIGHTS: usize = 32;
const MAX_DEVICE_NAME_BYTES: usize = 255;
const MAX_WIRE_BYTES: usize = 2048;
const MAX_SESSION_ID_BYTES: usize = 255;
const VERIFY_TIMEOUT: Duration = Duration::from_secs(1);
const VERIFY_INTERVAL: Duration = Duration::from_millis(25);

pub(super) fn execute(command: Command) -> Result<Presentation, Error> {
    match command {
        Command::VolumeUp
        | Command::VolumeDown
        | Command::ToggleOutputMute
        | Command::ToggleInputMute
        | Command::ShowVolume => execute_audio(command),
        Command::BrightnessUp | Command::BrightnessDown | Command::ShowBrightness => {
            execute_brightness(command)
        }
    }
}

fn execute_audio(command: Command) -> Result<Presentation, Error> {
    use rmac_audio::DeviceKind;

    let kind = if command == Command::ToggleInputMute {
        DeviceKind::Input
    } else {
        DeviceKind::Output
    };
    let before = rmac_audio::default_device(kind).map_err(|_| Error::new(Operation::ReadAudio))?;
    match command {
        Command::VolumeUp | Command::VolumeDown => {
            let volume = match command {
                Command::VolumeUp => before.level.volume.saturating_add(AUDIO_STEP).min(100),
                Command::VolumeDown => before.level.volume.saturating_sub(AUDIO_STEP),
                _ => unreachable!(),
            };
            rmac_audio::set_volume(DeviceKind::Output, volume)
                .map_err(|_| Error::new(Operation::ChangeAudio))?;
            if before.level.muted {
                rmac_audio::set_muted(DeviceKind::Output, false)
                    .map_err(|_| Error::new(Operation::ChangeAudio))?;
            }
        }
        Command::ToggleOutputMute => {
            rmac_audio::set_muted(DeviceKind::Output, !before.level.muted)
                .map_err(|_| Error::new(Operation::ChangeAudio))?;
        }
        Command::ToggleInputMute => {
            rmac_audio::set_muted(DeviceKind::Input, !before.level.muted)
                .map_err(|_| Error::new(Operation::ChangeAudio))?;
        }
        Command::ShowVolume => {}
        _ => unreachable!(),
    }

    let after = rmac_audio::default_device(kind).map_err(|_| Error::new(Operation::ReadAudio))?;
    if matches!(command, Command::VolumeUp | Command::VolumeDown) {
        let _ = rmac_sound::play(rmac_sound::Cue::VolumeTick);
    }
    Presentation::new(
        if kind == DeviceKind::Input {
            Kind::Input
        } else {
            Kind::Output
        },
        truncate_utf8(&after.name, 128),
        after.level.volume,
        after.level.muted,
    )
}

fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].trim_end().to_owned()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Backlight {
    name: String,
    current: u32,
    maximum: u32,
    kind: String,
}

impl Backlight {
    fn percentage(&self) -> u8 {
        let percentage =
            (u64::from(self.current) * 100 + u64::from(self.maximum) / 2) / u64::from(self.maximum);
        percentage.min(100) as u8
    }

    fn adjusted(&self, direction: Direction) -> u32 {
        let step = self.maximum.div_ceil(16).max(1);
        match direction {
            Direction::Up => self.current.saturating_add(step).min(self.maximum),
            Direction::Down => self.current.saturating_sub(step),
        }
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
}

fn execute_brightness(command: Command) -> Result<Presentation, Error> {
    let mut backlight = preferred_backlight()?;
    let direction = match command {
        Command::BrightnessUp => Some(Direction::Up),
        Command::BrightnessDown => Some(Direction::Down),
        Command::ShowBrightness => None,
        _ => unreachable!(),
    };
    if let Some(direction) = direction {
        let target = backlight.adjusted(direction);
        set_brightness(&backlight.name, target)?;
        let deadline = Instant::now() + VERIFY_TIMEOUT;
        loop {
            backlight = read_backlight(&backlight.name)?;
            if backlight.current == target {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Error::new(Operation::ChangeBrightness));
            }
            std::thread::sleep(VERIFY_INTERVAL);
        }
    }
    Presentation::new(Kind::Display, "Display", backlight.percentage(), false)
}

fn preferred_backlight() -> Result<Backlight, Error> {
    let directory =
        fs::read_dir("/sys/class/backlight").map_err(|_| Error::new(Operation::ReadBrightness))?;
    let mut devices = Vec::new();
    for entry in directory.take(MAX_BACKLIGHTS + 1) {
        let entry = entry.map_err(|_| Error::new(Operation::ReadBrightness))?;
        if devices.len() == MAX_BACKLIGHTS {
            return Err(Error::new(Operation::ReadBrightness));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::new(Operation::ReadBrightness))?;
        if !valid_device_name(&name) {
            return Err(Error::new(Operation::ReadBrightness));
        }
        devices.push(read_backlight(&name)?);
    }
    devices
        .into_iter()
        .min_by_key(|device| (backlight_priority(&device.kind), device.name.clone()))
        .ok_or_else(|| Error::new(Operation::ReadBrightness))
}

fn read_backlight(name: &str) -> Result<Backlight, Error> {
    if !valid_device_name(name) {
        return Err(Error::new(Operation::ReadBrightness));
    }
    let base = Path::new("/sys/class/backlight").join(name);
    let current = read_u32(&base.join("brightness"))?;
    let maximum = read_u32(&base.join("max_brightness"))?;
    if maximum == 0 || current > maximum {
        return Err(Error::new(Operation::ReadBrightness));
    }
    let kind = fs::read_to_string(base.join("type"))
        .unwrap_or_default()
        .trim()
        .to_owned();
    Ok(Backlight {
        name: name.to_owned(),
        current,
        maximum,
        kind,
    })
}

fn read_u32(path: &Path) -> Result<u32, Error> {
    let value = fs::read_to_string(path).map_err(|_| Error::new(Operation::ReadBrightness))?;
    value
        .trim()
        .parse()
        .map_err(|_| Error::new(Operation::ReadBrightness))
}

fn valid_device_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DEVICE_NAME_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn backlight_priority(value: &str) -> u8 {
    match value {
        "firmware" => 0,
        "platform" => 1,
        "raw" => 2,
        _ => 3,
    }
}

fn set_brightness(device: &str, brightness: u32) -> Result<(), Error> {
    let session_id = std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|value| valid_session_id(value))
        .ok_or_else(|| Error::new(Operation::ResolveSession))?;
    let connection =
        zbus::blocking::Connection::system().map_err(|_| Error::new(Operation::ResolveSession))?;
    let manager = LoginManagerProxyBlocking::new(&connection)
        .map_err(|_| Error::new(Operation::ResolveSession))?;
    let path = manager
        .get_session(&session_id)
        .map_err(|_| Error::new(Operation::ResolveSession))?;
    let session = LoginSessionProxyBlocking::builder(&connection)
        .path(path)
        .map_err(|_| Error::new(Operation::ResolveSession))?
        .build()
        .map_err(|_| Error::new(Operation::ResolveSession))?;
    validate_session(&session)?;
    session
        .set_brightness("backlight", device, brightness)
        .map_err(|_| Error::new(Operation::ChangeBrightness))
}

fn validate_session(session: &LoginSessionProxyBlocking<'_>) -> Result<(), Error> {
    let (uid, _) = session
        .user()
        .map_err(|_| Error::new(Operation::ResolveSession))?;
    let remote = session
        .remote()
        .map_err(|_| Error::new(Operation::ResolveSession))?;
    let session_type = session
        .session_type()
        .map_err(|_| Error::new(Operation::ResolveSession))?;
    let (seat, _) = session
        .seat()
        .map_err(|_| Error::new(Operation::ResolveSession))?;
    if uid == effective_uid() && !remote && session_type == "wayland" && !seat.is_empty() {
        Ok(())
    } else {
        Err(Error::new(Operation::ResolveSession))
    }
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SESSION_ID_BYTES
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` takes no arguments and cannot violate Rust memory invariants.
    unsafe { libc::geteuid() }
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    fn get_session(&self, session_id: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait LoginSession {
    fn set_brightness(&self, subsystem: &str, name: &str, brightness: u32) -> zbus::Result<()>;

    #[zbus(property)]
    fn remote(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "Type")]
    fn session_type(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn seat(&self) -> zbus::Result<(String, zbus::zvariant::OwnedObjectPath)>;

    #[zbus(property)]
    fn user(&self) -> zbus::Result<(u32, zbus::zvariant::OwnedObjectPath)>;
}

fn runtime_socket_path() -> Result<PathBuf, Error> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| Error::new(Operation::ResolveRuntime))?;
    let metadata =
        fs::symlink_metadata(&runtime).map_err(|_| Error::new(Operation::ResolveRuntime))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid() != effective_uid()
    {
        return Err(Error::new(Operation::ResolveRuntime));
    }
    let directory = runtime.join("rmac");
    match fs::symlink_metadata(&directory) {
        Ok(metadata)
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == effective_uid() => {}
        Ok(_) => return Err(Error::new(Operation::ResolveRuntime)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder
                .create(&directory)
                .map_err(|_| Error::new(Operation::ResolveRuntime))?;
        }
        Err(_) => return Err(Error::new(Operation::ResolveRuntime)),
    }
    Ok(directory.join("osd.sock"))
}

pub struct Listener {
    socket: UnixDatagram,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Listener {
    pub fn bind() -> Result<Self, Error> {
        let path = runtime_socket_path()?;
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && !metadata.file_type().is_symlink()
                    && metadata.uid() == effective_uid() =>
            {
                fs::remove_file(&path).map_err(|_| Error::new(Operation::BindTransport))?;
            }
            Ok(_) => return Err(Error::new(Operation::BindTransport)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::new(Operation::BindTransport)),
        }
        let socket = UnixDatagram::bind(&path).map_err(|_| Error::new(Operation::BindTransport))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|_| Error::new(Operation::BindTransport))?;
        let metadata = fs::metadata(&path).map_err(|_| Error::new(Operation::BindTransport))?;
        Ok(Self {
            socket,
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    pub fn receive(&self) -> Result<Presentation, Error> {
        let mut buffer = [0_u8; MAX_WIRE_BYTES + 1];
        loop {
            match self.socket.recv(&mut buffer) {
                Ok(length) if length <= MAX_WIRE_BYTES => {
                    if let Ok(presentation) = decode(&buffer[..length]) {
                        return Ok(presentation);
                    }
                }
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(Error::new(Operation::ReadTransport)),
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let remove = fs::metadata(&self.path)
            .is_ok_and(|metadata| metadata.dev() == self.device && metadata.ino() == self.inode);
        if remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn send(presentation: &Presentation) -> Result<(), Error> {
    presentation.validate()?;
    let bytes =
        serde_json::to_vec(presentation).map_err(|_| Error::new(Operation::SendTransport))?;
    if bytes.len() > MAX_WIRE_BYTES {
        return Err(Error::new(Operation::SendTransport));
    }
    let socket = UnixDatagram::unbound().map_err(|_| Error::new(Operation::SendTransport))?;
    socket
        .send_to(&bytes, runtime_socket_path()?)
        .map_err(|_| Error::new(Operation::SendTransport))?;
    Ok(())
}

pub(super) fn decode(bytes: &[u8]) -> Result<Presentation, Error> {
    if bytes.is_empty() || bytes.len() > MAX_WIRE_BYTES {
        return Err(Error::new(Operation::ReadTransport));
    }
    let presentation: Presentation =
        serde_json::from_slice(bytes).map_err(|_| Error::new(Operation::ReadTransport))?;
    presentation.validate()?;
    Ok(presentation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backlight_steps_follow_sixteen_bounded_divisions() {
        let device = Backlight {
            name: "intel_backlight".into(),
            current: 468,
            maximum: 937,
            kind: "raw".into(),
        };
        assert_eq!(device.percentage(), 50);
        assert_eq!(device.adjusted(Direction::Up), 527);
        assert_eq!(device.adjusted(Direction::Down), 409);

        let full = Backlight {
            current: 937,
            ..device
        };
        assert_eq!(full.adjusted(Direction::Up), 937);
    }

    #[test]
    fn device_and_session_identifiers_are_bounded() {
        assert!(valid_device_name("intel_backlight"));
        assert!(!valid_device_name("../brightness"));
        assert!(!valid_device_name("name/child"));
        assert!(valid_session_id("2"));
        assert!(!valid_session_id("2 3"));
    }

    #[test]
    fn firmware_backlights_win_stably() {
        assert!(backlight_priority("firmware") < backlight_priority("platform"));
        assert!(backlight_priority("platform") < backlight_priority("raw"));
    }
}
