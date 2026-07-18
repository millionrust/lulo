//! Event-driven application runtime for rmac Notes.
//!
//! This crate contains no GPUI or platform event loop. It owns generation,
//! debounce, background repository, and conflict state so the Notes view can
//! remain a renderer of accepted snapshots rather than a filesystem authority.

mod markdown_preview_worker;
mod preview_worker;
mod scheduler;
mod search;
mod search_worker;
mod session;
mod worker;

pub use markdown_preview_worker::{
    MarkdownPreviewRequest, MarkdownPreviewRequestError, MarkdownPreviewState,
    MarkdownPreviewWorkerEvent, MarkdownPreviewWorkerSendError, MarkdownPreviewWorkerStartError,
    NotesMarkdownPreviewSession, NotesMarkdownPreviewWorker, NotesMarkdownPreviewWorkerClient,
    NotesMarkdownPreviewWorkerEvents, MARKDOWN_PREVIEW_COMMAND_CAPACITY,
    MARKDOWN_PREVIEW_EVENT_CAPACITY,
};
pub use preview_worker::{
    NotesPreviewSession, NotesPreviewWorker, NotesPreviewWorkerClient, NotesPreviewWorkerEvents,
    PreviewCancellation, PreviewGeneration, PreviewRequest, PreviewRequestError, PreviewState,
    PreviewWorkerEvent, PreviewWorkerSendError, PreviewWorkerStartError, PREVIEW_COMMAND_CAPACITY,
    PREVIEW_EVENT_CAPACITY,
};
pub use scheduler::{
    EditGeneration, EditScheduler, ScheduleOutcome, ScheduledEdit, SchedulerError,
    DEFAULT_EDIT_DEBOUNCE, MAX_EDIT_DEBOUNCE,
};
pub use search::{
    NotesSearchIndex, NotesSearchSession, SearchBatch, SearchCancellation, SearchError,
    SearchField, SearchGeneration, SearchHit, SearchMatch, SearchRank, SearchRequest, SearchState,
    TextSpan, MAX_SEARCH_INDEX_TEXT_BYTES, MAX_SEARCH_MATCHES_PER_RESULT, MAX_SEARCH_QUERY_BYTES,
    MAX_SEARCH_RESULTS, SEARCH_INDEX_VERSION,
};
pub use search_worker::{
    NotesSearchWorker, NotesSearchWorkerClient, NotesSearchWorkerEvents, SearchJob,
    SearchWorkerEvent, SearchWorkerSendError, SearchWorkerStartError, SEARCH_COMMAND_CAPACITY,
    SEARCH_EVENT_CAPACITY,
};
pub use session::{FolderSelection, NotesSession, SessionPhase};
pub use worker::{
    AcceptedEvent, ActionRequest, ActionResult, BundleImportAcceptRequest,
    BundleImportReviewRequest, BundleImportReviewedEvent, DraftRecoveryKind, DraftRestoredEvent,
    DraftReviewSummary, DraftSummary, ExportRequest, ExportedEvent, LibraryAction,
    MarkdownImportReviewedEvent, MigrationReviewSummary, NotesWorker, NotesWorkerClient,
    NotesWorkerEvents, PendingEvent, RejectedEvent, SnapshotEvent, WorkerCommand, WorkerEvent,
    WorkerFailure, WorkerSendError, WorkerStartError, COMMAND_CAPACITY, EVENT_CAPACITY,
};
