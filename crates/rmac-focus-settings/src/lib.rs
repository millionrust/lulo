//! Framework-neutral, whole-configuration edits for the Focus Settings pane.

mod mode;
mod model;
mod schedule;

pub use mode::{set_allowed_app, set_mode_urgent};
pub use model::Error;
pub use schedule::{
    create_schedule, remove_schedule, set_schedule_day, set_schedule_enabled, set_schedule_end,
    set_schedule_start, upsert_schedule,
};

#[cfg(test)]
mod tests;
