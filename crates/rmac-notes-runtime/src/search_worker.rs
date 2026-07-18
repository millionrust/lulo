use std::fmt;
use std::io;
use std::sync::mpsc::{
    self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rmac_notes_store::LibrarySnapshot;

use crate::{
    NotesSearchIndex, NotesSearchSession, SearchBatch, SearchCancellation, SearchError,
    SearchGeneration, SearchRequest,
};

pub const SEARCH_COMMAND_CAPACITY: usize = 8;
pub const SEARCH_EVENT_CAPACITY: usize = 16;

pub struct SearchJob {
    snapshot: Arc<LibrarySnapshot>,
    request: SearchRequest,
}

impl SearchJob {
    pub fn new(
        snapshot: Arc<LibrarySnapshot>,
        request: SearchRequest,
    ) -> Result<Self, SearchWorkerSendError> {
        if snapshot.revision != request.library_revision() {
            return Err(SearchWorkerSendError::InvalidRequest);
        }
        Ok(Self { snapshot, request })
    }

    pub fn generation(&self) -> SearchGeneration {
        self.request.generation()
    }

    pub fn library_revision(&self) -> u64 {
        self.request.library_revision()
    }
}

impl fmt::Debug for SearchJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchJob")
            .field("generation", &self.generation())
            .field("library_revision", &self.library_revision())
            .field("request", &self.request)
            .finish()
    }
}

enum SearchWorkerCommand {
    Run(SearchJob),
    Shutdown,
}

impl fmt::Debug for SearchWorkerCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(job) => formatter.debug_tuple("Run").field(job).finish(),
            Self::Shutdown => formatter.write_str("Shutdown"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchWorkerEvent {
    Started {
        generation: SearchGeneration,
        library_revision: u64,
        rebuilding: bool,
    },
    Results(SearchBatch),
    Failed {
        generation: SearchGeneration,
        library_revision: u64,
        error: SearchError,
    },
    Stopped {
        jobs_started: u64,
        index_rebuilds: u64,
    },
}

impl SearchWorkerEvent {
    pub fn project(self, session: &mut NotesSearchSession) -> bool {
        match self {
            Self::Started {
                generation,
                library_revision,
                ..
            } => session.is_pending(generation, library_revision),
            Self::Results(batch) => session.apply(batch),
            Self::Failed {
                generation,
                library_revision,
                error,
            } => session.fail(generation, library_revision, error),
            Self::Stopped { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchWorkerSendError {
    InvalidRequest,
    Full,
    Closed,
}

impl fmt::Display for SearchWorkerSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "the Notes search job is invalid",
            Self::Full => "the Notes search queue is full",
            Self::Closed => "Notes search is no longer available",
        })
    }
}

impl std::error::Error for SearchWorkerSendError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchWorkerStartError {
    Thread(io::ErrorKind),
}

impl fmt::Display for SearchWorkerStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Notes could not start its private search worker")
    }
}

impl std::error::Error for SearchWorkerStartError {}

pub struct NotesSearchWorker {
    commands: Option<SyncSender<SearchWorkerCommand>>,
    events: Option<Receiver<SearchWorkerEvent>>,
    active: Arc<Mutex<Option<SearchCancellation>>>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct NotesSearchWorkerClient {
    commands: SyncSender<SearchWorkerCommand>,
}

impl NotesSearchWorkerClient {
    pub fn try_run(
        &self,
        snapshot: Arc<LibrarySnapshot>,
        request: SearchRequest,
    ) -> Result<(), SearchWorkerSendError> {
        try_send_job(&self.commands, SearchJob::new(snapshot, request)?)
    }

    /// Nonblockingly asks the search thread to stop. Callers should cancel
    /// their active [`NotesSearchSession`] first, then retry if bounded command
    /// backpressure reports [`SearchWorkerSendError::Full`].
    pub fn try_shutdown(&self) -> Result<(), SearchWorkerSendError> {
        self.commands
            .try_send(SearchWorkerCommand::Shutdown)
            .map_err(|error| match error {
                TrySendError::Full(_) => SearchWorkerSendError::Full,
                TrySendError::Disconnected(_) => SearchWorkerSendError::Closed,
            })
    }

    /// Requests shutdown after bounded queue space becomes available. This is
    /// intended for a background teardown helper, not an interactive UI path.
    pub fn shutdown_blocking(&self) -> Result<(), SearchWorkerSendError> {
        self.commands
            .send(SearchWorkerCommand::Shutdown)
            .map_err(|_| SearchWorkerSendError::Closed)
    }
}

impl fmt::Debug for NotesSearchWorkerClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("NotesSearchWorkerClient").finish()
    }
}

pub struct NotesSearchWorkerEvents {
    events: Option<Receiver<SearchWorkerEvent>>,
    shutdown: Option<SyncSender<SearchWorkerCommand>>,
    active: Arc<Mutex<Option<SearchCancellation>>>,
    thread: Option<JoinHandle<()>>,
}

impl NotesSearchWorkerEvents {
    pub fn recv(&self) -> Result<SearchWorkerEvent, RecvError> {
        self.events.as_ref().ok_or(RecvError)?.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<SearchWorkerEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<SearchWorkerEvent, TryRecvError> {
        self.events
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }
}

impl fmt::Debug for NotesSearchWorkerEvents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("NotesSearchWorkerEvents").finish()
    }
}

impl Drop for NotesSearchWorkerEvents {
    fn drop(&mut self) {
        self.events.take();
        cancel_active(&self.active);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.try_send(SearchWorkerCommand::Shutdown);
            drop(shutdown);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl NotesSearchWorker {
    pub fn start() -> Result<Self, SearchWorkerStartError> {
        let (command_sender, command_receiver) = mpsc::sync_channel(SEARCH_COMMAND_CAPACITY);
        let (event_sender, event_receiver) = mpsc::sync_channel(SEARCH_EVENT_CAPACITY);
        let active = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let thread = thread::Builder::new()
            .name("rmac-notes-search".into())
            .spawn(move || run_worker(command_receiver, event_sender, worker_active))
            .map_err(|error| SearchWorkerStartError::Thread(error.kind()))?;
        Ok(Self {
            commands: Some(command_sender),
            events: Some(event_receiver),
            active,
            thread: Some(thread),
        })
    }

    pub fn try_run(
        &self,
        snapshot: Arc<LibrarySnapshot>,
        request: SearchRequest,
    ) -> Result<(), SearchWorkerSendError> {
        try_send_job(
            self.commands
                .as_ref()
                .ok_or(SearchWorkerSendError::Closed)?,
            SearchJob::new(snapshot, request)?,
        )
    }

    pub fn recv(&self) -> Result<SearchWorkerEvent, RecvError> {
        self.events.as_ref().ok_or(RecvError)?.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<SearchWorkerEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<SearchWorkerEvent, TryRecvError> {
        self.events
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }

    pub fn into_parts(mut self) -> (NotesSearchWorkerClient, NotesSearchWorkerEvents) {
        let commands = self
            .commands
            .take()
            .expect("a live Notes search worker owns its command endpoint");
        let events = self
            .events
            .take()
            .expect("a live Notes search worker owns its event endpoint");
        let thread = self
            .thread
            .take()
            .expect("a live Notes search worker owns its thread");
        (
            NotesSearchWorkerClient {
                commands: commands.clone(),
            },
            NotesSearchWorkerEvents {
                events: Some(events),
                shutdown: Some(commands),
                active: self.active.clone(),
                thread: Some(thread),
            },
        )
    }
}

impl fmt::Debug for NotesSearchWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("NotesSearchWorker").finish()
    }
}

impl Drop for NotesSearchWorker {
    fn drop(&mut self) {
        self.events.take();
        cancel_active(&self.active);
        if let Some(commands) = self.commands.take() {
            let _ = commands.try_send(SearchWorkerCommand::Shutdown);
            drop(commands);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn try_send_job(
    commands: &SyncSender<SearchWorkerCommand>,
    job: SearchJob,
) -> Result<(), SearchWorkerSendError> {
    commands
        .try_send(SearchWorkerCommand::Run(job))
        .map_err(|error| match error {
            TrySendError::Full(_) => SearchWorkerSendError::Full,
            TrySendError::Disconnected(_) => SearchWorkerSendError::Closed,
        })
}

fn lock_active(
    active: &Arc<Mutex<Option<SearchCancellation>>>,
) -> MutexGuard<'_, Option<SearchCancellation>> {
    active
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cancel_active(active: &Arc<Mutex<Option<SearchCancellation>>>) {
    if let Some(cancellation) = lock_active(active).take() {
        cancellation.cancel();
    }
}

fn run_worker(
    commands: Receiver<SearchWorkerCommand>,
    events: SyncSender<SearchWorkerEvent>,
    active: Arc<Mutex<Option<SearchCancellation>>>,
) {
    let mut index: Option<NotesSearchIndex> = None;
    let mut jobs_started = 0_u64;
    let mut index_rebuilds = 0_u64;
    while let Ok(command) = commands.recv() {
        let SearchWorkerCommand::Run(job) = command else {
            break;
        };
        if job.request.cancellation().is_cancelled() {
            continue;
        }
        jobs_started = jobs_started.saturating_add(1);
        let generation = job.generation();
        let library_revision = job.library_revision();
        let rebuilding = index
            .as_ref()
            .is_none_or(|index| index.library_revision() != library_revision);
        *lock_active(&active) = Some(job.request.cancellation().clone());
        if events
            .send(SearchWorkerEvent::Started {
                generation,
                library_revision,
                rebuilding,
            })
            .is_err()
        {
            break;
        }

        if rebuilding {
            match NotesSearchIndex::build_cancellable(
                job.snapshot.clone(),
                job.request.cancellation(),
            ) {
                Ok(rebuilt) => {
                    index = Some(rebuilt);
                    index_rebuilds = index_rebuilds.saturating_add(1);
                }
                Err(SearchError::Cancelled) => {
                    lock_active(&active).take();
                    continue;
                }
                Err(error) => {
                    lock_active(&active).take();
                    if events
                        .send(SearchWorkerEvent::Failed {
                            generation,
                            library_revision,
                            error,
                        })
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
            }
        }

        let result = index
            .as_ref()
            .expect("a successful rebuild installs the requested index")
            .search(&job.request);
        lock_active(&active).take();
        let event = match result {
            Ok(batch) => SearchWorkerEvent::Results(batch),
            Err(SearchError::Cancelled) => continue,
            Err(error) => SearchWorkerEvent::Failed {
                generation,
                library_revision,
                error,
            },
        };
        if events.send(event).is_err() {
            break;
        }
    }
    cancel_active(&active);
    let _ = events.send(SearchWorkerEvent::Stopped {
        jobs_started,
        index_rebuilds,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notes_store::{NoteId, NoteRecord, SortOrder};

    fn snapshot(revision: u64) -> Arc<LibrarySnapshot> {
        Arc::new(LibrarySnapshot {
            revision,
            sort_order: SortOrder::Edited,
            next_note_id: 2,
            next_folder_id: 1,
            next_attachment_id: 1,
            folders: Vec::new(),
            notes: vec![NoteRecord {
                id: NoteId::new(1).unwrap(),
                revision: 1,
                created_unix_ms: 1,
                modified_unix_ms: 2,
                title: "Private Roadmap".into(),
                body: "Search worker body".into(),
                tags: vec!["planning".into()],
                folder_id: None,
                pinned: false,
                deleted: false,
                attachments: Vec::new(),
            }],
            attachments: Vec::new(),
        })
    }

    fn next(worker: &NotesSearchWorker) -> SearchWorkerEvent {
        worker.recv_timeout(Duration::from_secs(2)).unwrap()
    }

    #[test]
    fn worker_rebuilds_once_then_reuses_the_exact_revision() {
        let worker = NotesSearchWorker::start().unwrap();
        let source = snapshot(1);
        let mut session = NotesSearchSession::new();
        let request = session.begin("roadmap", 10, 1).unwrap().unwrap();
        worker.try_run(source.clone(), request).unwrap();

        let started = next(&worker);
        assert_eq!(
            started,
            SearchWorkerEvent::Started {
                generation: SearchGeneration::new(1).unwrap(),
                library_revision: 1,
                rebuilding: true,
            }
        );
        assert!(started.project(&mut session));
        assert!(next(&worker).project(&mut session));
        assert_eq!(session.hits()[0].note_id, NoteId::new(1).unwrap());

        let request = session.begin("body", 10, 1).unwrap().unwrap();
        worker.try_run(source, request).unwrap();
        assert!(matches!(
            next(&worker),
            SearchWorkerEvent::Started {
                rebuilding: false,
                ..
            }
        ));
        assert!(next(&worker).project(&mut session));

        let request = session.begin("roadmap", 10, 2).unwrap().unwrap();
        worker.try_run(snapshot(2), request).unwrap();
        assert!(matches!(
            next(&worker),
            SearchWorkerEvent::Started {
                library_revision: 2,
                rebuilding: true,
                ..
            }
        ));
        assert!(next(&worker).project(&mut session));
    }

    #[test]
    fn cancelled_queued_work_never_publishes_over_the_latest_generation() {
        let worker = NotesSearchWorker::start().unwrap();
        let source = snapshot(1);
        let mut session = NotesSearchSession::new();
        let stale = session.begin("roadmap", 10, 1).unwrap().unwrap();
        let current = session.begin("body", 10, 1).unwrap().unwrap();
        worker.try_run(source.clone(), stale).unwrap();
        worker.try_run(source, current).unwrap();

        let started = next(&worker);
        assert!(matches!(
            started,
            SearchWorkerEvent::Started { generation, .. }
                if generation == SearchGeneration::new(2).unwrap()
        ));
        assert!(started.project(&mut session));
        assert!(next(&worker).project(&mut session));
        assert_eq!(session.hits()[0].note_id, NoteId::new(1).unwrap());
    }

    #[test]
    fn mismatched_revisions_and_debug_output_expose_no_private_text() {
        let worker = NotesSearchWorker::start().unwrap();
        let source = snapshot(1);
        let request = SearchRequest::new(
            SearchGeneration::new(1).unwrap(),
            2,
            "private roadmap query",
            10,
            SearchCancellation::default(),
        )
        .unwrap();
        let debug = format!(
            "{:?}",
            SearchJob {
                snapshot: source.clone(),
                request: request.clone(),
            }
        );

        assert_eq!(
            worker.try_run(source, request),
            Err(SearchWorkerSendError::InvalidRequest)
        );
        assert!(!debug.contains("Private Roadmap"));
        assert!(!debug.contains("private roadmap query"));
        assert!(debug.contains("[private]"));
    }

    #[test]
    fn split_event_endpoint_owns_deterministic_shutdown() {
        let worker = NotesSearchWorker::start().unwrap();
        let (client, events) = worker.into_parts();
        drop(events);
        let request = SearchRequest::new(
            SearchGeneration::new(1).unwrap(),
            1,
            "roadmap",
            10,
            SearchCancellation::default(),
        )
        .unwrap();

        assert_eq!(
            client.try_run(snapshot(1), request),
            Err(SearchWorkerSendError::Closed)
        );
    }

    #[test]
    fn split_client_can_request_ordered_shutdown() {
        let worker = NotesSearchWorker::start().unwrap();
        let (client, events) = worker.into_parts();
        client.try_shutdown().unwrap();
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(2)).unwrap(),
            SearchWorkerEvent::Stopped {
                jobs_started: 0,
                index_rebuilds: 0,
            }
        ));
    }

    #[test]
    fn command_backpressure_is_bounded_and_nonblocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let first = SearchRequest::new(
            SearchGeneration::new(1).unwrap(),
            1,
            "roadmap",
            10,
            SearchCancellation::default(),
        )
        .unwrap();
        let second = SearchRequest::new(
            SearchGeneration::new(2).unwrap(),
            1,
            "body",
            10,
            SearchCancellation::default(),
        )
        .unwrap();

        assert_eq!(
            try_send_job(&sender, SearchJob::new(snapshot(1), first).unwrap()),
            Ok(())
        );
        assert_eq!(
            try_send_job(&sender, SearchJob::new(snapshot(1), second).unwrap()),
            Err(SearchWorkerSendError::Full)
        );
    }
}
