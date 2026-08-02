//! Small cross-platform boundary for user-mediated desktop open operations.

mod chooser;
mod model;
mod open;

pub use chooser::*;
pub use model::*;
pub use open::*;

#[cfg(test)]
mod tests;
