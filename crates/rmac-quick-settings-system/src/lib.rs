//! Blocking platform executor for typed quick-settings operations.
//!
//! Call [`execute`] from a blocking/background executor, never a render path.

mod backend;
mod execution;
mod model;

pub use backend::{Backend, SystemBackend};
pub use execution::execute;
pub use model::{Error, Phase};

#[cfg(test)]
mod tests;
