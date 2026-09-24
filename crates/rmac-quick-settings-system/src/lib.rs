//! Blocking platform executor for typed quick-settings operations.
//!
//! Call [`execute`] from a blocking/background executor, never a render path.

mod backend;
mod execution;
mod model;

pub use backend::{Backend, SystemBackend};
pub use execution::execute;

/// Ask NetworkManager for a fresh scan while the Wi-Fi list is open. The
/// result arrives through the normal Wi-Fi watch; nothing polls.
pub fn request_wifi_scan() -> Result<(), String> {
    rmac_network::request_scan().map_err(|error| error.to_string())
}
pub use model::{Error, Phase};

#[cfg(test)]
mod tests;
