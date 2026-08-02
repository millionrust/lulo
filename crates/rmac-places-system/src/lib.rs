//! Filesystem, portal, and freedesktop Trash adapter for user places.

mod backend;
mod model;
mod snapshot;
mod trash;

pub use backend::SystemBackend;
pub use model::*;
pub use snapshot::*;
pub use trash::*;

#[cfg(test)]
mod tests;
