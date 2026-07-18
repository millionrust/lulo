use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{
    self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rmac_notes_storage::{
    inspect_notes_startup, AcceptedCommit, AcceptedLibrary, CommitError, DraftError, DraftRecord,
    DraftStore, ExportFailure, ExportFormat, ExportOutcome, ImportedTextEncoding, MigrationReview,
    MigrationWarning, NotesPaths, NotesStartup, PendingCommit, PendingReason,
    PreparedExportDestination, PreparedImageAttachment, RecoveryNotice, StartupError, StoreError,
    TextImportError,
};
use rmac_notes_store::{
    AttachmentId, ExportError, ExportScope, FolderId, LibrarySnapshot, MutationError, NewNote,
    NoteId, SortOrder,
};

use crate::{EditGeneration, EditScheduler, ScheduledEdit, SchedulerError, DEFAULT_EDIT_DEBOUNCE};

pub const COMMAND_CAPACITY: usize = 64;
pub const EVENT_CAPACITY: usize = 16;

pub enum LibraryAction {
    CreateNote(NewNote),
    CreateFolder {
        name: String,
    },
    RenameFolder {
        folder_id: FolderId,
        expected_revision: u64,
        name: String,
    },
    DeleteFolder {
        folder_id: FolderId,
        expected_revision: u64,
    },
    MoveNote {
        note_id: NoteId,
        expected_revision: u64,
        folder_id: Option<FolderId>,
    },
    SetPinned {
        note_id: NoteId,
        expected_revision: u64,
        pinned: bool,
    },
    SetSort(SortOrder),
    TrashNote {
        note_id: NoteId,
        expected_revision: u64,
    },
    RestoreNote {
        note_id: NoteId,
        expected_revision: u64,
    },
    AttachImage {
        note_id: NoteId,
        expected_revision: u64,
        modified_unix_ms: u64,
        selected_path: PathBuf,
    },
    ImportTextNote {
        created_unix_ms: u64,
        folder_id: Option<FolderId>,
        selected_path: PathBuf,
    },
    RemoveAttachmentReference {
        note_id: NoteId,
        expected_note_revision: u64,
        attachment_id: AttachmentId,
        expected_attachment_revision: u64,
        modified_unix_ms: u64,
    },
    CollectOrphanedAttachment {
        attachment_id: AttachmentId,
        expected_attachment_revision: u64,
    },
    DeleteNotePermanently {
        note_id: NoteId,
        expected_revision: u64,
    },
    EmptyTrash {
        expected_library_revision: u64,
    },
}

impl fmt::Debug for LibraryAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CreateNote(_) => "CreateNote([private])",
            Self::CreateFolder { .. } => "CreateFolder([private])",
            Self::RenameFolder { .. } => "RenameFolder([private])",
            Self::DeleteFolder { .. } => "DeleteFolder",
            Self::MoveNote { .. } => "MoveNote",
            Self::SetPinned { .. } => "SetPinned",
            Self::SetSort(_) => "SetSort",
            Self::TrashNote { .. } => "TrashNote",
            Self::RestoreNote { .. } => "RestoreNote",
            Self::AttachImage { .. } => "AttachImage([private source])",
            Self::ImportTextNote { .. } => "ImportTextNote([private source])",
            Self::RemoveAttachmentReference { .. } => "RemoveAttachmentReference",
            Self::CollectOrphanedAttachment { .. } => "CollectOrphanedAttachment",
            Self::DeleteNotePermanently { .. } => "DeleteNotePermanently",
            Self::EmptyTrash { .. } => "EmptyTrash",
        })
    }
}

pub struct ActionRequest {
    request_id: u64,
    action: LibraryAction,
}

impl ActionRequest {
    pub fn new(request_id: u64, action: LibraryAction) -> Result<Self, WorkerSendError> {
        if request_id == 0 {
            return Err(WorkerSendError::InvalidRequest);
        }
        Ok(Self { request_id, action })
    }

    pub fn request_id(&self) -> u64 {
        self.request_id
    }
}

impl fmt::Debug for ActionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActionRequest")
            .field("request_id", &self.request_id)
            .field("action", &self.action)
            .finish()
    }
}

pub struct ExportRequest {
    request_id: u64,
    scope: ExportScope,
    format: ExportFormat,
    selected_path: PathBuf,
}

impl ExportRequest {
    pub fn new(
        request_id: u64,
        scope: ExportScope,
        format: ExportFormat,
        selected_path: PathBuf,
    ) -> Result<Self, WorkerSendError> {
        if request_id == 0 {
            return Err(WorkerSendError::InvalidRequest);
        }
        Ok(Self {
            request_id,
            scope,
            format,
            selected_path,
        })
    }

    pub fn request_id(&self) -> u64 {
        self.request_id
    }
}

impl fmt::Debug for ExportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExportRequest")
            .field("request_id", &self.request_id)
            .field("scope", &self.scope)
            .field("format", &self.format)
            .field("selected_path", &"<redacted>")
            .finish()
    }
}

pub enum WorkerCommand {
    AcceptMigration { request_id: u64 },
    StartEmpty { request_id: u64 },
    ScheduleEdit(ScheduledEdit),
    Apply(ActionRequest),
    Export(ExportRequest),
    Flush { request_id: u64 },
    RetryPending,
    DiscardPending { request_id: u64 },
    RestoreDraft { request_id: u64, note_id: NoteId },
    DiscardDraft { request_id: u64, note_id: NoteId },
    Shutdown,
}

impl WorkerCommand {
    fn request_context(&self) -> (u64, Option<EditGeneration>) {
        match self {
            Self::AcceptMigration { request_id }
            | Self::StartEmpty { request_id }
            | Self::Flush { request_id }
            | Self::DiscardPending { request_id }
            | Self::RestoreDraft { request_id, .. }
            | Self::DiscardDraft { request_id, .. } => (*request_id, None),
            Self::ScheduleEdit(edit) => (edit.request_id(), Some(edit.generation())),
            Self::Apply(request) => (request.request_id(), None),
            Self::Export(request) => (request.request_id(), None),
            Self::RetryPending | Self::Shutdown => (0, None),
        }
    }
}

impl fmt::Debug for WorkerCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScheduleEdit(edit) => formatter.debug_tuple("ScheduleEdit").field(edit).finish(),
            Self::Apply(request) => formatter.debug_tuple("Apply").field(request).finish(),
            Self::Export(request) => formatter.debug_tuple("Export").field(request).finish(),
            Self::AcceptMigration { request_id } => formatter
                .debug_struct("AcceptMigration")
                .field("request_id", request_id)
                .finish(),
            Self::StartEmpty { request_id } => formatter
                .debug_struct("StartEmpty")
                .field("request_id", request_id)
                .finish(),
            Self::Flush { request_id } => formatter
                .debug_struct("Flush")
                .field("request_id", request_id)
                .finish(),
            Self::DiscardPending { request_id } => formatter
                .debug_struct("DiscardPending")
                .field("request_id", request_id)
                .finish(),
            Self::RestoreDraft {
                request_id,
                note_id,
            } => formatter
                .debug_struct("RestoreDraft")
                .field("request_id", request_id)
                .field("note_id", note_id)
                .finish(),
            Self::DiscardDraft {
                request_id,
                note_id,
            } => formatter
                .debug_struct("DiscardDraft")
                .field("request_id", request_id)
                .field("note_id", note_id)
                .finish(),
            Self::RetryPending => formatter.write_str("RetryPending"),
            Self::Shutdown => formatter.write_str("Shutdown"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationReviewSummary {
    pub revision: u64,
    pub folders: usize,
    pub notes: usize,
    pub managed_attachments: usize,
    pub recovery_files: usize,
    pub warnings: Vec<MigrationWarning>,
}

impl MigrationReviewSummary {
    fn from_review(review: &MigrationReview) -> Self {
        let plan = review.plan();
        Self {
            revision: plan.snapshot.revision,
            folders: plan.snapshot.folders.len(),
            notes: plan.snapshot.notes.len(),
            managed_attachments: plan.snapshot.attachments.len(),
            recovery_files: plan.recovery_files.len(),
            warnings: plan.warnings.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftRecoveryKind {
    Applicable,
    Conflict,
    Orphaned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraftSummary {
    pub note_id: NoteId,
    pub base_note_revision: u64,
    pub edit_generation: u64,
    pub updated_unix_ms: u64,
    pub kind: DraftRecoveryKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DraftReviewSummary {
    pub drafts: Vec<DraftSummary>,
    pub malformed: usize,
    pub quarantined: usize,
    pub cleanup_pending: usize,
    pub excessive: bool,
    pub unavailable: bool,
}

impl DraftReviewSummary {
    pub(crate) fn requires_attention(&self) -> bool {
        !self.drafts.is_empty()
            || self.malformed != 0
            || self.quarantined != 0
            || self.cleanup_pending != 0
            || self.excessive
            || self.unavailable
    }
}

#[derive(Clone)]
pub struct DraftRestoredEvent {
    pub request_id: u64,
    pub draft: DraftRecord,
}

impl fmt::Debug for DraftRestoredEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DraftRestoredEvent")
            .field("request_id", &self.request_id)
            .field("draft", &self.draft)
            .finish()
    }
}

#[derive(Clone)]
pub struct SnapshotEvent {
    pub request_id: Option<u64>,
    pub snapshot: Arc<LibrarySnapshot>,
    pub notices: Vec<RecoveryNotice>,
}

impl fmt::Debug for SnapshotEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotEvent")
            .field("request_id", &self.request_id)
            .field("revision", &self.snapshot.revision)
            .field("folders", &self.snapshot.folders.len())
            .field("notes", &self.snapshot.notes.len())
            .field("attachments", &self.snapshot.attachments.len())
            .field("notices", &self.notices)
            .finish()
    }
}

impl SnapshotEvent {
    fn from_library(library: &AcceptedLibrary, request_id: Option<u64>) -> Self {
        Self {
            request_id,
            snapshot: Arc::new(library.snapshot().clone()),
            notices: library.recovery_notices().to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionResult {
    Edited(NoteId),
    CreatedNote(NoteId),
    CreatedFolder(FolderId),
    Changed,
    DeletedFolder {
        moved_notes: usize,
    },
    RestoredNote {
        folder_id: Option<FolderId>,
    },
    AttachedImage {
        note_id: NoteId,
        attachment_id: rmac_notes_store::AttachmentId,
        width: u32,
        height: u32,
        byte_len: u64,
    },
    ImportedNote {
        note_id: NoteId,
        encoding: ImportedTextEncoding,
        source_byte_len: u64,
    },
    AttachmentReferenceRemoved {
        note_id: NoteId,
        attachment_id: AttachmentId,
    },
    OrphanCollectionAccepted {
        attachment_id: AttachmentId,
        byte_len: u64,
    },
    PermanentDeleteAccepted {
        note_id: NoteId,
        attachment_count: usize,
        attachment_bytes: u64,
    },
    EmptyTrashAccepted {
        note_count: usize,
        attachment_count: usize,
        attachment_bytes: u64,
    },
}

#[derive(Clone, Debug)]
pub struct AcceptedEvent {
    pub request_id: u64,
    pub generation: Option<EditGeneration>,
    pub result: ActionResult,
    pub commit: AcceptedCommit,
    pub accepted: SnapshotEvent,
    pub draft_cleanup_pending: bool,
}

#[derive(Clone, Debug)]
pub struct PendingEvent {
    pub request_id: u64,
    pub generation: Option<EditGeneration>,
    pub reason: PendingReason,
    pub accepted: SnapshotEvent,
    pub draft_error: Option<DraftError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerFailure {
    WrongPhase,
    CommitPending,
    Scheduler(SchedulerError),
    Mutation(MutationError),
    Storage(StoreError),
    TextImport(TextImportError),
    Draft(DraftError),
    MissingDraft,
    ExportPlan(ExportError),
    Export(ExportFailure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportedEvent {
    pub request_id: u64,
    pub outcome: ExportOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RejectedEvent {
    pub request_id: u64,
    pub generation: Option<EditGeneration>,
    pub failure: WorkerFailure,
}

pub enum WorkerEvent {
    MigrationReview(MigrationReviewSummary),
    Ready(SnapshotEvent),
    DraftReview(DraftReviewSummary),
    DraftRestored(DraftRestoredEvent),
    DraftDiscarded {
        request_id: u64,
        note_id: NoteId,
    },
    Coalesced {
        request_id: u64,
        generation: EditGeneration,
        replaced_generation: EditGeneration,
    },
    Accepted(AcceptedEvent),
    Exported(ExportedEvent),
    Pending(PendingEvent),
    Rejected(RejectedEvent),
    StartupFailed(StartupError),
    Stopped {
        deadline_wakeups: u64,
    },
}

impl fmt::Debug for WorkerEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MigrationReview(summary) => formatter
                .debug_tuple("MigrationReview")
                .field(summary)
                .finish(),
            Self::Ready(snapshot) => formatter.debug_tuple("Ready").field(snapshot).finish(),
            Self::DraftReview(summary) => {
                formatter.debug_tuple("DraftReview").field(summary).finish()
            }
            Self::DraftRestored(event) => {
                formatter.debug_tuple("DraftRestored").field(event).finish()
            }
            Self::DraftDiscarded {
                request_id,
                note_id,
            } => formatter
                .debug_struct("DraftDiscarded")
                .field("request_id", request_id)
                .field("note_id", note_id)
                .finish(),
            Self::Coalesced {
                request_id,
                generation,
                replaced_generation,
            } => formatter
                .debug_struct("Coalesced")
                .field("request_id", request_id)
                .field("generation", generation)
                .field("replaced_generation", replaced_generation)
                .finish(),
            Self::Accepted(event) => formatter.debug_tuple("Accepted").field(event).finish(),
            Self::Exported(event) => formatter.debug_tuple("Exported").field(event).finish(),
            Self::Pending(event) => formatter.debug_tuple("Pending").field(event).finish(),
            Self::Rejected(event) => formatter.debug_tuple("Rejected").field(event).finish(),
            Self::StartupFailed(error) => {
                formatter.debug_tuple("StartupFailed").field(error).finish()
            }
            Self::Stopped { deadline_wakeups } => formatter
                .debug_struct("Stopped")
                .field("deadline_wakeups", deadline_wakeups)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerSendError {
    InvalidRequest,
    Full,
    Closed,
}

impl fmt::Display for WorkerSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "the Notes runtime request is invalid",
            Self::Full => "the Notes runtime command queue is full",
            Self::Closed => "the Notes runtime is no longer available",
        })
    }
}

impl std::error::Error for WorkerSendError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerStartError {
    Scheduler(SchedulerError),
    Thread(io::ErrorKind),
}

impl fmt::Display for WorkerStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Scheduler(_) => "the Notes runtime debounce configuration is invalid",
            Self::Thread(_) => "Notes could not start its private storage worker",
        })
    }
}

impl std::error::Error for WorkerStartError {}

pub struct NotesWorker {
    commands: Option<SyncSender<WorkerCommand>>,
    events: Option<Receiver<WorkerEvent>>,
    thread: Option<JoinHandle<()>>,
}

/// Cloneable, nonblocking command endpoint retained by the UI thread.
#[derive(Clone)]
pub struct NotesWorkerClient {
    commands: SyncSender<WorkerCommand>,
}

impl NotesWorkerClient {
    pub fn try_send(&self, command: WorkerCommand) -> Result<(), WorkerSendError> {
        try_send_command(&self.commands, command)
    }
}

impl fmt::Debug for NotesWorkerClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("NotesWorkerClient").finish()
    }
}

/// Blocking event endpoint intended to live on one background task.
///
/// Dropping it disconnects event delivery, requests shutdown, and joins the
/// repository thread even if a cloned [`NotesWorkerClient`] still exists.
pub struct NotesWorkerEvents {
    events: Option<Receiver<WorkerEvent>>,
    shutdown: Option<SyncSender<WorkerCommand>>,
    thread: Option<JoinHandle<()>>,
}

impl NotesWorkerEvents {
    pub fn recv(&self) -> Result<WorkerEvent, RecvError> {
        self.events.as_ref().ok_or(RecvError)?.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<WorkerEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<WorkerEvent, TryRecvError> {
        self.events
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }
}

impl fmt::Debug for NotesWorkerEvents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("NotesWorkerEvents").finish()
    }
}

impl Drop for NotesWorkerEvents {
    fn drop(&mut self) {
        self.events.take();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.try_send(WorkerCommand::Shutdown);
            drop(shutdown);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl NotesWorker {
    pub fn start(paths: NotesPaths) -> Result<Self, WorkerStartError> {
        Self::start_with_debounce(paths, DEFAULT_EDIT_DEBOUNCE)
    }

    pub fn start_with_debounce(
        paths: NotesPaths,
        debounce: Duration,
    ) -> Result<Self, WorkerStartError> {
        let scheduler = EditScheduler::new(debounce).map_err(WorkerStartError::Scheduler)?;
        let (command_sender, command_receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (event_sender, event_receiver) = mpsc::sync_channel(EVENT_CAPACITY);
        let thread = thread::Builder::new()
            .name("rmac-notes-storage".into())
            .spawn(move || run_worker(paths, scheduler, command_receiver, event_sender))
            .map_err(|error| WorkerStartError::Thread(error.kind()))?;
        Ok(Self {
            commands: Some(command_sender),
            events: Some(event_receiver),
            thread: Some(thread),
        })
    }

    pub fn try_send(&self, command: WorkerCommand) -> Result<(), WorkerSendError> {
        try_send_command(
            self.commands.as_ref().ok_or(WorkerSendError::Closed)?,
            command,
        )
    }

    pub fn recv(&self) -> Result<WorkerEvent, RecvError> {
        self.events.as_ref().ok_or(RecvError)?.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<WorkerEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<WorkerEvent, TryRecvError> {
        self.events
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }

    /// Separates the UI command endpoint from the background event endpoint.
    pub fn into_parts(mut self) -> (NotesWorkerClient, NotesWorkerEvents) {
        let commands = self
            .commands
            .take()
            .expect("a live Notes worker always owns its command endpoint");
        let events = self
            .events
            .take()
            .expect("a live Notes worker always owns its event endpoint");
        let thread = self
            .thread
            .take()
            .expect("a live Notes worker always owns its repository thread");
        (
            NotesWorkerClient {
                commands: commands.clone(),
            },
            NotesWorkerEvents {
                events: Some(events),
                shutdown: Some(commands),
                thread: Some(thread),
            },
        )
    }
}

impl Drop for NotesWorker {
    fn drop(&mut self) {
        self.events.take();
        if let Some(commands) = self.commands.take() {
            let _ = commands.try_send(WorkerCommand::Shutdown);
            drop(commands);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn try_send_command(
    commands: &SyncSender<WorkerCommand>,
    command: WorkerCommand,
) -> Result<(), WorkerSendError> {
    let (request_id, _) = command.request_context();
    if !matches!(
        command,
        WorkerCommand::RetryPending | WorkerCommand::Shutdown
    ) && request_id == 0
    {
        return Err(WorkerSendError::InvalidRequest);
    }
    commands.try_send(command).map_err(|error| match error {
        TrySendError::Full(_) => WorkerSendError::Full,
        TrySendError::Disconnected(_) => WorkerSendError::Closed,
    })
}

struct ReadyState {
    library: Box<AcceptedLibrary>,
    scheduler: EditScheduler,
    drafts: DraftStore,
    recoverable_drafts: BTreeMap<NoteId, DraftRecord>,
}

impl ReadyState {
    fn new(
        library: Box<AcceptedLibrary>,
        scheduler: EditScheduler,
    ) -> (Self, Option<DraftReviewSummary>) {
        let drafts = DraftStore::for_library(library.root());
        let discovery = drafts.discover();
        let mut recoverable_drafts = BTreeMap::new();
        let mut summary = DraftReviewSummary {
            malformed: discovery.malformed,
            quarantined: discovery.quarantined,
            excessive: discovery.excessive,
            unavailable: discovery.unavailable,
            ..DraftReviewSummary::default()
        };
        for draft in discovery.drafts {
            let note = library
                .snapshot()
                .notes
                .iter()
                .find(|note| note.id == draft.note_id);
            if note.is_some_and(|note| draft_matches_note(&draft, note)) {
                if drafts.remove(draft.note_id).is_err() {
                    summary.cleanup_pending = summary.cleanup_pending.saturating_add(1);
                }
                continue;
            }
            let kind = match note {
                Some(note)
                    if !note.deleted
                        && note.revision == draft.base_note_revision
                        && draft.changes.modified_unix_ms >= note.created_unix_ms =>
                {
                    DraftRecoveryKind::Applicable
                }
                Some(note) if !note.deleted => DraftRecoveryKind::Conflict,
                Some(_) | None => DraftRecoveryKind::Orphaned,
            };
            summary.drafts.push(DraftSummary {
                note_id: draft.note_id,
                base_note_revision: draft.base_note_revision,
                edit_generation: draft.edit_generation,
                updated_unix_ms: draft.updated_unix_ms,
                kind,
            });
            recoverable_drafts.insert(draft.note_id, draft);
        }
        let review = summary.requires_attention().then_some(summary);
        (
            Self {
                library,
                scheduler,
                drafts,
                recoverable_drafts,
            },
            review,
        )
    }
}

#[derive(Clone, Copy)]
struct RequestContext {
    request_id: u64,
    generation: Option<EditGeneration>,
    result: ActionResult,
    draft: Option<DraftContext>,
}

#[derive(Clone, Copy)]
struct DraftContext {
    note_id: NoteId,
    persisted: bool,
    error: Option<DraftError>,
}

struct PendingState {
    ready: ReadyState,
    pending: PendingCommit,
    context: RequestContext,
}

struct ReviewState {
    review: Box<MigrationReview>,
    scheduler: EditScheduler,
}

enum Phase {
    Review(ReviewState),
    Ready(ReadyState),
    Pending(PendingState),
    Stopped,
}

fn run_worker(
    paths: NotesPaths,
    scheduler: EditScheduler,
    commands: Receiver<WorkerCommand>,
    events: SyncSender<WorkerEvent>,
) {
    let mut phase = match inspect_notes_startup(&paths) {
        Ok(NotesStartup::Ready(library)) => {
            let phase = enter_ready(library, scheduler, None, &events);
            if matches!(phase, Phase::Stopped) {
                return;
            }
            phase
        }
        Ok(NotesStartup::MigrationReview(review)) => {
            if events
                .send(WorkerEvent::MigrationReview(
                    MigrationReviewSummary::from_review(&review),
                ))
                .is_err()
            {
                return;
            }
            Phase::Review(ReviewState { review, scheduler })
        }
        Err(error) => {
            let _ = events.send(WorkerEvent::StartupFailed(error));
            return;
        }
    };
    let origin = Instant::now();
    let mut deadline_wakeups = 0_u64;
    loop {
        phase = process_due(phase, elapsed_millis(origin), &events);
        if matches!(phase, Phase::Stopped) {
            break;
        }
        let command = match wait_for_command(&commands, &phase, elapsed_millis(origin)) {
            Ok(Some(command)) => command,
            Ok(None) => {
                deadline_wakeups = deadline_wakeups.saturating_add(1);
                continue;
            }
            Err(()) => break,
        };
        phase = process_command(phase, command, elapsed_millis(origin), &events);
    }
    let _ = events.send(WorkerEvent::Stopped { deadline_wakeups });
}

fn elapsed_millis(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn enter_ready(
    library: Box<AcceptedLibrary>,
    scheduler: EditScheduler,
    request_id: Option<u64>,
    events: &SyncSender<WorkerEvent>,
) -> Phase {
    let (ready, draft_review) = ReadyState::new(library, scheduler);
    if !emit_ready(events, &ready.library, request_id) {
        return Phase::Stopped;
    }
    if let Some(summary) = draft_review {
        if events.send(WorkerEvent::DraftReview(summary)).is_err() {
            return Phase::Stopped;
        }
    }
    Phase::Ready(ready)
}

fn draft_matches_note(draft: &DraftRecord, note: &rmac_notes_store::NoteRecord) -> bool {
    note.title == draft.changes.title
        && note.body == draft.changes.body
        && note.tags == draft.changes.tags
        && note.modified_unix_ms == draft.changes.modified_unix_ms
}

fn wait_for_command(
    commands: &Receiver<WorkerCommand>,
    phase: &Phase,
    now_millis: u64,
) -> Result<Option<WorkerCommand>, ()> {
    let wait = match phase {
        Phase::Ready(ready) => ready.scheduler.next_wait(now_millis),
        _ => None,
    };
    match wait {
        Some(wait) => match commands.recv_timeout(wait) {
            Ok(command) => Ok(Some(command)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(()),
        },
        None => commands.recv().map(Some).map_err(|_| ()),
    }
}

fn process_due(phase: Phase, now_millis: u64, events: &SyncSender<WorkerEvent>) -> Phase {
    let Phase::Ready(mut ready) = phase else {
        return phase;
    };
    let Some(edit) = ready.scheduler.take_due(now_millis) else {
        return Phase::Ready(ready);
    };
    match commit_edit(&mut ready, edit, events) {
        CommitDisposition::Ready => Phase::Ready(ready),
        CommitDisposition::Pending(pending, context) => Phase::Pending(PendingState {
            ready,
            pending,
            context,
        }),
        CommitDisposition::Rejected => Phase::Ready(ready),
        CommitDisposition::Stopped => Phase::Stopped,
    }
}

fn process_command(
    phase: Phase,
    command: WorkerCommand,
    now_millis: u64,
    events: &SyncSender<WorkerEvent>,
) -> Phase {
    match (phase, command) {
        (_, WorkerCommand::Shutdown) => Phase::Stopped,
        (Phase::Review(review), WorkerCommand::AcceptMigration { request_id }) => {
            match review.review.accept() {
                Ok(library) => enter_ready(
                    Box::new(library),
                    review.scheduler,
                    Some(request_id),
                    events,
                ),
                Err(error) => {
                    let _ = events.send(WorkerEvent::StartupFailed(error));
                    Phase::Stopped
                }
            }
        }
        (Phase::Review(review), WorkerCommand::StartEmpty { request_id }) => {
            let library = Box::new(review.review.start_empty());
            enter_ready(library, review.scheduler, Some(request_id), events)
        }
        (Phase::Review(review), command) => {
            if emit_rejected(events, command, WorkerFailure::WrongPhase) {
                Phase::Review(review)
            } else {
                Phase::Stopped
            }
        }
        (Phase::Ready(mut ready), WorkerCommand::ScheduleEdit(edit)) => {
            let request_id = edit.request_id();
            let generation = edit.generation();
            match ready.scheduler.schedule(now_millis, edit) {
                Ok(outcome) => {
                    if let Some(replaced_generation) = outcome.replaced_generation {
                        if events
                            .send(WorkerEvent::Coalesced {
                                request_id,
                                generation,
                                replaced_generation,
                            })
                            .is_err()
                        {
                            return Phase::Stopped;
                        }
                    }
                    if let Some(displaced) = outcome.displaced {
                        return match commit_edit(&mut ready, displaced, events) {
                            CommitDisposition::Ready => Phase::Ready(ready),
                            CommitDisposition::Pending(pending, context) => {
                                Phase::Pending(PendingState {
                                    ready,
                                    pending,
                                    context,
                                })
                            }
                            // The displaced edit has already received its rejection event.
                            // Keep the newly selected note scheduled: rejecting one note must
                            // never discard the complete editor state for another note.
                            CommitDisposition::Rejected => Phase::Ready(ready),
                            CommitDisposition::Stopped => Phase::Stopped,
                        };
                    }
                    Phase::Ready(ready)
                }
                Err(error) => {
                    if events
                        .send(WorkerEvent::Rejected(RejectedEvent {
                            request_id,
                            generation: Some(generation),
                            failure: WorkerFailure::Scheduler(error),
                        }))
                        .is_ok()
                    {
                        Phase::Ready(ready)
                    } else {
                        Phase::Stopped
                    }
                }
            }
        }
        (Phase::Ready(mut ready), WorkerCommand::Apply(request)) => {
            if let Some(edit) = ready.scheduler.flush() {
                match commit_edit(&mut ready, edit, events) {
                    CommitDisposition::Ready => {}
                    CommitDisposition::Pending(pending, context) => {
                        if !emit_request_rejected(
                            events,
                            request.request_id,
                            None,
                            WorkerFailure::CommitPending,
                        ) {
                            return Phase::Stopped;
                        }
                        return Phase::Pending(PendingState {
                            ready,
                            pending,
                            context,
                        });
                    }
                    CommitDisposition::Rejected => {
                        if emit_request_rejected(
                            events,
                            request.request_id,
                            None,
                            WorkerFailure::CommitPending,
                        ) {
                            return Phase::Ready(ready);
                        }
                        return Phase::Stopped;
                    }
                    CommitDisposition::Stopped => return Phase::Stopped,
                }
            }
            match commit_action(&mut ready, request, events) {
                CommitDisposition::Ready => Phase::Ready(ready),
                CommitDisposition::Pending(pending, context) => Phase::Pending(PendingState {
                    ready,
                    pending,
                    context,
                }),
                CommitDisposition::Rejected => Phase::Ready(ready),
                CommitDisposition::Stopped => Phase::Stopped,
            }
        }
        (Phase::Ready(mut ready), WorkerCommand::Export(request)) => {
            if let Some(edit) = ready.scheduler.flush() {
                match commit_edit(&mut ready, edit, events) {
                    CommitDisposition::Ready => {}
                    CommitDisposition::Pending(pending, context) => {
                        if !emit_request_rejected(
                            events,
                            request.request_id,
                            None,
                            WorkerFailure::CommitPending,
                        ) {
                            return Phase::Stopped;
                        }
                        return Phase::Pending(PendingState {
                            ready,
                            pending,
                            context,
                        });
                    }
                    CommitDisposition::Rejected => {
                        if emit_request_rejected(
                            events,
                            request.request_id,
                            None,
                            WorkerFailure::CommitPending,
                        ) {
                            return Phase::Ready(ready);
                        }
                        return Phase::Stopped;
                    }
                    CommitDisposition::Stopped => return Phase::Stopped,
                }
            }
            let plan = match ready.library.snapshot().plan_export(request.scope) {
                Ok(plan) => plan,
                Err(error) => {
                    return if emit_request_rejected(
                        events,
                        request.request_id,
                        None,
                        WorkerFailure::ExportPlan(error),
                    ) {
                        Phase::Ready(ready)
                    } else {
                        Phase::Stopped
                    };
                }
            };
            let destination = match PreparedExportDestination::review(request.selected_path) {
                Ok(destination) => destination,
                Err(error) => {
                    return if emit_request_rejected(
                        events,
                        request.request_id,
                        None,
                        WorkerFailure::Export(error),
                    ) {
                        Phase::Ready(ready)
                    } else {
                        Phase::Stopped
                    };
                }
            };
            match ready.library.export(&plan, request.format, destination) {
                Ok(outcome) => {
                    if events
                        .send(WorkerEvent::Exported(ExportedEvent {
                            request_id: request.request_id,
                            outcome,
                        }))
                        .is_ok()
                    {
                        Phase::Ready(ready)
                    } else {
                        Phase::Stopped
                    }
                }
                Err(error) => {
                    if emit_request_rejected(
                        events,
                        request.request_id,
                        None,
                        WorkerFailure::Export(error),
                    ) {
                        Phase::Ready(ready)
                    } else {
                        Phase::Stopped
                    }
                }
            }
        }
        (Phase::Ready(mut ready), WorkerCommand::Flush { request_id }) => {
            if let Some(edit) = ready.scheduler.flush() {
                match commit_edit(&mut ready, edit, events) {
                    CommitDisposition::Ready => {
                        if emit_ready(events, &ready.library, Some(request_id)) {
                            Phase::Ready(ready)
                        } else {
                            Phase::Stopped
                        }
                    }
                    CommitDisposition::Pending(pending, context) => {
                        if !emit_request_rejected(
                            events,
                            request_id,
                            None,
                            WorkerFailure::CommitPending,
                        ) {
                            return Phase::Stopped;
                        }
                        Phase::Pending(PendingState {
                            ready,
                            pending,
                            context,
                        })
                    }
                    CommitDisposition::Rejected => {
                        if emit_request_rejected(
                            events,
                            request_id,
                            None,
                            WorkerFailure::CommitPending,
                        ) {
                            Phase::Ready(ready)
                        } else {
                            Phase::Stopped
                        }
                    }
                    CommitDisposition::Stopped => Phase::Stopped,
                }
            } else if emit_ready(events, &ready.library, Some(request_id)) {
                Phase::Ready(ready)
            } else {
                Phase::Stopped
            }
        }
        (
            Phase::Ready(ready),
            WorkerCommand::RestoreDraft {
                request_id,
                note_id,
            },
        ) => {
            let Some(draft) = ready.recoverable_drafts.get(&note_id).cloned() else {
                return if emit_request_rejected(
                    events,
                    request_id,
                    None,
                    WorkerFailure::MissingDraft,
                ) {
                    Phase::Ready(ready)
                } else {
                    Phase::Stopped
                };
            };
            if events
                .send(WorkerEvent::DraftRestored(DraftRestoredEvent {
                    request_id,
                    draft,
                }))
                .is_ok()
            {
                Phase::Ready(ready)
            } else {
                Phase::Stopped
            }
        }
        (
            Phase::Ready(mut ready),
            WorkerCommand::DiscardDraft {
                request_id,
                note_id,
            },
        ) => {
            if !ready.recoverable_drafts.contains_key(&note_id) {
                return if emit_request_rejected(
                    events,
                    request_id,
                    None,
                    WorkerFailure::MissingDraft,
                ) {
                    Phase::Ready(ready)
                } else {
                    Phase::Stopped
                };
            }
            if let Err(error) = ready.drafts.remove(note_id) {
                return if emit_request_rejected(
                    events,
                    request_id,
                    None,
                    WorkerFailure::Draft(error),
                ) {
                    Phase::Ready(ready)
                } else {
                    Phase::Stopped
                };
            }
            ready.recoverable_drafts.remove(&note_id);
            if events
                .send(WorkerEvent::DraftDiscarded {
                    request_id,
                    note_id,
                })
                .is_ok()
            {
                Phase::Ready(ready)
            } else {
                Phase::Stopped
            }
        }
        (Phase::Ready(ready), command) => {
            if emit_rejected(events, command, WorkerFailure::WrongPhase) {
                Phase::Ready(ready)
            } else {
                Phase::Stopped
            }
        }
        (Phase::Pending(mut pending), WorkerCommand::RetryPending) => {
            match pending.ready.library.retry(pending.pending) {
                Ok(commit) => {
                    let event = AcceptedEvent {
                        request_id: pending.context.request_id,
                        generation: pending.context.generation,
                        result: pending.context.result,
                        commit,
                        accepted: SnapshotEvent::from_library(
                            &pending.ready.library,
                            Some(pending.context.request_id),
                        ),
                        draft_cleanup_pending: cleanup_draft(
                            &mut pending.ready,
                            pending.context.draft,
                        ),
                    };
                    if events.send(WorkerEvent::Accepted(event)).is_ok() {
                        Phase::Ready(pending.ready)
                    } else {
                        Phase::Stopped
                    }
                }
                Err(still_pending) => {
                    pending.pending = still_pending;
                    if emit_pending(events, &pending) {
                        Phase::Pending(pending)
                    } else {
                        Phase::Stopped
                    }
                }
            }
        }
        (Phase::Pending(mut pending), WorkerCommand::DiscardPending { request_id }) => {
            if let Some(draft) = pending.context.draft.filter(|draft| draft.persisted) {
                if let Err(error) = pending.ready.drafts.remove(draft.note_id) {
                    return if emit_request_rejected(
                        events,
                        request_id,
                        None,
                        WorkerFailure::Draft(error),
                    ) {
                        Phase::Pending(pending)
                    } else {
                        Phase::Stopped
                    };
                }
                pending.ready.recoverable_drafts.remove(&draft.note_id);
                if events
                    .send(WorkerEvent::DraftDiscarded {
                        request_id,
                        note_id: draft.note_id,
                    })
                    .is_err()
                {
                    return Phase::Stopped;
                }
            }
            if emit_ready(events, &pending.ready.library, Some(request_id)) {
                Phase::Ready(pending.ready)
            } else {
                Phase::Stopped
            }
        }
        (Phase::Pending(pending), command) => {
            if emit_rejected(events, command, WorkerFailure::CommitPending) {
                Phase::Pending(pending)
            } else {
                Phase::Stopped
            }
        }
        (Phase::Stopped, _) => Phase::Stopped,
    }
}

enum CommitDisposition {
    Ready,
    Pending(PendingCommit, RequestContext),
    Rejected,
    Stopped,
}

fn commit_edit(
    ready: &mut ReadyState,
    edit: ScheduledEdit,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let request_id = edit.request_id();
    let generation = edit.generation();
    let note_id = edit.note_id();
    let Some(note) = ready
        .library
        .snapshot()
        .notes
        .iter()
        .find(|note| note.id == note_id)
    else {
        return reject_mutation(
            events,
            request_id,
            Some(generation),
            MutationError::MissingNote,
        );
    };
    if edit.expected_note_revision() > note.revision {
        return reject_mutation(
            events,
            request_id,
            Some(generation),
            MutationError::RevisionConflict,
        );
    }
    let current_revision = note.revision;
    let mut transaction = match ready.library.begin() {
        Ok(transaction) => transaction,
        Err(error) => return reject_mutation(events, request_id, Some(generation), error),
    };
    let changes = edit.changes().clone();
    if let Err(error) = transaction.edit_note(note_id, current_revision, changes.clone()) {
        return reject_mutation(events, request_id, Some(generation), error);
    }
    let record = DraftRecord {
        note_id,
        base_note_revision: current_revision,
        edit_generation: generation.get(),
        updated_unix_ms: changes.modified_unix_ms,
        changes,
    };
    let draft = match ready.drafts.save(&record) {
        Ok(()) => {
            ready.recoverable_drafts.insert(note_id, record);
            DraftContext {
                note_id,
                persisted: true,
                error: None,
            }
        }
        Err(error) => DraftContext {
            note_id,
            persisted: false,
            error: Some(error),
        },
    };
    let context = RequestContext {
        request_id,
        generation: Some(generation),
        result: ActionResult::Edited(note_id),
        draft: Some(draft),
    };
    commit_transaction(
        ready,
        transaction,
        TransactionCommit::Ordinary,
        context,
        events,
    )
}

fn commit_action(
    ready: &mut ReadyState,
    request: ActionRequest,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let request_id = request.request_id;
    let action = match request.action {
        LibraryAction::AttachImage {
            note_id,
            expected_revision,
            modified_unix_ms,
            selected_path,
        } => {
            return commit_attachment_action(
                ready,
                request_id,
                note_id,
                expected_revision,
                modified_unix_ms,
                selected_path,
                events,
            );
        }
        LibraryAction::ImportTextNote {
            created_unix_ms,
            folder_id,
            selected_path,
        } => {
            return commit_text_import_action(
                ready,
                request_id,
                created_unix_ms,
                folder_id,
                selected_path,
                events,
            );
        }
        action => action,
    };
    let mut transaction = match ready.library.begin() {
        Ok(transaction) => transaction,
        Err(error) => return reject_mutation(events, request_id, None, error),
    };
    let mut purge = None;
    let mut orphan_collection = None;
    let result = match action {
        LibraryAction::CreateNote(note) => {
            transaction.create_note(note).map(ActionResult::CreatedNote)
        }
        LibraryAction::CreateFolder { name } => transaction
            .create_folder(name)
            .map(ActionResult::CreatedFolder),
        LibraryAction::RenameFolder {
            folder_id,
            expected_revision,
            name,
        } => transaction
            .rename_folder(folder_id, expected_revision, name)
            .map(|_| ActionResult::Changed),
        LibraryAction::DeleteFolder {
            folder_id,
            expected_revision,
        } => transaction
            .delete_folder(folder_id, expected_revision)
            .map(|moved_notes| ActionResult::DeletedFolder { moved_notes }),
        LibraryAction::MoveNote {
            note_id,
            expected_revision,
            folder_id,
        } => transaction
            .move_note(note_id, expected_revision, folder_id)
            .map(|_| ActionResult::Changed),
        LibraryAction::SetPinned {
            note_id,
            expected_revision,
            pinned,
        } => transaction
            .set_note_pinned(note_id, expected_revision, pinned)
            .map(|_| ActionResult::Changed),
        LibraryAction::SetSort(sort_order) => {
            transaction.set_sort_order(sort_order);
            Ok(ActionResult::Changed)
        }
        LibraryAction::TrashNote {
            note_id,
            expected_revision,
        } => transaction
            .trash_note(note_id, expected_revision)
            .map(|()| ActionResult::Changed),
        LibraryAction::RestoreNote {
            note_id,
            expected_revision,
        } => transaction
            .restore_note(note_id, expected_revision)
            .map(|folder_id| ActionResult::RestoredNote { folder_id }),
        LibraryAction::RemoveAttachmentReference {
            note_id,
            expected_note_revision,
            attachment_id,
            expected_attachment_revision,
            modified_unix_ms,
        } => transaction
            .remove_attachment_reference(
                note_id,
                expected_note_revision,
                attachment_id,
                expected_attachment_revision,
                modified_unix_ms,
            )
            .map(|()| ActionResult::AttachmentReferenceRemoved {
                note_id,
                attachment_id,
            }),
        LibraryAction::CollectOrphanedAttachment {
            attachment_id,
            expected_attachment_revision,
        } => transaction
            .collect_orphaned_attachment(attachment_id, expected_attachment_revision)
            .map(|plan| {
                let result = ActionResult::OrphanCollectionAccepted {
                    attachment_id,
                    byte_len: plan.byte_len,
                };
                orphan_collection = Some(plan);
                result
            }),
        LibraryAction::DeleteNotePermanently {
            note_id,
            expected_revision,
        } => transaction
            .purge_trashed_note(note_id, expected_revision)
            .map(|plan| {
                let result = ActionResult::PermanentDeleteAccepted {
                    note_id,
                    attachment_count: plan.attachment_ids.len(),
                    attachment_bytes: plan.attachment_bytes,
                };
                purge = Some(plan);
                result
            }),
        LibraryAction::EmptyTrash {
            expected_library_revision,
        } => transaction
            .empty_trash(expected_library_revision)
            .map(|plan| {
                let result = ActionResult::EmptyTrashAccepted {
                    note_count: plan.note_ids.len(),
                    attachment_count: plan.attachment_ids.len(),
                    attachment_bytes: plan.attachment_bytes,
                };
                purge = Some(plan);
                result
            }),
        LibraryAction::AttachImage { .. } => unreachable!("attachment action handled above"),
        LibraryAction::ImportTextNote { .. } => unreachable!("text import handled above"),
    };
    let result = match result {
        Ok(result) => result,
        Err(error) => return reject_mutation(events, request_id, None, error),
    };
    let context = RequestContext {
        request_id,
        generation: None,
        result,
        draft: None,
    };
    let commit = orphan_collection.map_or_else(
        || purge.map_or(TransactionCommit::Ordinary, TransactionCommit::Purge),
        TransactionCommit::OrphanCollection,
    );
    commit_transaction(ready, transaction, commit, context, events)
}

fn commit_text_import_action(
    ready: &mut ReadyState,
    request_id: u64,
    created_unix_ms: u64,
    folder_id: Option<FolderId>,
    selected_path: PathBuf,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let prepared = match ready.library.prepare_text_note(&selected_path) {
        Ok(prepared) => prepared,
        Err(error) => return reject_text_import(events, request_id, error),
    };
    let mut transaction = match ready.library.begin() {
        Ok(transaction) => transaction,
        Err(error) => return reject_mutation(events, request_id, None, error),
    };
    let note_id = match transaction.create_note(prepared.new_note(created_unix_ms, folder_id)) {
        Ok(note_id) => note_id,
        Err(error) => return reject_mutation(events, request_id, None, error),
    };
    let context = RequestContext {
        request_id,
        generation: None,
        result: ActionResult::ImportedNote {
            note_id,
            encoding: prepared.encoding(),
            source_byte_len: prepared.source_byte_len(),
        },
        draft: None,
    };
    commit_transaction(
        ready,
        transaction,
        TransactionCommit::Ordinary,
        context,
        events,
    )
}

#[allow(clippy::too_many_arguments)]
fn commit_attachment_action(
    ready: &mut ReadyState,
    request_id: u64,
    note_id: NoteId,
    expected_revision: u64,
    modified_unix_ms: u64,
    selected_path: PathBuf,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let prepared = match ready.library.prepare_image_attachment(&selected_path) {
        Ok(prepared) => prepared,
        Err(error) => return reject_storage(events, request_id, error),
    };
    let mut transaction = match ready.library.begin() {
        Ok(transaction) => transaction,
        Err(error) => return reject_mutation(events, request_id, None, error),
    };
    let plan = match transaction.add_attachment(
        note_id,
        expected_revision,
        modified_unix_ms,
        prepared.metadata(),
    ) {
        Ok(plan) => plan,
        Err(error) => return reject_mutation(events, request_id, None, error),
    };
    let result = ActionResult::AttachedImage {
        note_id,
        attachment_id: plan.attachment_id,
        width: prepared.width(),
        height: prepared.height(),
        byte_len: prepared.byte_len(),
    };
    let context = RequestContext {
        request_id,
        generation: None,
        result,
        draft: None,
    };
    commit_transaction(
        ready,
        transaction,
        TransactionCommit::AttachmentImport { plan, prepared },
        context,
        events,
    )
}

enum TransactionCommit {
    Ordinary,
    Purge(rmac_notes_store::PurgePlan),
    OrphanCollection(rmac_notes_store::OrphanCollectionPlan),
    AttachmentImport {
        plan: rmac_notes_store::AttachmentImportPlan,
        prepared: PreparedImageAttachment,
    },
}

fn commit_transaction(
    ready: &mut ReadyState,
    transaction: rmac_notes_store::LibraryTransaction,
    transaction_commit: TransactionCommit,
    context: RequestContext,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let outcome = match transaction_commit {
        TransactionCommit::Ordinary => ready.library.commit(transaction),
        TransactionCommit::Purge(plan) => ready.library.commit_purge(transaction, plan),
        TransactionCommit::OrphanCollection(plan) => {
            ready.library.commit_orphan_collection(transaction, plan)
        }
        TransactionCommit::AttachmentImport { plan, prepared } => ready
            .library
            .commit_attachment_import(transaction, plan, prepared),
    };
    match outcome {
        Ok(commit) => {
            let event = AcceptedEvent {
                request_id: context.request_id,
                generation: context.generation,
                result: context.result,
                commit,
                accepted: SnapshotEvent::from_library(&ready.library, Some(context.request_id)),
                draft_cleanup_pending: cleanup_draft(ready, context.draft),
            };
            if events.send(WorkerEvent::Accepted(event)).is_ok() {
                CommitDisposition::Ready
            } else {
                CommitDisposition::Stopped
            }
        }
        Err(CommitError::Mutation(error)) => {
            reject_mutation(events, context.request_id, context.generation, error)
        }
        Err(CommitError::Pending(pending)) => {
            let event = PendingEvent {
                request_id: context.request_id,
                generation: context.generation,
                reason: pending.reason,
                accepted: SnapshotEvent::from_library(&ready.library, Some(context.request_id)),
                draft_error: context.draft.and_then(|draft| draft.error),
            };
            if events.send(WorkerEvent::Pending(event)).is_ok() {
                CommitDisposition::Pending(pending, context)
            } else {
                CommitDisposition::Stopped
            }
        }
    }
}

fn reject_mutation(
    events: &SyncSender<WorkerEvent>,
    request_id: u64,
    generation: Option<EditGeneration>,
    error: MutationError,
) -> CommitDisposition {
    if events
        .send(WorkerEvent::Rejected(RejectedEvent {
            request_id,
            generation,
            failure: WorkerFailure::Mutation(error),
        }))
        .is_ok()
    {
        CommitDisposition::Rejected
    } else {
        CommitDisposition::Stopped
    }
}

fn reject_storage(
    events: &SyncSender<WorkerEvent>,
    request_id: u64,
    error: StoreError,
) -> CommitDisposition {
    if events
        .send(WorkerEvent::Rejected(RejectedEvent {
            request_id,
            generation: None,
            failure: WorkerFailure::Storage(error),
        }))
        .is_ok()
    {
        CommitDisposition::Rejected
    } else {
        CommitDisposition::Stopped
    }
}

fn reject_text_import(
    events: &SyncSender<WorkerEvent>,
    request_id: u64,
    error: TextImportError,
) -> CommitDisposition {
    if events
        .send(WorkerEvent::Rejected(RejectedEvent {
            request_id,
            generation: None,
            failure: WorkerFailure::TextImport(error),
        }))
        .is_ok()
    {
        CommitDisposition::Rejected
    } else {
        CommitDisposition::Stopped
    }
}

fn emit_ready(
    events: &SyncSender<WorkerEvent>,
    library: &AcceptedLibrary,
    request_id: Option<u64>,
) -> bool {
    events
        .send(WorkerEvent::Ready(SnapshotEvent::from_library(
            library, request_id,
        )))
        .is_ok()
}

fn emit_pending(events: &SyncSender<WorkerEvent>, pending: &PendingState) -> bool {
    events
        .send(WorkerEvent::Pending(PendingEvent {
            request_id: pending.context.request_id,
            generation: pending.context.generation,
            reason: pending.pending.reason,
            accepted: SnapshotEvent::from_library(
                &pending.ready.library,
                Some(pending.context.request_id),
            ),
            draft_error: pending.context.draft.and_then(|draft| draft.error),
        }))
        .is_ok()
}

fn cleanup_draft(ready: &mut ReadyState, draft: Option<DraftContext>) -> bool {
    let Some(draft) = draft.filter(|draft| draft.persisted) else {
        return false;
    };
    if ready.drafts.remove(draft.note_id).is_err() {
        return true;
    }
    ready.recoverable_drafts.remove(&draft.note_id);
    false
}

fn emit_rejected(
    events: &SyncSender<WorkerEvent>,
    command: WorkerCommand,
    failure: WorkerFailure,
) -> bool {
    let (request_id, generation) = command.request_context();
    events
        .send(WorkerEvent::Rejected(RejectedEvent {
            request_id,
            generation,
            failure,
        }))
        .is_ok()
}

fn emit_request_rejected(
    events: &SyncSender<WorkerEvent>,
    request_id: u64,
    generation: Option<EditGeneration>,
    failure: WorkerFailure,
) -> bool {
    events
        .send(WorkerEvent::Rejected(RejectedEvent {
            request_id,
            generation,
            failure,
        }))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder as _;
    use rmac_notes_storage::{ErrorKind, PendingReason};
    use rmac_notes_store::{encode, LibraryTransaction, NewNote, NoteChanges};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn roots(label: &str) -> (PathBuf, NotesPaths) {
        let container = std::env::temp_dir().join(format!(
            "rmac-notes-worker-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let paths = NotesPaths::new(container.join("data"), container.join("legacy")).unwrap();
        (container, paths)
    }

    fn ready(worker: &NotesWorker) -> SnapshotEvent {
        match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Ready(snapshot) => snapshot,
            event => panic!("expected ready event, got {event:?}"),
        }
    }

    fn create_note(worker: &NotesWorker, request_id: u64) -> (NoteId, SnapshotEvent) {
        worker
            .try_send(WorkerCommand::Apply(
                ActionRequest::new(
                    request_id,
                    LibraryAction::CreateNote(NewNote {
                        created_unix_ms: 10,
                        title: "Private title".into(),
                        body: "Initial body".into(),
                        tags: Vec::new(),
                        folder_id: None,
                    }),
                )
                .unwrap(),
            ))
            .unwrap();
        match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Accepted(event) => match event.result {
                ActionResult::CreatedNote(note_id) => (note_id, event.accepted),
                result => panic!("unexpected action result {result:?}"),
            },
            event => panic!("expected accepted create, got {event:?}"),
        }
    }

    fn apply_action(worker: &NotesWorker, request_id: u64, action: LibraryAction) -> WorkerEvent {
        worker
            .try_send(WorkerCommand::Apply(
                ActionRequest::new(request_id, action).unwrap(),
            ))
            .unwrap();
        worker.recv_timeout(Duration::from_secs(2)).unwrap()
    }

    fn scheduled_edit(
        request_id: u64,
        generation: u64,
        note_id: NoteId,
        expected_revision: u64,
        body: &str,
    ) -> ScheduledEdit {
        ScheduledEdit::new(
            request_id,
            EditGeneration::new(generation).unwrap(),
            note_id,
            expected_revision,
            NoteChanges {
                modified_unix_ms: 10 + generation,
                title: "Private title".into(),
                body: body.into(),
                tags: Vec::new(),
            },
        )
        .unwrap()
    }

    fn tiny_png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[12, 34, 56, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    fn utf16_be(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xfe, 0xff];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn worker_commits_actions_and_due_edits_without_idle_events() {
        let (container, paths) = roots("edit");
        let worker =
            NotesWorker::start_with_debounce(paths.clone(), Duration::from_millis(30)).unwrap();
        assert_eq!(ready(&worker).snapshot.revision, 1);
        let (note_id, created) = create_note(&worker, 1);
        assert_eq!(created.snapshot.notes[0].revision, 1);

        worker
            .try_send(WorkerCommand::ScheduleEdit(scheduled_edit(
                2,
                1,
                note_id,
                1,
                "Newest body",
            )))
            .unwrap();
        assert!(matches!(
            worker.recv_timeout(Duration::from_millis(10)),
            Err(RecvTimeoutError::Timeout)
        ));
        let accepted = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected accepted edit, got {event:?}"),
        };
        assert_eq!(accepted.request_id, 2);
        assert_eq!(accepted.generation, EditGeneration::new(1));
        assert_eq!(accepted.accepted.snapshot.notes[0].body, "Newest body");
        assert!(!accepted.draft_cleanup_pending);
        assert_eq!(
            DraftStore::for_library(paths.data_root())
                .load(note_id)
                .unwrap(),
            None
        );

        assert!(matches!(
            worker.recv_timeout(Duration::from_millis(80)),
            Err(RecvTimeoutError::Timeout)
        ));
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Stopped { deadline_wakeups } => assert_eq!(deadline_wakeups, 1),
            event => panic!("expected stopped event, got {event:?}"),
        }
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn worker_imports_content_validated_image_without_exposing_source_path() {
        let (container, paths) = roots("attachment");
        std::fs::create_dir_all(&container).unwrap();
        let source = container.join("private-source.untrusted-extension");
        std::fs::write(&source, tiny_png()).unwrap();
        let worker = NotesWorker::start(paths.clone()).unwrap();
        ready(&worker);
        let (note_id, _) = create_note(&worker, 1);
        let action = LibraryAction::AttachImage {
            note_id,
            expected_revision: 1,
            modified_unix_ms: 11,
            selected_path: source.clone(),
        };
        let debug = format!("{action:?}");
        assert!(!debug.contains("private-source"));
        assert!(!debug.contains(container.to_string_lossy().as_ref()));

        let accepted = match apply_action(&worker, 2, action) {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected accepted image import, got {event:?}"),
        };
        let attachment_id = match accepted.result {
            ActionResult::AttachedImage {
                note_id: accepted_note_id,
                attachment_id,
                width,
                height,
                byte_len,
            } => {
                assert_eq!(accepted_note_id, note_id);
                assert_eq!((width, height), (1, 1));
                assert_eq!(byte_len, tiny_png().len() as u64);
                attachment_id
            }
            result => panic!("unexpected action result {result:?}"),
        };
        assert!(!accepted.commit.maintenance_pending);
        assert!(!accepted.commit.attachment_import_pending);
        assert_eq!(accepted.accepted.snapshot.attachments.len(), 1);
        assert_eq!(
            accepted.accepted.snapshot.attachments[0].display_name,
            "private-source.png"
        );
        let managed_path = paths
            .data_root()
            .join("attachments")
            .join(format!("{:020}.bin", attachment_id.get()));
        assert_eq!(std::fs::read(&managed_path).unwrap(), tiny_png());

        let removed = match apply_action(
            &worker,
            3,
            LibraryAction::RemoveAttachmentReference {
                note_id,
                expected_note_revision: 2,
                attachment_id,
                expected_attachment_revision: 1,
                modified_unix_ms: 12,
            },
        ) {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected accepted reference removal, got {event:?}"),
        };
        assert_eq!(
            removed.result,
            ActionResult::AttachmentReferenceRemoved {
                note_id,
                attachment_id,
            }
        );
        assert!(removed.accepted.snapshot.notes[0].attachments.is_empty());
        assert!(removed.accepted.snapshot.attachments[0].deleted);
        assert_eq!(removed.accepted.snapshot.attachments[0].revision, 2);
        assert_eq!(std::fs::read(&managed_path).unwrap(), tiny_png());

        assert!(matches!(
            apply_action(
                &worker,
                4,
                LibraryAction::CollectOrphanedAttachment {
                    attachment_id,
                    expected_attachment_revision: 1,
                },
            ),
            WorkerEvent::Rejected(RejectedEvent {
                failure: WorkerFailure::Mutation(MutationError::RevisionConflict),
                ..
            })
        ));
        let collected = match apply_action(
            &worker,
            5,
            LibraryAction::CollectOrphanedAttachment {
                attachment_id,
                expected_attachment_revision: 2,
            },
        ) {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected accepted orphan collection, got {event:?}"),
        };
        assert_eq!(
            collected.result,
            ActionResult::OrphanCollectionAccepted {
                attachment_id,
                byte_len: tiny_png().len() as u64,
            }
        );
        assert!(!collected.commit.orphan_collection_pending);
        assert!(collected.accepted.snapshot.attachments.is_empty());
        assert!(!managed_path.exists());

        let invalid = container.join("private-invalid.png");
        std::fs::write(&invalid, b"not an image").unwrap();
        match apply_action(
            &worker,
            6,
            LibraryAction::AttachImage {
                note_id,
                expected_revision: 3,
                modified_unix_ms: 13,
                selected_path: invalid,
            },
        ) {
            WorkerEvent::Rejected(RejectedEvent {
                request_id: 6,
                failure: WorkerFailure::Storage(error),
                ..
            }) => assert_eq!(error.kind, ErrorKind::UnsupportedAttachment),
            event => panic!("expected rejected invalid image, got {event:?}"),
        }

        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn worker_exports_exact_accepted_note_to_a_redacted_destination() {
        let (container, paths) = roots("export");
        let worker = NotesWorker::start(paths).unwrap();
        ready(&worker);
        let (note_id, created) = create_note(&worker, 1);
        let destination = container.join("private-export-name.md");
        let request = ExportRequest::new(
            2,
            ExportScope::Note {
                note_id,
                expected_note_revision: created.snapshot.notes[0].revision,
            },
            ExportFormat::Markdown,
            destination.clone(),
        )
        .unwrap();
        let debug = format!("{request:?}");
        assert!(!debug.contains("private-export-name"));
        assert!(!debug.contains(container.to_string_lossy().as_ref()));

        worker.try_send(WorkerCommand::Export(request)).unwrap();
        let exported = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Exported(event) => event,
            event => panic!("expected exported event, got {event:?}"),
        };
        assert_eq!(exported.request_id, 2);
        assert_eq!(exported.outcome.format, ExportFormat::Markdown);
        assert_eq!(exported.outcome.library_revision, created.snapshot.revision);
        assert_eq!(exported.outcome.note_count, 1);
        assert_eq!(exported.outcome.attachment_count, 0);
        let markdown = std::fs::read_to_string(&destination).unwrap();
        assert!(markdown.ends_with("# Private title\n\nInitial body"));
        assert!(!format!("{exported:?}").contains("Private title"));

        let stale = ExportRequest::new(
            3,
            ExportScope::Note {
                note_id,
                expected_note_revision: 2,
            },
            ExportFormat::Markdown,
            container.join("stale.md"),
        )
        .unwrap();
        worker.try_send(WorkerCommand::Export(stale)).unwrap();
        assert!(matches!(
            worker.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::Rejected(RejectedEvent {
                request_id: 3,
                failure: WorkerFailure::ExportPlan(ExportError::RevisionConflict),
                ..
            })
        ));

        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn worker_imports_strict_text_into_an_accepted_stable_note() {
        let (container, paths) = roots("text-import");
        std::fs::create_dir_all(&container).unwrap();
        let source = container.join("Imported Plan.md");
        let body = "First line\r\nनमस्ते 🦀\r\n";
        let source_bytes = utf16_be(body);
        std::fs::write(&source, &source_bytes).unwrap();
        let worker = NotesWorker::start(paths).unwrap();
        ready(&worker);
        let folder_id = match apply_action(
            &worker,
            1,
            LibraryAction::CreateFolder {
                name: "Imports".into(),
            },
        ) {
            WorkerEvent::Accepted(AcceptedEvent {
                result: ActionResult::CreatedFolder(folder_id),
                ..
            }) => folder_id,
            event => panic!("expected accepted folder, got {event:?}"),
        };
        let action = LibraryAction::ImportTextNote {
            created_unix_ms: 20,
            folder_id: Some(folder_id),
            selected_path: source.clone(),
        };
        let debug = format!("{action:?}");
        assert!(!debug.contains("Imported Plan"));
        assert!(!debug.contains(container.to_string_lossy().as_ref()));

        let accepted = match apply_action(&worker, 2, action) {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected accepted text import, got {event:?}"),
        };
        let note_id = match accepted.result {
            ActionResult::ImportedNote {
                note_id,
                encoding: ImportedTextEncoding::Utf16Be,
                source_byte_len,
            } => {
                assert_eq!(source_byte_len, source_bytes.len() as u64);
                note_id
            }
            result => panic!("unexpected text import result {result:?}"),
        };
        let imported = accepted
            .accepted
            .snapshot
            .notes
            .iter()
            .find(|note| note.id == note_id)
            .unwrap();
        assert_eq!(imported.title, "Imported Plan");
        assert_eq!(imported.body, body);
        assert_eq!(imported.folder_id, Some(folder_id));

        let invalid = container.join("invalid.txt");
        std::fs::write(&invalid, [0xff]).unwrap();
        match apply_action(
            &worker,
            3,
            LibraryAction::ImportTextNote {
                created_unix_ms: 21,
                folder_id: None,
                selected_path: invalid,
            },
        ) {
            WorkerEvent::Rejected(RejectedEvent {
                request_id: 3,
                failure: WorkerFailure::TextImport(TextImportError::InvalidUtf8),
                ..
            }) => {}
            event => panic!("expected rejected invalid text, got {event:?}"),
        }

        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn worker_coalesces_same_note_and_persists_only_the_latest_generation() {
        let (container, paths) = roots("coalesce");
        let worker = NotesWorker::start_with_debounce(paths, Duration::from_millis(300)).unwrap();
        ready(&worker);
        let (note_id, _) = create_note(&worker, 1);

        worker
            .try_send(WorkerCommand::ScheduleEdit(scheduled_edit(
                2,
                1,
                note_id,
                1,
                "Older body",
            )))
            .unwrap();
        worker
            .try_send(WorkerCommand::ScheduleEdit(scheduled_edit(
                3,
                2,
                note_id,
                1,
                "Latest body",
            )))
            .unwrap();

        match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Coalesced {
                request_id,
                generation,
                replaced_generation,
            } => {
                assert_eq!(request_id, 3);
                assert_eq!(generation, EditGeneration::new(2).unwrap());
                assert_eq!(replaced_generation, EditGeneration::new(1).unwrap());
            }
            event => panic!("expected coalesced event, got {event:?}"),
        }
        let accepted = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected latest accepted edit, got {event:?}"),
        };
        assert_eq!(accepted.request_id, 3);
        assert_eq!(accepted.accepted.snapshot.notes[0].body, "Latest body");
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn rejected_displaced_edit_does_not_discard_the_new_note_edit() {
        let (container, paths) = roots("displaced-rejection");
        let worker = NotesWorker::start_with_debounce(paths, Duration::from_millis(30)).unwrap();
        ready(&worker);
        let (note_id, _) = create_note(&worker, 1);

        worker
            .try_send(WorkerCommand::ScheduleEdit(scheduled_edit(
                2,
                1,
                NoteId::new(u64::MAX).unwrap(),
                1,
                "Rejected note body",
            )))
            .unwrap();
        worker
            .try_send(WorkerCommand::ScheduleEdit(scheduled_edit(
                3,
                2,
                note_id,
                1,
                "Preserved note body",
            )))
            .unwrap();

        match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Rejected(RejectedEvent {
                request_id: 2,
                generation: Some(generation),
                failure: WorkerFailure::Mutation(MutationError::MissingNote),
            }) => assert_eq!(generation, EditGeneration::new(1).unwrap()),
            event => panic!("expected displaced edit rejection, got {event:?}"),
        }
        let accepted = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected preserved edit acceptance, got {event:?}"),
        };
        assert_eq!(accepted.request_id, 3);
        assert_eq!(accepted.generation, EditGeneration::new(2));
        assert_eq!(
            accepted.accepted.snapshot.notes[0].body,
            "Preserved note body"
        );

        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn worker_holds_migration_review_until_an_explicit_decision() {
        let (container, paths) = roots("migration");
        std::fs::create_dir_all(paths.legacy_root()).unwrap();
        std::fs::write(paths.legacy_root().join("legacy.md"), b"Legacy\nBody").unwrap();
        let worker = NotesWorker::start_with_debounce(paths, Duration::from_millis(20)).unwrap();

        match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::MigrationReview(summary) => assert_eq!(summary.notes, 1),
            event => panic!("expected migration review, got {event:?}"),
        }
        worker
            .try_send(WorkerCommand::AcceptMigration { request_id: 1 })
            .unwrap();
        let accepted = ready(&worker);
        assert_eq!(accepted.request_id, Some(1));
        assert_eq!(accepted.snapshot.notes[0].title, "Legacy");
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn startup_reviews_restores_and_discards_recoverable_drafts_explicitly() {
        let (container, paths) = roots("draft-review");
        let worker = NotesWorker::start(paths.clone()).unwrap();
        ready(&worker);
        let (note_id, _) = create_note(&worker, 1);
        let (conflict_id, _) = create_note(&worker, 2);
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);

        let drafts = DraftStore::for_library(paths.data_root());
        drafts
            .save(
                &DraftRecord::new(
                    note_id,
                    1,
                    7,
                    99,
                    NoteChanges {
                        modified_unix_ms: 99,
                        title: "Recovered title".into(),
                        body: "Recovered private body".into(),
                        tags: vec!["recovered".into()],
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let orphan_id = NoteId::new(99).unwrap();
        drafts
            .save(
                &DraftRecord::new(
                    conflict_id,
                    2,
                    8,
                    98,
                    NoteChanges {
                        modified_unix_ms: 98,
                        title: "Conflicting title".into(),
                        body: "Conflicting private body".into(),
                        tags: Vec::new(),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        drafts
            .save(
                &DraftRecord::new(
                    orphan_id,
                    1,
                    9,
                    97,
                    NoteChanges {
                        modified_unix_ms: 98,
                        title: "Orphaned title".into(),
                        body: "Orphaned private body".into(),
                        tags: Vec::new(),
                    },
                )
                .unwrap(),
            )
            .unwrap();

        let worker = NotesWorker::start(paths).unwrap();
        ready(&worker);
        let review = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::DraftReview(summary) => summary,
            event => panic!("expected draft review, got {event:?}"),
        };
        assert_eq!(review.drafts.len(), 3);
        assert_eq!(review.drafts[0].note_id, note_id);
        assert_eq!(review.drafts[0].kind, DraftRecoveryKind::Applicable);
        assert_eq!(review.drafts[1].note_id, conflict_id);
        assert_eq!(review.drafts[1].kind, DraftRecoveryKind::Conflict);
        assert_eq!(review.drafts[2].note_id, orphan_id);
        assert_eq!(review.drafts[2].kind, DraftRecoveryKind::Orphaned);

        worker
            .try_send(WorkerCommand::RestoreDraft {
                request_id: 2,
                note_id,
            })
            .unwrap();
        let restored = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::DraftRestored(event) => event,
            event => panic!("expected restored draft, got {event:?}"),
        };
        assert_eq!(restored.request_id, 2);
        assert_eq!(restored.draft.changes.body, "Recovered private body");
        assert!(!format!("{restored:?}").contains("Recovered private body"));

        worker
            .try_send(WorkerCommand::DiscardDraft {
                request_id: 3,
                note_id,
            })
            .unwrap();
        assert!(matches!(
            worker.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::DraftDiscarded {
                request_id: 3,
                note_id: discarded,
            } if discarded == note_id
        ));
        assert_eq!(drafts.load(note_id).unwrap(), None);

        worker
            .try_send(WorkerCommand::DiscardDraft {
                request_id: 4,
                note_id,
            })
            .unwrap();
        assert!(matches!(
            worker.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::Rejected(RejectedEvent {
                request_id: 4,
                failure: WorkerFailure::MissingDraft,
                ..
            })
        ));
        worker
            .try_send(WorkerCommand::DiscardDraft {
                request_id: 5,
                note_id: conflict_id,
            })
            .unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        worker
            .try_send(WorkerCommand::DiscardDraft {
                request_id: 6,
                note_id: orphan_id,
            })
            .unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn startup_prunes_a_draft_identical_to_the_durable_note() {
        let (container, paths) = roots("draft-prune");
        let worker = NotesWorker::start(paths.clone()).unwrap();
        ready(&worker);
        let (note_id, _) = create_note(&worker, 1);
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);

        let drafts = DraftStore::for_library(paths.data_root());
        drafts
            .save(
                &DraftRecord::new(
                    note_id,
                    1,
                    1,
                    10,
                    NoteChanges {
                        modified_unix_ms: 10,
                        title: "Private title".into(),
                        body: "Initial body".into(),
                        tags: Vec::new(),
                    },
                )
                .unwrap(),
            )
            .unwrap();

        let worker = NotesWorker::start(paths).unwrap();
        ready(&worker);
        assert!(matches!(
            worker.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        ));
        assert_eq!(drafts.load(note_id).unwrap(), None);
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn unrelated_durable_change_keeps_local_candidate_pending_until_discard() {
        let (container, paths) = roots("pending");
        let worker =
            NotesWorker::start_with_debounce(paths.clone(), Duration::from_millis(20)).unwrap();
        ready(&worker);
        let (note_id, created) = create_note(&worker, 1);
        let mut external = LibraryTransaction::begin(&created.snapshot).unwrap();
        external.set_sort_order(SortOrder::Title);
        let external = external.finish().unwrap();
        std::fs::write(
            paths.data_root().join("library.bin"),
            encode(&external).unwrap(),
        )
        .unwrap();

        worker
            .try_send(WorkerCommand::ScheduleEdit(scheduled_edit(
                2,
                1,
                note_id,
                1,
                "Local pending body",
            )))
            .unwrap();
        let pending = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Pending(event) => event,
            event => panic!("expected pending save, got {event:?}"),
        };
        assert_eq!(pending.request_id, 2);
        assert!(matches!(pending.reason, PendingReason::Store(_)));
        assert_eq!(pending.draft_error, None);
        assert_eq!(pending.accepted.snapshot.notes[0].body, "Initial body");
        let draft_store = DraftStore::for_library(paths.data_root());
        let retained = draft_store.load(note_id).unwrap().unwrap();
        assert_eq!(retained.base_note_revision, 1);
        assert_eq!(retained.edit_generation, 1);
        assert_eq!(retained.changes.body, "Local pending body");

        worker.try_send(WorkerCommand::RetryPending).unwrap();
        let conflict = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Pending(event) => event,
            event => panic!("expected retained conflict, got {event:?}"),
        };
        assert_eq!(conflict.request_id, 2);
        assert_eq!(conflict.generation, EditGeneration::new(1));
        assert_eq!(conflict.reason, PendingReason::AcceptedStateChanged);
        assert_eq!(conflict.draft_error, None);
        assert_eq!(conflict.accepted.snapshot.sort_order, SortOrder::Title);
        assert_eq!(conflict.accepted.snapshot.notes[0].body, "Initial body");
        assert!(draft_store.load(note_id).unwrap().is_some());

        worker
            .try_send(WorkerCommand::DiscardPending { request_id: 3 })
            .unwrap();
        assert!(matches!(
            worker.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::DraftDiscarded {
                request_id: 3,
                note_id: discarded,
            } if discarded == note_id
        ));
        let discarded = ready(&worker);
        assert_eq!(discarded.request_id, Some(3));
        assert_eq!(discarded.snapshot, conflict.accepted.snapshot);
        assert_eq!(draft_store.load(note_id).unwrap(), None);
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pending_commit_discloses_when_private_draft_storage_is_unavailable() {
        use std::os::unix::fs::symlink;

        let (container, paths) = roots("draft-unavailable");
        let worker =
            NotesWorker::start_with_debounce(paths.clone(), Duration::from_millis(20)).unwrap();
        ready(&worker);
        let (note_id, created) = create_note(&worker, 1);
        let mut external = LibraryTransaction::begin(&created.snapshot).unwrap();
        external.set_sort_order(SortOrder::Title);
        let external = external.finish().unwrap();
        std::fs::write(
            paths.data_root().join("library.bin"),
            encode(&external).unwrap(),
        )
        .unwrap();
        let outside = container.join("outside-drafts");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, paths.data_root().join("drafts")).unwrap();

        worker
            .try_send(WorkerCommand::ScheduleEdit(scheduled_edit(
                2,
                1,
                note_id,
                1,
                "Memory-only pending body",
            )))
            .unwrap();

        let pending = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Pending(event) => event,
            event => panic!("expected pending save, got {event:?}"),
        };
        let error = pending
            .draft_error
            .expect("the unavailable draft store must remain visible");
        assert_eq!(
            error.operation,
            rmac_notes_storage::DraftOperation::PrepareDirectory
        );
        assert!(matches!(
            error.kind,
            rmac_notes_storage::DraftErrorKind::Io(io::ErrorKind::InvalidData)
        ));

        worker
            .try_send(WorkerCommand::DiscardPending { request_id: 3 })
            .unwrap();
        assert!(matches!(
            worker.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::Ready(_)
        ));
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn permanent_delete_requires_exact_trash_revision_and_verified_cleanup() {
        let (container, paths) = roots("permanent-delete");
        let worker = NotesWorker::start(paths).unwrap();
        ready(&worker);
        let (note_id, created) = create_note(&worker, 1);

        assert!(matches!(
            apply_action(
                &worker,
                2,
                LibraryAction::DeleteNotePermanently {
                    note_id,
                    expected_revision: 1,
                },
            ),
            WorkerEvent::Rejected(RejectedEvent {
                failure: WorkerFailure::Mutation(MutationError::NoteNotTrashed),
                ..
            })
        ));
        let trashed = match apply_action(
            &worker,
            3,
            LibraryAction::TrashNote {
                note_id,
                expected_revision: created.snapshot.notes[0].revision,
            },
        ) {
            WorkerEvent::Accepted(event) => event.accepted,
            event => panic!("expected accepted Trash transition, got {event:?}"),
        };
        assert!(trashed.snapshot.notes[0].deleted);

        let deleted = match apply_action(
            &worker,
            4,
            LibraryAction::DeleteNotePermanently {
                note_id,
                expected_revision: trashed.snapshot.notes[0].revision,
            },
        ) {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected accepted permanent delete, got {event:?}"),
        };
        assert_eq!(
            deleted.result,
            ActionResult::PermanentDeleteAccepted {
                note_id,
                attachment_count: 0,
                attachment_bytes: 0,
            }
        );
        assert!(!deleted.commit.purge_cleanup_pending);
        assert!(deleted.accepted.snapshot.notes.is_empty());

        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn empty_trash_never_sweeps_a_newer_library_revision() {
        let (container, paths) = roots("empty-trash");
        let worker = NotesWorker::start(paths).unwrap();
        ready(&worker);
        let (first, first_created) = create_note(&worker, 1);
        let (second, second_created) = create_note(&worker, 2);
        let first_trashed = match apply_action(
            &worker,
            3,
            LibraryAction::TrashNote {
                note_id: first,
                expected_revision: first_created.snapshot.notes[0].revision,
            },
        ) {
            WorkerEvent::Accepted(event) => event.accepted,
            event => panic!("expected first Trash transition, got {event:?}"),
        };
        let second_revision = second_created
            .snapshot
            .notes
            .iter()
            .find(|note| note.id == second)
            .unwrap()
            .revision;
        let second_trashed = match apply_action(
            &worker,
            4,
            LibraryAction::TrashNote {
                note_id: second,
                expected_revision: second_revision,
            },
        ) {
            WorkerEvent::Accepted(event) => event.accepted,
            event => panic!("expected second Trash transition, got {event:?}"),
        };

        assert!(matches!(
            apply_action(
                &worker,
                5,
                LibraryAction::EmptyTrash {
                    expected_library_revision: first_trashed.snapshot.revision,
                },
            ),
            WorkerEvent::Rejected(RejectedEvent {
                failure: WorkerFailure::Mutation(MutationError::RevisionConflict),
                ..
            })
        ));
        let emptied = match apply_action(
            &worker,
            6,
            LibraryAction::EmptyTrash {
                expected_library_revision: second_trashed.snapshot.revision,
            },
        ) {
            WorkerEvent::Accepted(event) => event,
            event => panic!("expected accepted Empty Trash, got {event:?}"),
        };
        assert_eq!(
            emptied.result,
            ActionResult::EmptyTrashAccepted {
                note_count: 2,
                attachment_count: 0,
                attachment_bytes: 0,
            }
        );
        assert!(!emptied.commit.purge_cleanup_pending);
        assert!(emptied.accepted.snapshot.notes.is_empty());

        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn bounded_queues_report_backpressure_instead_of_growing() {
        let (container, paths) = roots("backpressure");
        std::fs::create_dir_all(paths.legacy_root()).unwrap();
        std::fs::write(paths.legacy_root().join("legacy.md"), b"Legacy").unwrap();
        let worker = NotesWorker::start(paths).unwrap();
        assert!(matches!(
            worker.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::MigrationReview(_)
        ));

        let mut saw_full = false;
        for request_id in 1..=(COMMAND_CAPACITY as u64 + EVENT_CAPACITY as u64 + 64) {
            match worker.try_send(WorkerCommand::Flush { request_id }) {
                Ok(()) => {}
                Err(WorkerSendError::Full) => {
                    saw_full = true;
                    break;
                }
                Err(error) => panic!("unexpected queue error {error:?}"),
            }
        }
        assert!(saw_full);
        drop(worker);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn worker_debug_output_never_contains_editor_text() {
        let command = WorkerCommand::ScheduleEdit(scheduled_edit(
            1,
            1,
            NoteId::new(1).unwrap(),
            1,
            "secret worker body",
        ));
        let debug = format!("{command:?}");

        assert!(!debug.contains("Private title"));
        assert!(!debug.contains("secret worker body"));
        assert!(debug.contains("[private]"));
    }

    #[test]
    fn split_endpoints_support_background_event_delivery() {
        let (container, paths) = roots("split");
        let worker = NotesWorker::start(paths).unwrap();
        let (client, events) = worker.into_parts();

        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::Ready(_)
        ));
        client
            .try_send(WorkerCommand::Apply(
                ActionRequest::new(
                    1,
                    LibraryAction::CreateNote(NewNote {
                        created_unix_ms: 10,
                        title: "Private title".into(),
                        body: "Private body".into(),
                        tags: Vec::new(),
                        folder_id: None,
                    }),
                )
                .unwrap(),
            ))
            .unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::Accepted(_)
        ));
        client.try_send(WorkerCommand::Shutdown).unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::Stopped { .. }
        ));

        drop(events);
        assert_eq!(
            client.try_send(WorkerCommand::Flush { request_id: 2 }),
            Err(WorkerSendError::Closed)
        );
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn dropping_event_endpoint_stops_worker_while_clients_remain() {
        let (container, paths) = roots("split-drop");
        let worker = NotesWorker::start(paths).unwrap();
        let (client, events) = worker.into_parts();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            WorkerEvent::Ready(_)
        ));

        drop(events);

        assert_eq!(
            client.try_send(WorkerCommand::Flush { request_id: 1 }),
            Err(WorkerSendError::Closed)
        );
        std::fs::remove_dir_all(container).unwrap();
    }
}
