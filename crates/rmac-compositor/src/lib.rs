//! Compositor-independent desktop state for the rmac session.
//!
//! This crate deliberately contains no GPUI, socket, niri, or Wayland types.
//! Adapters translate their wire format into these stable identities, snapshots,
//! and incremental events.

mod actions;
mod events;
mod model;
mod parking;
pub mod placement;
pub mod reveal;
mod state;
mod tile_history;

pub use actions::*;
pub use events::*;
pub use model::*;
pub use parking::*;
pub use state::*;
pub use tile_history::*;

/// Where the shell keeps what lasts only as long as the session (the
/// windows Hide parked, the sizes to return to): `$XDG_RUNTIME_DIR/rmac` on
/// Lulo OS. Windows has no runtime directory, so there it is Lulo's folder
/// in the user's temporary directory; the window handles kept there are
/// pruned against the live windows on every read. Without it, Show All had
/// nothing to show on Windows.
pub(crate) fn runtime_dir() -> Option<std::path::PathBuf> {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Some(std::path::PathBuf::from(runtime).join("rmac"));
    }
    if cfg!(windows) {
        return Some(std::env::temp_dir().join("Lulo").join("runtime"));
    }
    None
}

#[cfg(test)]
mod tests;
