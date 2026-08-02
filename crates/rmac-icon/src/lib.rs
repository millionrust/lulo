//! Shared bounded external icon decoding for rmac renderers.
//!
//! Callers must run synchronous decoding on a dedicated worker, never GPUI.

mod cache;
mod decode;
mod model;

pub use cache::Cache;
pub use decode::*;
pub use model::*;

#[cfg(test)]
mod tests;
