//! Small cross-platform boundary for user-mediated desktop open operations.

mod chooser;
mod model;
mod open;
#[cfg(windows)]
mod windows_dialog;

pub use chooser::*;
pub use model::*;
pub use open::*;

#[cfg(test)]
mod tests;
