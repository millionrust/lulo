//! Process bridge from one shortcut endpoint to one coherent shell invocation.

mod activation;
mod context;
mod coordinator;
mod watch;

pub use activation::{Activation, ActivationError};
pub use context::{Context, LogicalBounds, PlacementError};
pub use coordinator::{Coordinator, Update};
#[cfg(target_os = "linux")]
pub use watch::watch;
pub use watch::{Error, Operation};

#[cfg(test)]
mod tests;
