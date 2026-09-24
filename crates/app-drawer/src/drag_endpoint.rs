//! Client side of the Dock's bounded local "drag from Apps" transport
//! (`shell/bins/rmac-dock/src/drag_endpoint.rs` documents the full design
//! and its one confirmed gap: GPUI's Linux Wayland backend has no
//! drag-source support, only a drop-target one, so this reports Apps' own
//! window-local pointer position instead of a real cross-process drag).
//!
//! Unverified: whether niri keeps delivering pointer motion to Apps' own
//! surface once the drag visually crosses into the Dock's on-screen
//! rectangle (the assumption this whole channel rests on) has not been
//! confirmed interactively on the reference laptop.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

const SOCKET_NAME: &str = "dock-drag.sock";
const MAX_WIRE_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Still dragging; `fraction` is the current position along the Dock's
    /// axis (0.0 leading, 1.0 trailing), clamped by the caller.
    Hover(f32),
    /// The drag ended with the pointer over the Dock.
    Drop(f32),
    /// The drag ended without reaching the Dock, or was cancelled.
    Cancel,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

/// `$XDG_RUNTIME_DIR/rmac/dock-drag.sock`, with the same ownership checks
/// as the Dock's own `socket_path` (duplicated per-binary, matching every
/// other `shell/bins/*` local command endpoint in this repository).
fn socket_path() -> io::Result<PathBuf> {
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
    let metadata = fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid() != owner {
        return Err(invalid("the rmac runtime directory is not private"));
    }
    Ok(directory.join(SOCKET_NAME))
}

/// Best-effort: a Dock that is not running, or a socket that cannot be
/// reached, silently drops the hint. The drag still completes normally
/// within Apps either way (nothing in Apps depends on delivery).
pub fn send(app_id: &str, phase: Phase) {
    let encoded = match phase {
        Phase::Hover(fraction) => format!("hover:{fraction}:{app_id}"),
        Phase::Drop(fraction) => format!("drop:{fraction}:{app_id}"),
        Phase::Cancel => format!("cancel:{app_id}"),
    };
    if encoded.len() > MAX_WIRE_BYTES {
        return;
    }
    let Ok(path) = socket_path() else {
        return;
    };
    let Ok(socket) = UnixDatagram::unbound() else {
        return;
    };
    let _ = socket.send_to(encoded.as_bytes(), path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_never_panics_without_a_running_dock() {
        // XDG_RUNTIME_DIR is whatever the test process has; there is
        // ordinarily no `dock-drag.sock` listener, so this only exercises
        // that a missing/refused destination is handled quietly.
        send("org.rmac.Notes.desktop", Phase::Hover(0.5));
        send("org.rmac.Notes.desktop", Phase::Drop(1.0));
        send("org.rmac.Notes.desktop", Phase::Cancel);
    }
}
