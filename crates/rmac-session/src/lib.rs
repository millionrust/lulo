//! Typed health and safe-mode state for the systemd-supervised rmac session.

mod model;
mod platform;
mod supervisor;

pub use model::*;
pub use platform::*;
pub use supervisor::{parse_component_health, Supervisor};

#[cfg(test)]
mod tests;
