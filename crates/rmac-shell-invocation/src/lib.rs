//! Truthful output and Wayland-seat context for shell-surface invocations.

mod inventory;
mod registry;
mod resolution;

pub use inventory::{InventoryError, SeatId, SeatInventory, MAX_SEATS, MAX_SEAT_ID_BYTES};
pub use registry::{RegistryError, SeatRegistry, REQUIRED_WL_SEAT_VERSION};
pub use resolution::{global_shortcut, surface_control, Invocation, ResolveError};

#[cfg(target_os = "linux")]
pub mod wayland;

#[cfg(test)]
mod tests;
