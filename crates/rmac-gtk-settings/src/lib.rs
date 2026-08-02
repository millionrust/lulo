//! GNOME interface text scaling for GTK applications.
//!
//! GSettings is authoritative; mutations require writability and exact
//! readback from the same authority.

mod api;
mod process;
mod watch;

pub use api::{set_text_scale, snapshot, watch, Error, Snapshot, WatchEvent};

#[cfg(test)]
mod tests;
