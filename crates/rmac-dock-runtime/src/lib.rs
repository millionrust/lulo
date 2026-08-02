//! Event-driven, last-known-good orchestration for the Dock process.

mod consumer;
mod coordinator;
pub mod icons;
mod model;
pub mod session;
mod sources;
pub mod surfaces;
#[cfg(test)]
mod tests;

use std::fmt;
use std::time::Duration;

use async_channel::Sender;

use consumer::*;
pub use model::*;
pub use sources::*;
