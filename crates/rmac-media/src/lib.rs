//! Media playback control over MPRIS: the player that Now Playing shows and
//! the play/pause, next and previous keys act on.

#[cfg(target_os = "linux")]
mod linux;
mod model;

pub use model::*;

/// Every MPRIS player on the session bus, ordered by bus name.
pub fn players() -> Result<Vec<Player>, Error> {
    #[cfg(target_os = "linux")]
    return linux::players();
    #[cfg(not(target_os = "linux"))]
    Err(unavailable())
}

/// The player Now Playing presents and media keys control, if any.
pub fn active_player() -> Result<Option<Player>, Error> {
    #[cfg(target_os = "linux")]
    return linux::active_player();
    #[cfg(not(target_os = "linux"))]
    Err(unavailable())
}

/// Send a command to the active player and remember it as last controlled.
pub fn send(command: Command) -> Result<Player, Error> {
    #[cfg(target_os = "linux")]
    return linux::send(command);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = command;
        Err(unavailable())
    }
}

#[cfg(not(target_os = "linux"))]
fn unavailable() -> Error {
    Error::new(ErrorKind::Unavailable, "media control needs MPRIS on Linux")
}

#[cfg(test)]
mod tests;
