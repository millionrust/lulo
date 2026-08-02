//! Privacy-safe platform identity and hostname administration.
//!
//! Snapshots exclude usernames, machine IDs, serial numbers, network addresses,
//! and paths. Linux hostname changes use systemd-hostnamed and polkit.

mod api;
mod facts;
mod host;
mod watch;

pub use api::*;
#[cfg(target_os = "linux")]
pub use watch::watch;

#[cfg(test)]
mod tests;
