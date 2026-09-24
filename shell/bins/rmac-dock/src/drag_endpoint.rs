//! Bounded local transport for keeping an application dragged out of Apps
//! (`crates/app-drawer`) in the Dock, as macOS lets a Launchpad drag land on
//! the Dock.
//!
//! GPUI's vendored Linux Wayland backend (`shell/compat/gpui_linux`) never
//! creates a `wl_data_source` and never calls `wl_data_device.start_drag`;
//! it only ever receives drops (see ADR 0013's "Cross-process drag source"
//! amendment). A real Wayland drag therefore cannot carry a dragged Apps
//! tile to the Dock's separate process, so this is the Dock command
//! endpoint the ADR describes instead: while the pointer button that
//! started the drag in Apps is still held, most Wayland compositors
//! (niri included) keep delivering pointer motion to the surface that
//! received the press, even once the pointer visually crosses into another
//! surface's on-screen rectangle. Apps can therefore keep sending its own
//! window-local pointer position here for as long as the press lasts, and
//! the Dock turns that into the same live insertion gap an in-Dock drag
//! shows.
//!
//! This channel is a hint, not an authority: `Drop` only tells the Dock
//! which application and where along its axis to insert it; the Dock still
//! resolves the application against its own catalog and applies
//! `PinCommand::Pin` then `PinCommand::MoveTo` exactly as a `.desktop` file
//! drop does (`keep_dropped_applications`).
//!
//! Unverified: whether niri actually keeps the pointer grabbed on Apps'
//! surface once it visually leaves Apps' window has not been confirmed on
//! the reference laptop by this change; it needs an interactive check.

use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

use crate::ipc::{invalid, socket_path};

const SOCKET_NAME: &str = "dock-drag.sock";
/// Generous enough for a `.desktop` id/basename plus a short fraction, far
/// short of anything that could be mistaken for a real payload channel.
const MAX_WIRE_BYTES: usize = 512;
const MAX_APP_ID_BYTES: usize = 400;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// The dragged application's pointer position, as a fraction of the
    /// Dock's own axis length (0.0 leading, 1.0 trailing). The sender does
    /// not clamp it; the Dock does.
    Hover { app_id: String, fraction: f32 },
    /// The drag ended with the pointer over the Dock: keep the application
    /// at this position.
    Drop { app_id: String, fraction: f32 },
    /// The drag ended without reaching the Dock, or was cancelled.
    Cancel { app_id: String },
}

impl Command {
    fn encode(&self) -> String {
        match self {
            Self::Hover { app_id, fraction } => format!("hover:{fraction}:{app_id}"),
            Self::Drop { app_id, fraction } => format!("drop:{fraction}:{app_id}"),
            Self::Cancel { app_id } => format!("cancel:{app_id}"),
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let (kind, rest) = text.split_once(':')?;
        match kind {
            "cancel" => valid_app_id(rest).map(|app_id| Self::Cancel { app_id }),
            "hover" | "drop" => {
                let (fraction, app_id) = rest.split_once(':')?;
                let fraction: f32 = fraction.parse().ok()?;
                if !fraction.is_finite() {
                    return None;
                }
                let app_id = valid_app_id(app_id)?;
                Some(if kind == "hover" {
                    Self::Hover { app_id, fraction }
                } else {
                    Self::Drop { app_id, fraction }
                })
            }
            _ => None,
        }
    }
}

fn valid_app_id(app_id: &str) -> Option<String> {
    let trimmed = app_id.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_APP_ID_BYTES
        || trimmed.chars().any(char::is_control)
    {
        return None;
    }
    Some(trimmed.to_owned())
}

pub struct Listener {
    socket: UnixDatagram,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Listener {
    pub fn bind() -> io::Result<Self> {
        let path = socket_path(SOCKET_NAME)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(&path)?,
            Ok(_) => return Err(invalid("the Dock drag socket path is not a socket")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let socket = UnixDatagram::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::metadata(&path)?;
        Ok(Self {
            socket,
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    /// Block until one valid command arrives; malformed datagrams are
    /// ignored, matching `ipc::Listener::receive`.
    pub fn receive(&self) -> io::Result<Command> {
        let mut buffer = [0_u8; MAX_WIRE_BYTES + 1];
        loop {
            match self.socket.recv(&mut buffer) {
                Ok(length) if length <= MAX_WIRE_BYTES => {
                    if let Some(command) = Command::decode(&buffer[..length]) {
                        return Ok(command);
                    }
                }
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let ours = fs::metadata(&self.path)
            .is_ok_and(|metadata| metadata.dev() == self.device && metadata.ino() == self.inode);
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn send(command: &Command) -> io::Result<()> {
    let socket = UnixDatagram::unbound()?;
    let encoded = command.encode();
    if encoded.len() > MAX_WIRE_BYTES {
        return Err(invalid("Dock drag command is too large"));
    }
    socket.send_to(encoded.as_bytes(), socket_path(SOCKET_NAME)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_command_kind() {
        for command in [
            Command::Hover {
                app_id: "org.rmac.Notes.desktop".into(),
                fraction: 0.42,
            },
            Command::Drop {
                app_id: "org.rmac.Notes.desktop".into(),
                fraction: 1.0,
            },
            Command::Cancel {
                app_id: "org.rmac.Notes.desktop".into(),
            },
        ] {
            let encoded = command.encode();
            assert_eq!(Command::decode(encoded.as_bytes()), Some(command));
        }
    }

    #[test]
    fn rejects_malformed_or_oversized_datagrams() {
        assert_eq!(Command::decode(b"hover:not-a-number:a.desktop"), None);
        assert_eq!(Command::decode(b"hover:nan:a.desktop"), None);
        assert_eq!(Command::decode(b"hover:0.5:"), None);
        assert_eq!(Command::decode(b"bogus:0.5:a.desktop"), None);
        assert_eq!(Command::decode(&[0xff, 0xfe]), None);
        let long_id = "a".repeat(MAX_APP_ID_BYTES + 1);
        assert_eq!(
            Command::decode(format!("cancel:{long_id}").as_bytes()),
            None
        );
    }

    #[test]
    fn rejects_a_control_character_app_id() {
        assert_eq!(Command::decode(b"cancel:a\ndesktop"), None);
    }
}
