//! Coherent live compositor and Wayland-seat authority for shell invocations.

mod coordinator;
mod model;
mod watch;

pub use coordinator::Coordinator;
pub use model::*;
pub use watch::*;

#[cfg(test)]
mod tests;
