//! Framework-neutral, per-output presentation model for the rmac top bar.

mod labels;
mod model;
mod projection;

pub use labels::{active_app_name, clock_label, delay_until_next_clock_update, indicator_labels};
pub use model::*;
pub use projection::{project, State};

#[cfg(test)]
mod tests;
