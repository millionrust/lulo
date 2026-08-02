//! Event-driven process boundary for the rmac top bar.

mod coordinator;
mod watch;

pub mod session;
pub mod surfaces;

pub use coordinator::Coordinator;
pub use watch::{watch, Error};

#[cfg(test)]
mod tests;
