use std::fmt;
use std::io;
use std::path::{Component, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rmac_notes_storage::{
    load_managed_image_preview, DecodedImagePreview, PreviewError, PreviewSize,
};
use rmac_notes_store::{AttachmentId, AttachmentRecord};

pub const PREVIEW_COMMAND_CAPACITY: usize = 4;
pub const PREVIEW_EVENT_CAPACITY: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PreviewGeneration(u64);

impl PreviewGeneration {
    pub fn new(value: u64) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Default)]
pub struct PreviewCancellation(Arc<AtomicBool>);

impl PreviewCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl fmt::Debug for PreviewCancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreviewCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone)]
pub struct PreviewRequest {
    generation: PreviewGeneration,
    library_revision: u64,
    attachment: AttachmentRecord,
    target: PreviewSize,
    cancellation: PreviewCancellation,
}

impl PreviewRequest {
    fn new(
        generation: PreviewGeneration,
        library_revision: u64,
        attachment: AttachmentRecord,
        target: PreviewSize,
    ) -> Result<Self, PreviewRequestError> {
        if library_revision == 0 || attachment.deleted {
            return Err(PreviewRequestError::InvalidRequest);
        }
        Ok(Self {
            generation,
            library_revision,
            attachment,
            target,
            cancellation: PreviewCancellation::default(),
        })
    }

    pub fn generation(&self) -> PreviewGeneration {
        self.generation
    }

    pub fn library_revision(&self) -> u64 {
        self.library_revision
    }

    pub fn attachment_id(&self) -> AttachmentId {
        self.attachment.id
    }

    pub fn target(&self) -> PreviewSize {
        self.target
    }

    pub fn cancellation(&self) -> &PreviewCancellation {
        &self.cancellation
    }
}

impl fmt::Debug for PreviewRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreviewRequest")
            .field("generation", &self.generation)
            .field("library_revision", &self.library_revision)
            .field("attachment_id", &self.attachment.id)
            .field("attachment_revision", &self.attachment.revision)
            .field("target", &self.target)
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewRequestError {
    InvalidRequest,
    GenerationExhausted,
}

impl fmt::Display for PreviewRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "the Notes preview request is invalid",
            Self::GenerationExhausted => "the Notes preview generation is exhausted",
        })
    }
}

impl std::error::Error for PreviewRequestError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewState {
    Empty,
    Loading {
        generation: PreviewGeneration,
        library_revision: u64,
        attachment_id: AttachmentId,
    },
    Ready {
        generation: PreviewGeneration,
        library_revision: u64,
        image: Arc<DecodedImagePreview>,
    },
    Unavailable {
        generation: PreviewGeneration,
        library_revision: u64,
        attachment_id: AttachmentId,
        error: PreviewError,
    },
}

pub struct NotesPreviewSession {
    next_generation: u64,
    active: Option<PreviewCancellation>,
    state: PreviewState,
}

impl NotesPreviewSession {
    pub fn new() -> Self {
        Self {
            next_generation: 1,
            active: None,
            state: PreviewState::Empty,
        }
    }

    pub fn state(&self) -> &PreviewState {
        &self.state
    }

    pub fn request(
        &mut self,
        library_revision: u64,
        attachment: AttachmentRecord,
        target: PreviewSize,
    ) -> Result<PreviewRequest, PreviewRequestError> {
        let generation = PreviewGeneration::new(self.next_generation)
            .ok_or(PreviewRequestError::GenerationExhausted)?;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .filter(|next| *next != 0)
            .ok_or(PreviewRequestError::GenerationExhausted)?;
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        let request = PreviewRequest::new(generation, library_revision, attachment, target)?;
        self.active = Some(request.cancellation.clone());
        self.state = PreviewState::Loading {
            generation,
            library_revision,
            attachment_id: request.attachment_id(),
        };
        Ok(request)
    }

    pub fn clear(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        self.state = PreviewState::Empty;
    }

    fn is_pending(
        &self,
        generation: PreviewGeneration,
        library_revision: u64,
        attachment_id: AttachmentId,
    ) -> bool {
        matches!(
            self.state,
            PreviewState::Loading {
                generation: pending_generation,
                library_revision: pending_revision,
                attachment_id: pending_attachment,
            } if pending_generation == generation
                && pending_revision == library_revision
                && pending_attachment == attachment_id
        )
    }

    fn ready(
        &mut self,
        generation: PreviewGeneration,
        library_revision: u64,
        image: DecodedImagePreview,
    ) -> bool {
        if !self.is_pending(generation, library_revision, image.attachment_id()) {
            return false;
        }
        self.active = None;
        self.state = PreviewState::Ready {
            generation,
            library_revision,
            image: Arc::new(image),
        };
        true
    }

    fn fail(
        &mut self,
        generation: PreviewGeneration,
        library_revision: u64,
        attachment_id: AttachmentId,
        error: PreviewError,
    ) -> bool {
        if !self.is_pending(generation, library_revision, attachment_id) {
            return false;
        }
        self.active = None;
        self.state = PreviewState::Unavailable {
            generation,
            library_revision,
            attachment_id,
            error,
        };
        true
    }
}

impl Default for NotesPreviewSession {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for NotesPreviewSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotesPreviewSession")
            .field("state", &self.state)
            .finish()
    }
}

enum PreviewWorkerCommand {
    Run(PreviewRequest),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewWorkerEvent {
    Started {
        generation: PreviewGeneration,
        library_revision: u64,
        attachment_id: AttachmentId,
    },
    Ready {
        generation: PreviewGeneration,
        library_revision: u64,
        image: DecodedImagePreview,
    },
    Failed {
        generation: PreviewGeneration,
        library_revision: u64,
        attachment_id: AttachmentId,
        error: PreviewError,
    },
    Stopped {
        jobs_started: u64,
    },
}

impl PreviewWorkerEvent {
    pub fn project(self, session: &mut NotesPreviewSession) -> bool {
        match self {
            Self::Started {
                generation,
                library_revision,
                attachment_id,
            } => session.is_pending(generation, library_revision, attachment_id),
            Self::Ready {
                generation,
                library_revision,
                image,
            } => session.ready(generation, library_revision, image),
            Self::Failed {
                generation,
                library_revision,
                attachment_id,
                error,
            } => session.fail(generation, library_revision, attachment_id, error),
            Self::Stopped { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewWorkerSendError {
    Full,
    Closed,
}

impl fmt::Display for PreviewWorkerSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Full => "the Notes preview queue is full",
            Self::Closed => "Notes preview is no longer available",
        })
    }
}

impl std::error::Error for PreviewWorkerSendError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewWorkerStartError {
    InvalidRoot,
    Thread(io::ErrorKind),
}

impl fmt::Display for PreviewWorkerStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Notes could not start its managed image preview worker")
    }
}

impl std::error::Error for PreviewWorkerStartError {}

pub struct NotesPreviewWorker {
    commands: Option<SyncSender<PreviewWorkerCommand>>,
    events: Option<Receiver<PreviewWorkerEvent>>,
    active: Arc<Mutex<Option<PreviewCancellation>>>,
    thread: Option<JoinHandle<()>>,
}

impl NotesPreviewWorker {
    pub fn start(root: PathBuf) -> Result<Self, PreviewWorkerStartError> {
        if !root.is_absolute()
            || root
                .components()
                .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        {
            return Err(PreviewWorkerStartError::InvalidRoot);
        }
        let (command_sender, command_receiver) = mpsc::sync_channel(PREVIEW_COMMAND_CAPACITY);
        let (event_sender, event_receiver) = mpsc::sync_channel(PREVIEW_EVENT_CAPACITY);
        let active = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let thread = thread::Builder::new()
            .name("rmac-notes-preview".into())
            .spawn(move || run_worker(root, command_receiver, event_sender, worker_active))
            .map_err(|error| PreviewWorkerStartError::Thread(error.kind()))?;
        Ok(Self {
            commands: Some(command_sender),
            events: Some(event_receiver),
            active,
            thread: Some(thread),
        })
    }

    pub fn try_run(&self, request: PreviewRequest) -> Result<(), PreviewWorkerSendError> {
        self.commands
            .as_ref()
            .ok_or(PreviewWorkerSendError::Closed)?
            .try_send(PreviewWorkerCommand::Run(request))
            .map_err(|error| match error {
                TrySendError::Full(_) => PreviewWorkerSendError::Full,
                TrySendError::Disconnected(_) => PreviewWorkerSendError::Closed,
            })
    }

    pub fn recv(&self) -> Result<PreviewWorkerEvent, RecvError> {
        self.events.as_ref().ok_or(RecvError)?.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<PreviewWorkerEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<PreviewWorkerEvent, TryRecvError> {
        self.events
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }
}

impl fmt::Debug for NotesPreviewWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("NotesPreviewWorker").finish()
    }
}

impl Drop for NotesPreviewWorker {
    fn drop(&mut self) {
        self.events.take();
        if let Some(cancellation) = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            cancellation.cancel();
        }
        if let Some(commands) = self.commands.take() {
            let _ = commands.try_send(PreviewWorkerCommand::Shutdown);
            drop(commands);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_worker(
    root: PathBuf,
    commands: Receiver<PreviewWorkerCommand>,
    events: SyncSender<PreviewWorkerEvent>,
    active: Arc<Mutex<Option<PreviewCancellation>>>,
) {
    let mut jobs_started = 0_u64;
    while let Ok(command) = commands.recv() {
        let PreviewWorkerCommand::Run(request) = command else {
            break;
        };
        if request.cancellation.is_cancelled() {
            continue;
        }
        jobs_started = jobs_started.saturating_add(1);
        let generation = request.generation;
        let library_revision = request.library_revision;
        let attachment_id = request.attachment.id;
        *active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.cancellation.clone());
        if events
            .send(PreviewWorkerEvent::Started {
                generation,
                library_revision,
                attachment_id,
            })
            .is_err()
        {
            break;
        }
        let result = load_managed_image_preview(&root, &request.attachment, request.target);
        active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if request.cancellation.is_cancelled() {
            continue;
        }
        let event = match result {
            Ok(image) => PreviewWorkerEvent::Ready {
                generation,
                library_revision,
                image,
            },
            Err(error) => PreviewWorkerEvent::Failed {
                generation,
                library_revision,
                attachment_id,
                error,
            },
        };
        if events.send(event).is_err() {
            break;
        }
    }
    let _ = events.send(PreviewWorkerEvent::Stopped { jobs_started });
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder as _;
    use rmac_notes_store::{AttachmentKind, NoteId};
    use sha2::{Digest as _, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-notes-preview-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[12, 34, 56, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    fn attachment(bytes: &[u8]) -> AttachmentRecord {
        AttachmentRecord {
            id: AttachmentId::new(1).unwrap(),
            revision: 1,
            note_id: NoteId::new(1).unwrap(),
            display_name: "private-preview.png".into(),
            kind: AttachmentKind::Png,
            byte_len: bytes.len() as u64,
            sha256: Sha256::digest(bytes).into(),
            deleted: false,
        }
    }

    fn install(root: &std::path::Path, bytes: &[u8]) {
        let directory = root.join("attachments");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("00000000000000000001.bin"), bytes).unwrap();
    }

    #[test]
    fn preview_worker_projects_only_exact_generation_revision_and_identity() {
        let root = root("project");
        let bytes = png();
        install(&root, &bytes);
        let worker = NotesPreviewWorker::start(root.clone()).unwrap();
        let mut session = NotesPreviewSession::new();
        let request = session
            .request(7, attachment(&bytes), PreviewSize::new(512, 512).unwrap())
            .unwrap();
        let debug = format!("{request:?}");
        assert!(!debug.contains("private-preview"));
        assert!(!debug.contains(root.to_string_lossy().as_ref()));
        worker.try_run(request).unwrap();

        assert!(worker
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .project(&mut session));
        assert!(worker
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .project(&mut session));
        let PreviewState::Ready { image, .. } = session.state() else {
            panic!("expected ready preview, got {:?}", session.state())
        };
        assert_eq!((image.width(), image.height()), (1, 1));
        assert_eq!(image.rgba().as_ref(), &[12, 34, 56, 255]);

        drop(worker);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_managed_bytes_become_unavailable_without_private_diagnostics() {
        let root = root("changed");
        let bytes = png();
        let record = attachment(&bytes);
        install(&root, b"changed bytes");
        let worker = NotesPreviewWorker::start(root.clone()).unwrap();
        let mut session = NotesPreviewSession::new();
        let request = session
            .request(7, record, PreviewSize::new(64, 64).unwrap())
            .unwrap();
        worker.try_run(request).unwrap();
        assert!(worker
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .project(&mut session));
        assert!(worker
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .project(&mut session));
        assert!(matches!(
            session.state(),
            PreviewState::Unavailable {
                error: PreviewError::Changed,
                ..
            }
        ));
        let debug = format!("{session:?}");
        assert!(!debug.contains("private-preview"));
        assert!(!debug.contains("changed bytes"));

        drop(worker);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn newer_request_cancels_and_rejects_late_preview_events() {
        let bytes = png();
        let mut session = NotesPreviewSession::new();
        let first = session
            .request(7, attachment(&bytes), PreviewSize::new(64, 64).unwrap())
            .unwrap();
        let second = session
            .request(8, attachment(&bytes), PreviewSize::new(128, 128).unwrap())
            .unwrap();
        assert!(first.cancellation().is_cancelled());
        assert!(!second.cancellation().is_cancelled());
        assert!(!PreviewWorkerEvent::Failed {
            generation: first.generation(),
            library_revision: first.library_revision(),
            attachment_id: first.attachment_id(),
            error: PreviewError::Decode,
        }
        .project(&mut session));
        assert!(matches!(
            session.state(),
            PreviewState::Loading {
                generation,
                library_revision: 8,
                ..
            } if *generation == second.generation()
        ));
        session.clear();
        assert!(second.cancellation().is_cancelled());
        assert_eq!(session.state(), &PreviewState::Empty);
    }

    #[test]
    fn worker_root_and_preview_target_are_strictly_bounded() {
        assert!(matches!(
            NotesPreviewWorker::start(PathBuf::from("relative")),
            Err(PreviewWorkerStartError::InvalidRoot)
        ));
        assert_eq!(PreviewSize::new(0, 10), Err(PreviewError::InvalidRequest));
    }
}
