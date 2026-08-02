//! Live, last-known-good orchestration for shell status consumers.

mod coordinator;
mod model;
mod runtime;

pub use coordinator::Coordinator;
pub use model::*;
pub use runtime::{watch, ServiceReader, SystemServiceReader};

#[cfg(test)]
mod tests;
