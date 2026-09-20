//! Platform-neutral system locale and default keyboard metadata.

mod model;
mod normalization;
mod vocabulary;

pub use model::*;
pub use normalization::*;
pub use vocabulary::*;

#[cfg(test)]
mod tests;
