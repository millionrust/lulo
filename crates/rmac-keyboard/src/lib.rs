//! Mac keyboard behaviour on PC keyboards, system-wide.
//!
//! rmac's own apps read ⌘ as Super. Apps written for PC keyboards (Firefox,
//! GTK, Electron, terminals) expect Control. This crate owns the pure
//! generator for both halves of the fix described in
//! `docs/decisions/0017-mac-keyboard.md`:
//!
//! * the physical layout — which key is ⌘, what Caps Lock does, whether ⌥
//!   types special characters — expressed either as XKB options for
//!   systemd-localed or, when Mac shortcuts are on, as a keyd configuration;
//! * the per-application keyd bindings the session follower applies when
//!   niri's focus moves between rmac apps, PC apps and terminals.
//!
//! Everything here is side-effect free and unit-tested; the Linux side lives
//! in [`system`] and the `rmac-mac-keyboard` binary.

mod generator;
mod helper;
mod model;
#[cfg(target_os = "linux")]
pub mod system;
#[cfg(test)]
mod tests;

pub use generator::*;
pub use helper::*;
pub use model::*;
#[cfg(target_os = "linux")]
pub use system::{apply, status};

/// The current Mac keyboard state (Linux session only).
#[cfg(not(target_os = "linux"))]
pub fn status() -> Result<Status, Error> {
    Err(Error::new(
        "Mac keyboard settings are available in the rmac Linux session",
    ))
}

/// Apply a Mac keyboard state (Linux session only).
#[cfg(not(target_os = "linux"))]
pub fn apply(_target: &MacKeyboard) -> Result<Status, Error> {
    status()
}
