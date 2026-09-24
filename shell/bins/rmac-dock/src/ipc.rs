//! Bounded local transport from the niri ⌃F3 bind to the resident Dock.
//!
//! niri spawns `rmac-dock focus` for ⌃F3. That process sends one fixed word
//! to a user-private datagram socket and exits; the resident Dock (started
//! with no arguments by rmac-dock.service) then takes the keyboard.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

const SOCKET_NAME: &str = "dock.sock";
const MAX_WIRE_BYTES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    /// ⌃F3: move keyboard focus to the Dock.
    Focus,
}

impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "focus" => Some(Self::Focus),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Focus => "focus",
        }
    }
}

pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

/// `$XDG_RUNTIME_DIR/rmac/<name>`, refusing symlinks and directories owned
/// by anyone but the runtime directory's owner. Shared by every bounded
/// local Dock socket (`dock.sock` here, `dock-drag.sock` in
/// `drag_endpoint`), so the privacy checks are written and reviewed once.
pub(crate) fn socket_path(name: &str) -> io::Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| invalid("XDG_RUNTIME_DIR is not set"))?;
    let runtime_metadata = fs::symlink_metadata(&runtime)?;
    if !runtime_metadata.is_dir() || runtime_metadata.file_type().is_symlink() {
        return Err(invalid("XDG_RUNTIME_DIR is not a private directory"));
    }
    let owner = runtime_metadata.uid();
    let directory = runtime.join("rmac");
    match fs::symlink_metadata(&directory) {
        Ok(metadata)
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == owner => {}
        Ok(_) => return Err(invalid("the rmac runtime directory is not private")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(&directory)?;
        }
        Err(error) => return Err(error),
    }
    Ok(directory.join(name))
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
            Ok(_) => return Err(invalid("the Dock socket path is not a socket")),
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
    /// ignored.
    pub fn receive(&self) -> io::Result<Command> {
        let mut buffer = [0_u8; MAX_WIRE_BYTES + 1];
        loop {
            match self.socket.recv(&mut buffer) {
                Ok(length) if length <= MAX_WIRE_BYTES => {
                    if let Some(command) = decode(&buffer[..length]) {
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

pub fn send(command: Command) -> io::Result<()> {
    let socket = UnixDatagram::unbound()?;
    socket.send_to(command.as_str().as_bytes(), socket_path(SOCKET_NAME)?)?;
    Ok(())
}

fn decode(bytes: &[u8]) -> Option<Command> {
    std::str::from_utf8(bytes).ok().and_then(Command::parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_only_the_exact_word() {
        assert_eq!(decode(b"focus"), Some(Command::Focus));
        assert_eq!(
            decode(Command::Focus.as_str().as_bytes()),
            Some(Command::Focus)
        );
        assert_eq!(decode(b"focus\n"), None);
        assert_eq!(decode(b"next"), None);
        assert_eq!(decode(&[0xff, 0xfe]), None);
    }
}
