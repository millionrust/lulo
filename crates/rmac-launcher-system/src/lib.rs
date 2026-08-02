//! Private-safe execution boundary for launcher actions.

mod backend;
mod execution;
mod model;

pub use backend::{Backend, BackendFuture, Surface, SystemBackend};
pub use execution::execute;
pub use model::{ActivationId, BackendError, Error, FailureKind, Operation, Outcome, Receipt};

#[cfg(test)]
mod tests;
