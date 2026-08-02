//! Security state machine for the rmac `ext-session-lock-v1` provider.
//!
//! Credentials never enter this adapter-neutral model; only matching attempt
//! identifiers and outcomes can create the move-only unlock authority.

mod model;
mod provider;

pub use model::*;
pub use provider::{Error, Provider};

#[cfg(test)]
mod tests;
