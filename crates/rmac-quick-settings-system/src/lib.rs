//! Blocking platform executor for typed quick-settings operations.
//!
//! Call [`execute`] from a blocking/background executor, never a render path.

mod backend;
mod execution;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod model;

pub use backend::{Backend, SystemBackend};
pub use execution::execute;

/// Ask NetworkManager for a fresh scan while the Wi-Fi list is open. The
/// result arrives through the normal Wi-Fi watch; nothing polls.
/// The Focus service's modes, for Control Centre's Focus view.
pub fn focus_modes() -> Result<Vec<rmac_quick_settings::detail::FocusMode>, String> {
    rmac_focus_linux::client::configuration()
        .map(|configuration| {
            configuration
                .modes()
                .map(|mode| rmac_quick_settings::detail::FocusMode {
                    id: mode.id().as_str().to_owned(),
                    name: mode.name().to_owned(),
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

pub fn request_wifi_scan() -> Result<(), String> {
    rmac_network::request_scan().map_err(|error| error.to_string())
}
pub use model::{Error, Phase};

#[cfg(test)]
mod tests;
