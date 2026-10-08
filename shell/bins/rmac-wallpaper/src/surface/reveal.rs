//! Click wallpaper to show desktop. A plain click on bare wallpaper asks the
//! Mission Control service, which owns window placement
//! (docs/decisions/0014-mission-control.md), to push the windows aside or
//! bring them back. The service reads the Desktop & Dock setting, so the
//! wallpaper only reports the click.

use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

/// The service's socket and word (rmac-mission-control ipc.rs and
/// model::Command::WallpaperClick).
const SOCKET: &str = "rmac/mission-control.sock";
const WORD: &[u8] = b"wallpaper-click";

/// Report one wallpaper click. Never blocks the pointer: the datagram is
/// sent without waiting, and a missing service is logged.
pub(crate) fn wallpaper_clicked() {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    else {
        eprintln!("rmac-wallpaper: XDG_RUNTIME_DIR is not set; cannot show the desktop");
        return;
    };
    let sent = UnixDatagram::unbound().and_then(|socket| {
        socket.set_nonblocking(true)?;
        socket.send_to(WORD, runtime.join(SOCKET))
    });
    if let Err(error) = sent {
        eprintln!("rmac-wallpaper: Mission Control did not take the wallpaper click: {error}");
    }
}
