//! Compositor-independent desktop state for the rmac session.
//!
//! This crate deliberately contains no GPUI, socket, niri, or Wayland types.
//! Adapters translate their wire format into these stable identities, snapshots,
//! and incremental events.

mod actions;
mod events;
mod model;
mod parking;
mod state;

pub use actions::*;
pub use events::*;
pub use model::*;
pub use parking::*;
pub use state::*;

#[cfg(test)]
mod tests;
