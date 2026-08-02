//! Framework-neutral Focus modes, schedules, and notification enforcement.

mod engine;
mod model;

pub use engine::Engine;
pub use model::*;

#[cfg(test)]
mod tests;
