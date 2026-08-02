//! Live, last-known-good orchestration for the wallpaper session process.

pub mod accessibility;
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
