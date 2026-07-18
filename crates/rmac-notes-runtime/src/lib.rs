//! Event-driven application runtime for rmac Notes.
//!
//! This crate contains no GPUI or platform event loop. It owns generation,
//! debounce, background repository, and conflict state so the Notes view can
//! remain a renderer of accepted snapshots rather than a filesystem authority.

mod scheduler;

pub use scheduler::{
    EditGeneration, EditScheduler, ScheduleOutcome, ScheduledEdit, SchedulerError,
    DEFAULT_EDIT_DEBOUNCE, MAX_EDIT_DEBOUNCE,
};
