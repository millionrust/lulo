//! Live, last-known-good orchestration for the wallpaper session process.

mod coordinator;
mod model;
pub mod session;
pub mod surfaces;
mod watch;

pub use coordinator::Coordinator;
pub use model::*;
pub use watch::watch;

#[cfg(test)]
mod tests;
