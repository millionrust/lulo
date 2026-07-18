//! Event-driven application runtime for rmac Notes.
//!
//! This crate contains no GPUI or platform event loop. It owns generation,
//! debounce, background repository, and conflict state so the Notes view can
//! remain a renderer of accepted snapshots rather than a filesystem authority.

mod scheduler;
mod worker;

pub use scheduler::{
    EditGeneration, EditScheduler, ScheduleOutcome, ScheduledEdit, SchedulerError,
    DEFAULT_EDIT_DEBOUNCE, MAX_EDIT_DEBOUNCE,
};
pub use worker::{
    AcceptedEvent, ActionRequest, ActionResult, LibraryAction, MigrationReviewSummary, NotesWorker,
    PendingEvent, RejectedEvent, SnapshotEvent, WorkerCommand, WorkerEvent, WorkerFailure,
    WorkerSendError, WorkerStartError, COMMAND_CAPACITY, EVENT_CAPACITY,
};
