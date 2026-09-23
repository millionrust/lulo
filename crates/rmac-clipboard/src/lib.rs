//! Framework-neutral clipboard history for Spotlight's Clipboard view.
//!
//! This crate owns the privacy and bounding policy: which offered MIME type
//! is recorded, which offers are never recorded (password-manager secrets),
//! how an item is summarised for display, and how the history is bounded by
//! count, bytes and age. It performs no I/O; `rmac-clipboard-linux` watches
//! the Wayland clipboard, stores payloads and exports the history on D-Bus.

mod classify;
mod history;
mod model;

pub use classify::*;
pub use history::*;
pub use model::*;

#[cfg(test)]
mod tests;
