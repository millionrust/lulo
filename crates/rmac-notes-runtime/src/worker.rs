use std::fmt;
use std::io;
use std::sync::mpsc::{
    self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rmac_notes_storage::{
    inspect_notes_startup, AcceptedCommit, AcceptedLibrary, CommitError, MigrationReview,
    MigrationWarning, NotesPaths, NotesStartup, PendingCommit, PendingReason, RecoveryNotice,
    StartupError,
};
use rmac_notes_store::{FolderId, LibrarySnapshot, MutationError, NewNote, NoteId, SortOrder};

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

pub enum WorkerCommand {
    AcceptMigration { request_id: u64 },
    StartEmpty { request_id: u64 },
    ScheduleEdit(ScheduledEdit),
    Apply(ActionRequest),
    Flush { request_id: u64 },
    RetryPending,
    DiscardPending { request_id: u64 },
    Shutdown,
}

impl WorkerCommand {
    fn request_context(&self) -> (u64, Option<EditGeneration>) {
        match self {
            Self::AcceptMigration { request_id }
            | Self::StartEmpty { request_id }
            | Self::Flush { request_id }
            | Self::DiscardPending { request_id } => (*request_id, None),
            Self::ScheduleEdit(edit) => (edit.request_id(), Some(edit.generation())),
            Self::Apply(request) => (request.request_id(), None),
            Self::RetryPending | Self::Shutdown => (0, None),
        }
    }
}

impl fmt::Debug for WorkerCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScheduleEdit(edit) => formatter.debug_tuple("ScheduleEdit").field(edit).finish(),
            Self::Apply(request) => formatter.debug_tuple("Apply").field(request).finish(),
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
    DeletedFolder { moved_notes: usize },
    RestoredNote { folder_id: Option<FolderId> },
}

#[derive(Clone, Debug)]
pub struct AcceptedEvent {
    pub request_id: u64,
    pub generation: Option<EditGeneration>,
    pub result: ActionResult,
    pub commit: AcceptedCommit,
    pub accepted: SnapshotEvent,
}

#[derive(Clone, Debug)]
pub struct PendingEvent {
    pub request_id: u64,
    pub generation: Option<EditGeneration>,
    pub reason: PendingReason,
    pub accepted: SnapshotEvent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerFailure {
    WrongPhase,
    CommitPending,
    Scheduler(SchedulerError),
    Mutation(MutationError),
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
    Coalesced {
        request_id: u64,
        generation: EditGeneration,
        replaced_generation: EditGeneration,
    },
    Accepted(AcceptedEvent),
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
        let (request_id, _) = command.request_context();
        if !matches!(
            command,
            WorkerCommand::RetryPending | WorkerCommand::Shutdown
        ) && request_id == 0
        {
            return Err(WorkerSendError::InvalidRequest);
        }
        self.commands
            .as_ref()
            .ok_or(WorkerSendError::Closed)?
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => WorkerSendError::Full,
                TrySendError::Disconnected(_) => WorkerSendError::Closed,
            })
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

struct ReadyState {
    library: Box<AcceptedLibrary>,
    scheduler: EditScheduler,
}

#[derive(Clone, Copy)]
struct RequestContext {
    request_id: u64,
    generation: Option<EditGeneration>,
    result: ActionResult,
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
            if events
                .send(WorkerEvent::Ready(SnapshotEvent::from_library(
                    &library, None,
                )))
                .is_err()
            {
                return;
            }
            Phase::Ready(ReadyState { library, scheduler })
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
                Ok(library) => {
                    let library = Box::new(library);
                    if emit_ready(events, &library, Some(request_id)) {
                        Phase::Ready(ReadyState {
                            library,
                            scheduler: review.scheduler,
                        })
                    } else {
                        Phase::Stopped
                    }
                }
                Err(error) => {
                    let _ = events.send(WorkerEvent::StartupFailed(error));
                    Phase::Stopped
                }
            }
        }
        (Phase::Review(review), WorkerCommand::StartEmpty { request_id }) => {
            let library = Box::new(review.review.start_empty());
            if emit_ready(events, &library, Some(request_id)) {
                Phase::Ready(ReadyState {
                    library,
                    scheduler: review.scheduler,
                })
            } else {
                Phase::Stopped
            }
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
        (Phase::Pending(pending), WorkerCommand::DiscardPending { request_id }) => {
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
    if let Err(error) = transaction.edit_note(note_id, current_revision, edit.into_changes()) {
        return reject_mutation(events, request_id, Some(generation), error);
    }
    let context = RequestContext {
        request_id,
        generation: Some(generation),
        result: ActionResult::Edited(note_id),
    };
    commit_transaction(ready, transaction, context, events)
}

fn commit_action(
    ready: &mut ReadyState,
    request: ActionRequest,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let mut transaction = match ready.library.begin() {
        Ok(transaction) => transaction,
        Err(error) => return reject_mutation(events, request.request_id, None, error),
    };
    let result = match request.action {
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
    };
    let result = match result {
        Ok(result) => result,
        Err(error) => return reject_mutation(events, request.request_id, None, error),
    };
    let context = RequestContext {
        request_id: request.request_id,
        generation: None,
        result,
    };
    commit_transaction(ready, transaction, context, events)
}

fn commit_transaction(
    ready: &mut ReadyState,
    transaction: rmac_notes_store::LibraryTransaction,
    context: RequestContext,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    match ready.library.commit(transaction) {
        Ok(commit) => {
            let event = AcceptedEvent {
                request_id: context.request_id,
                generation: context.generation,
                result: context.result,
                commit,
                accepted: SnapshotEvent::from_library(&ready.library, Some(context.request_id)),
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
        }))
        .is_ok()
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
    use rmac_notes_storage::PendingReason;
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

    #[test]
    fn worker_commits_actions_and_due_edits_without_idle_events() {
        let (container, paths) = roots("edit");
        let worker = NotesWorker::start_with_debounce(paths, Duration::from_millis(30)).unwrap();
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
        assert_eq!(pending.accepted.snapshot.notes[0].body, "Initial body");

        worker.try_send(WorkerCommand::RetryPending).unwrap();
        let conflict = match worker.recv_timeout(Duration::from_secs(2)).unwrap() {
            WorkerEvent::Pending(event) => event,
            event => panic!("expected retained conflict, got {event:?}"),
        };
        assert_eq!(conflict.request_id, 2);
        assert_eq!(conflict.generation, EditGeneration::new(1));
        assert_eq!(conflict.reason, PendingReason::AcceptedStateChanged);
        assert_eq!(conflict.accepted.snapshot.sort_order, SortOrder::Title);
        assert_eq!(conflict.accepted.snapshot.notes[0].body, "Initial body");

        worker
            .try_send(WorkerCommand::DiscardPending { request_id: 3 })
            .unwrap();
        let discarded = ready(&worker);
        assert_eq!(discarded.request_id, Some(3));
        assert_eq!(discarded.snapshot, conflict.accepted.snapshot);
        worker.try_send(WorkerCommand::Shutdown).unwrap();
        let _ = worker.recv_timeout(Duration::from_secs(2)).unwrap();
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
}
