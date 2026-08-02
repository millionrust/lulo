//! Cross-platform discovery and unmounting of user-visible volumes.

mod inventory;
mod model;
mod mutation;
mod watch;

pub use inventory::{discover, revalidate, volumes};
pub use model::*;
pub use mutation::unmount;
pub use watch::watch;

#[cfg(test)]
mod tests;
