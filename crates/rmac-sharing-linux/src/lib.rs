//! Linux sharing adapter backed by systemd and read-only firewall inspection.

mod service;
mod system;
mod watch;

pub use service::{set_file_sharing, set_remote_login, snapshot, SystemService};
pub use watch::watch;

#[cfg(test)]
mod tests;
