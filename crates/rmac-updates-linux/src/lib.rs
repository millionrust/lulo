//! Modern, bounded PackageKit authority for the supported Linux session.

mod api;
mod transaction;
mod watch;

pub use api::*;
pub use watch::watch;

#[cfg(test)]
mod tests;
