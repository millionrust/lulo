//! Typed launch and niri-focus execution for Dock activation outcomes.

mod backend;
pub mod dispatch;
mod execution;
pub mod icons;
pub mod interaction;
mod model;

pub use backend::SystemBackend;
pub use execution::*;
pub use model::*;

#[cfg(test)]
mod tests;
