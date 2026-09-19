//! Writable rmac theme preferences and effective appearance resolution.
//!
//! The store persists user choices separately from the read-only host portal,
//! with versioning, atomic replacement, and last-known-good recovery.

mod model;
mod store;
mod wallpaper;

pub use model::*;
pub use wallpaper::*;

#[cfg(test)]
mod tests;
