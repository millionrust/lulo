//! Bounded choice, authority, focus-order, and live-state semantics for Appearance.

mod helpers;
mod model;
mod projection;

pub use model::*;
pub use projection::project_appearance;

#[cfg(test)]
use helpers::*;
#[cfg(test)]
mod tests;
