//! Linux systemd-timedated adapter.

mod api;
mod system;
mod watch;

pub use api::*;
pub use watch::*;

#[cfg(test)]
mod tests;
