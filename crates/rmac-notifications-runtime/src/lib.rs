//! Live authority coordinator for notification banners.
//!
//! Notification metadata combines with compositor topology and appearance
//! motion without copying private notification content.

mod coordinator;
mod model;
pub mod presentation;

pub use coordinator::Coordinator;
pub use model::*;

#[cfg(test)]
mod tests;
