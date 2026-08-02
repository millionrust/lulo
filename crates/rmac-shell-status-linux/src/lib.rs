//! Linux service-change adapter for the shell status projection.
//!
//! This crate emits coalesced refresh hints. The owning domain crates still
//! read authoritative snapshots and perform mutations; signal payloads are not
//! treated as partial state.

mod model;
mod watch;

pub use model::{Error, Event, Sources};
pub use watch::watch;

#[cfg(test)]
mod tests;
