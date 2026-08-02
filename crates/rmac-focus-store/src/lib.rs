//! Private, versioned persistence for Focus modes and manual activation.

mod model;
mod store;
mod wire;

pub use model::*;
pub use store::default_config;

#[cfg(test)]
mod tests;
