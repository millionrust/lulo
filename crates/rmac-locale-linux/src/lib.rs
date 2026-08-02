//! Linux systemd-localed adapter.

mod service;
mod system;
mod watch;

pub use service::*;
pub use watch::watch;

#[cfg(test)]
mod tests;
