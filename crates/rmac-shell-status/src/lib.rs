//! Framework-neutral, redraw-aware state for shell status surfaces.
//!
//! Platform adapters publish typed events into a shared reducer without
//! polling one another.

mod model;
mod state;

pub use model::*;
pub use state::State;

#[cfg(test)]
mod tests;
