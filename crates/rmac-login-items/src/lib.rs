//! Platform-neutral login-item state and desktop-entry mutation rules.

mod entry;
mod model;

pub use entry::{
    applies_to_session, background_service, parse_entry, validate_id, validate_service_id,
    with_hidden,
};
pub use model::*;

#[cfg(test)]
mod tests;
