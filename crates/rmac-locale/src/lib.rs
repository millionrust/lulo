//! Platform-neutral system locale and default keyboard metadata.

mod model;
mod normalization;

pub use model::*;
pub use normalization::*;

#[cfg(test)]
mod tests;
