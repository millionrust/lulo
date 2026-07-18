use std::fmt;
use std::io;
use std::sync::mpsc::{
    self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rmac_notes_storage::{
    parse_inert_markdown_preview, MarkdownPreviewDocument, MarkdownPreviewError,
};
use rmac_notes_store::{NoteId, MAX_BODY_BYTES};

use crate::{PreviewCancellation, PreviewGeneration};

pub const MARKDOWN_PREVIEW_COMMAND_CAPACITY: usize = 4;
pub const MARKDOWN_PREVIEW_EVENT_CAPACITY: usize = 2;

#[derive(Clone)]
pub struct MarkdownPreviewRequest {
    generation: PreviewGeneration,
    library_revision: u64,
    note_id: NoteId,
    note_revision: u64,
    source: Arc<str>,
    cancellation: PreviewCancellation,
}

impl MarkdownPreviewRequest {
    fn new(
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
        source: Arc<str>,
    ) -> Result<Self, MarkdownPreviewRequestError> {
        if library_revision == 0 || note_revision == 0 || source.len() > MAX_BODY_BYTES {
            return Err(MarkdownPreviewRequestError::InvalidRequest);
        }
        Ok(Self {
            generation,
            library_revision,
            note_id,
            note_revision,
            source,
            cancellation: PreviewCancellation::default(),
        })
    }

    pub fn generation(&self) -> PreviewGeneration {
        self.generation
    }

    pub fn library_revision(&self) -> u64 {
        self.library_revision
    }

    pub fn note_id(&self) -> NoteId {
        self.note_id
    }

    pub fn note_revision(&self) -> u64 {
        self.note_revision
    }

    pub fn cancellation(&self) -> &PreviewCancellation {
        &self.cancellation
    }
}

impl fmt::Debug for MarkdownPreviewRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MarkdownPreviewRequest")
            .field("generation", &self.generation)
            .field("library_revision", &self.library_revision)
            .field("note_id", &self.note_id)
            .field("note_revision", &self.note_revision)
            .field("source_bytes", &self.source.len())
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownPreviewRequestError {
    InvalidRequest,
    GenerationExhausted,
}

impl fmt::Display for MarkdownPreviewRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "the Notes Markdown preview request is invalid",
            Self::GenerationExhausted => "the Notes Markdown preview generation is exhausted",
        })
    }
}

impl std::error::Error for MarkdownPreviewRequestError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownPreviewState {
    Empty,
    Loading {
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
    },
    Ready {
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
        document: Arc<MarkdownPreviewDocument>,
    },
    Unavailable {
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
        error: MarkdownPreviewError,
    },
}

pub struct NotesMarkdownPreviewSession {
    next_generation: u64,
    active: Option<PreviewCancellation>,
    state: MarkdownPreviewState,
}

impl NotesMarkdownPreviewSession {
    pub fn new() -> Self {
        Self {
            next_generation: 1,
            active: None,
            state: MarkdownPreviewState::Empty,
        }
    }

    pub fn state(&self) -> &MarkdownPreviewState {
        &self.state
    }

    pub fn request(
        &mut self,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
        source: Arc<str>,
    ) -> Result<MarkdownPreviewRequest, MarkdownPreviewRequestError> {
        let generation = PreviewGeneration::new(self.next_generation)
            .ok_or(MarkdownPreviewRequestError::GenerationExhausted)?;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .filter(|next| *next != 0)
            .ok_or(MarkdownPreviewRequestError::GenerationExhausted)?;
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        let request = MarkdownPreviewRequest::new(
            generation,
            library_revision,
            note_id,
            note_revision,
            source,
        )?;
        self.active = Some(request.cancellation.clone());
        self.state = MarkdownPreviewState::Loading {
            generation,
            library_revision,
            note_id,
            note_revision,
        };
        Ok(request)
    }

    pub fn clear(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        self.state = MarkdownPreviewState::Empty;
    }

    fn is_pending(
        &self,
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
    ) -> bool {
        matches!(
            self.state,
            MarkdownPreviewState::Loading {
                generation: pending_generation,
                library_revision: pending_library,
                note_id: pending_note,
                note_revision: pending_note_revision,
            } if generation == pending_generation
                && library_revision == pending_library
                && note_id == pending_note
                && note_revision == pending_note_revision
        )
    }

    fn ready(
        &mut self,
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
        document: Arc<MarkdownPreviewDocument>,
    ) -> bool {
        if !self.is_pending(generation, library_revision, note_id, note_revision) {
            return false;
        }
        self.active = None;
        self.state = MarkdownPreviewState::Ready {
            generation,
            library_revision,
            note_id,
            note_revision,
            document,
        };
        true
    }

    fn fail(
        &mut self,
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
        error: MarkdownPreviewError,
    ) -> bool {
        if !self.is_pending(generation, library_revision, note_id, note_revision) {
            return false;
        }
        self.active = None;
        self.state = MarkdownPreviewState::Unavailable {
            generation,
            library_revision,
            note_id,
            note_revision,
            error,
        };
        true
    }
}

impl Default for NotesMarkdownPreviewSession {
    fn default() -> Self {
        Self::new()
    }
}

enum MarkdownPreviewWorkerCommand {
    Run(MarkdownPreviewRequest),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownPreviewWorkerEvent {
    Started {
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
    },
    Ready {
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
        document: Arc<MarkdownPreviewDocument>,
    },
    Failed {
        generation: PreviewGeneration,
        library_revision: u64,
        note_id: NoteId,
        note_revision: u64,
        error: MarkdownPreviewError,
    },
    Stopped {
        jobs_started: u64,
    },
}

impl MarkdownPreviewWorkerEvent {
    pub fn project(self, session: &mut NotesMarkdownPreviewSession) -> bool {
        match self {
            Self::Started {
                generation,
                library_revision,
                note_id,
                note_revision,
            } => session.is_pending(generation, library_revision, note_id, note_revision),
            Self::Ready {
                generation,
                library_revision,
                note_id,
                note_revision,
                document,
            } => session.ready(
                generation,
                library_revision,
                note_id,
                note_revision,
                document,
            ),
            Self::Failed {
                generation,
                library_revision,
                note_id,
                note_revision,
                error,
            } => session.fail(generation, library_revision, note_id, note_revision, error),
            Self::Stopped { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownPreviewWorkerSendError {
    Full,
    Closed,
}

impl fmt::Display for MarkdownPreviewWorkerSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Full => "the Notes Markdown preview queue is full",
            Self::Closed => "Notes Markdown preview is no longer available",
        })
    }
}

impl std::error::Error for MarkdownPreviewWorkerSendError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkdownPreviewWorkerStartError(io::ErrorKind);

impl fmt::Display for MarkdownPreviewWorkerStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Notes could not start its Markdown preview worker")
    }
}

impl std::error::Error for MarkdownPreviewWorkerStartError {}

pub struct NotesMarkdownPreviewWorker {
    commands: Option<SyncSender<MarkdownPreviewWorkerCommand>>,
    events: Option<Receiver<MarkdownPreviewWorkerEvent>>,
    active: Arc<Mutex<Option<PreviewCancellation>>>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct NotesMarkdownPreviewWorkerClient {
    commands: SyncSender<MarkdownPreviewWorkerCommand>,
    active: Arc<Mutex<Option<PreviewCancellation>>>,
}

impl NotesMarkdownPreviewWorkerClient {
    pub fn try_run(
        &self,
        request: MarkdownPreviewRequest,
    ) -> Result<(), MarkdownPreviewWorkerSendError> {
        try_send(&self.commands, request)
    }

    pub fn try_shutdown(&self) -> Result<(), MarkdownPreviewWorkerSendError> {
        cancel_active(&self.active);
        self.commands
            .try_send(MarkdownPreviewWorkerCommand::Shutdown)
            .map_err(map_send_error)
    }

    pub fn shutdown_blocking(&self) -> Result<(), MarkdownPreviewWorkerSendError> {
        cancel_active(&self.active);
        self.commands
            .send(MarkdownPreviewWorkerCommand::Shutdown)
            .map_err(|_| MarkdownPreviewWorkerSendError::Closed)
    }
}

impl fmt::Debug for NotesMarkdownPreviewWorkerClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotesMarkdownPreviewWorkerClient")
            .finish()
    }
}

pub struct NotesMarkdownPreviewWorkerEvents {
    events: Option<Receiver<MarkdownPreviewWorkerEvent>>,
    shutdown: Option<SyncSender<MarkdownPreviewWorkerCommand>>,
    active: Arc<Mutex<Option<PreviewCancellation>>>,
    thread: Option<JoinHandle<()>>,
}

impl NotesMarkdownPreviewWorkerEvents {
    pub fn recv(&self) -> Result<MarkdownPreviewWorkerEvent, RecvError> {
        self.events.as_ref().ok_or(RecvError)?.recv()
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<MarkdownPreviewWorkerEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<MarkdownPreviewWorkerEvent, TryRecvError> {
        self.events
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }
}

impl Drop for NotesMarkdownPreviewWorkerEvents {
    fn drop(&mut self) {
        self.events.take();
        cancel_active(&self.active);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.try_send(MarkdownPreviewWorkerCommand::Shutdown);
            drop(shutdown);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl NotesMarkdownPreviewWorker {
    pub fn start() -> Result<Self, MarkdownPreviewWorkerStartError> {
        let (command_sender, command_receiver) =
            mpsc::sync_channel(MARKDOWN_PREVIEW_COMMAND_CAPACITY);
        let (event_sender, event_receiver) = mpsc::sync_channel(MARKDOWN_PREVIEW_EVENT_CAPACITY);
        let active = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let thread = thread::Builder::new()
            .name("rmac-notes-markdown-preview".into())
            .spawn(move || run_worker(command_receiver, event_sender, worker_active))
            .map_err(|error| MarkdownPreviewWorkerStartError(error.kind()))?;
        Ok(Self {
            commands: Some(command_sender),
            events: Some(event_receiver),
            active,
            thread: Some(thread),
        })
    }

    pub fn into_parts(
        mut self,
    ) -> (
        NotesMarkdownPreviewWorkerClient,
        NotesMarkdownPreviewWorkerEvents,
    ) {
        let commands = self
            .commands
            .take()
            .expect("a live Markdown preview worker owns its command endpoint");
        let events = self
            .events
            .take()
            .expect("a live Markdown preview worker owns its event endpoint");
        let thread = self
            .thread
            .take()
            .expect("a live Markdown preview worker owns its thread");
        (
            NotesMarkdownPreviewWorkerClient {
                commands: commands.clone(),
                active: self.active.clone(),
            },
            NotesMarkdownPreviewWorkerEvents {
                events: Some(events),
                shutdown: Some(commands),
                active: self.active.clone(),
                thread: Some(thread),
            },
        )
    }
}

impl Drop for NotesMarkdownPreviewWorker {
    fn drop(&mut self) {
        self.events.take();
        cancel_active(&self.active);
        if let Some(commands) = self.commands.take() {
            let _ = commands.try_send(MarkdownPreviewWorkerCommand::Shutdown);
            drop(commands);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn try_send(
    commands: &SyncSender<MarkdownPreviewWorkerCommand>,
    request: MarkdownPreviewRequest,
) -> Result<(), MarkdownPreviewWorkerSendError> {
    commands
        .try_send(MarkdownPreviewWorkerCommand::Run(request))
        .map_err(map_send_error)
}

fn map_send_error<T>(error: TrySendError<T>) -> MarkdownPreviewWorkerSendError {
    match error {
        TrySendError::Full(_) => MarkdownPreviewWorkerSendError::Full,
        TrySendError::Disconnected(_) => MarkdownPreviewWorkerSendError::Closed,
    }
}

fn cancel_active(active: &Mutex<Option<PreviewCancellation>>) {
    if let Some(cancellation) = active
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        cancellation.cancel();
    }
}

fn run_worker(
    commands: Receiver<MarkdownPreviewWorkerCommand>,
    events: SyncSender<MarkdownPreviewWorkerEvent>,
    active: Arc<Mutex<Option<PreviewCancellation>>>,
) {
    let mut jobs_started = 0_u64;
    while let Ok(command) = commands.recv() {
        let MarkdownPreviewWorkerCommand::Run(request) = command else {
            break;
        };
        if request.cancellation.is_cancelled() {
            continue;
        }
        jobs_started = jobs_started.saturating_add(1);
        let generation = request.generation;
        let library_revision = request.library_revision;
        let note_id = request.note_id;
        let note_revision = request.note_revision;
        *active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.cancellation.clone());
        if events
            .send(MarkdownPreviewWorkerEvent::Started {
                generation,
                library_revision,
                note_id,
                note_revision,
            })
            .is_err()
        {
            break;
        }
        let result = parse_inert_markdown_preview(&request.source);
        active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if request.cancellation.is_cancelled() {
            continue;
        }
        let event = match result {
            Ok(document) => MarkdownPreviewWorkerEvent::Ready {
                generation,
                library_revision,
                note_id,
                note_revision,
                document: Arc::new(document),
            },
            Err(error) => MarkdownPreviewWorkerEvent::Failed {
                generation,
                library_revision,
                note_id,
                note_revision,
                error,
            },
        };
        if events.send(event).is_err() {
            break;
        }
    }
    let _ = events.send(MarkdownPreviewWorkerEvent::Stopped { jobs_started });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(session: &mut NotesMarkdownPreviewSession, source: &str) -> MarkdownPreviewRequest {
        session
            .request(7, NoteId::new(3).unwrap(), 5, Arc::<str>::from(source))
            .unwrap()
    }

    #[test]
    fn worker_projects_only_exact_generation_revision_and_note_identity() {
        let worker = NotesMarkdownPreviewWorker::start().unwrap();
        let mut session = NotesMarkdownPreviewSession::new();
        let request = request(
            &mut session,
            "# Private heading\n\n[link](https://private.invalid)",
        );
        assert!(!format!("{request:?}").contains("Private heading"));
        assert!(!format!("{request:?}").contains("private.invalid"));
        worker
            .commands
            .as_ref()
            .unwrap()
            .try_send(MarkdownPreviewWorkerCommand::Run(request))
            .unwrap();

        let started = worker
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(started.project(&mut session));
        let ready = worker
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(ready.project(&mut session));
        let MarkdownPreviewState::Ready { document, .. } = session.state() else {
            panic!("expected ready Markdown preview")
        };
        let visible = document
            .blocks()
            .iter()
            .map(|block| block.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(visible.contains("Private heading"));
        assert!(!visible.contains("private.invalid"));
    }

    #[test]
    fn newer_request_cancels_and_rejects_late_preview_events() {
        let mut session = NotesMarkdownPreviewSession::new();
        let first = request(&mut session, "first private body");
        let second = request(&mut session, "second private body");
        assert!(first.cancellation().is_cancelled());
        let stale = MarkdownPreviewWorkerEvent::Ready {
            generation: first.generation(),
            library_revision: first.library_revision(),
            note_id: first.note_id(),
            note_revision: first.note_revision(),
            document: Arc::new(parse_inert_markdown_preview("first").unwrap()),
        };
        assert!(!stale.project(&mut session));
        assert!(matches!(
            session.state(),
            MarkdownPreviewState::Loading { generation, .. }
                if *generation == second.generation()
        ));
    }
}
