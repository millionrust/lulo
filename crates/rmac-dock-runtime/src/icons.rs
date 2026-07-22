//! Bounded, generation-safe icon work for a future Dock surface.

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

pub const ICON_COMMAND_CAPACITY: usize = 2;
pub const ICON_EVENT_CAPACITY: usize = 2;
pub const MAX_ICON_BATCH_ITEMS: usize = 512;
pub const MAX_ICON_BATCH_RGBA_BYTES: u64 = 32 * 1024 * 1024;
const MAX_APPLICATION_ID_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IconGeneration(u64);

impl IconGeneration {
    pub fn new(value: u64) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconKey {
    application_id: String,
    edge: u32,
}

impl IconKey {
    pub fn application_id(&self) -> &str {
        &self.application_id
    }

    pub fn edge(&self) -> u32 {
        self.edge
    }
}

#[derive(Clone)]
pub struct IconSource {
    key: IconKey,
    path: PathBuf,
    request: rmac_dock_system::icons::DecodeRequest,
}

impl IconSource {
    pub fn new(
        application_id: impl Into<String>,
        path: PathBuf,
        edge: u32,
    ) -> Result<Self, IconRequestError> {
        let application_id = application_id.into();
        if application_id.is_empty()
            || application_id.len() > MAX_APPLICATION_ID_BYTES
            || application_id.chars().any(char::is_control)
            || !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return Err(IconRequestError::InvalidSource);
        }
        let request = rmac_dock_system::icons::DecodeRequest::new(edge)
            .map_err(|_| IconRequestError::InvalidSource)?;
        Ok(Self {
            key: IconKey {
                application_id,
                edge,
            },
            path,
            request,
        })
    }

    pub fn key(&self) -> &IconKey {
        &self.key
    }
}

impl fmt::Debug for IconSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IconSource")
            .field("key", &self.key)
            .field("path", &"<private>")
            .finish()
    }
}

#[derive(Clone, Default)]
struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl fmt::Debug for Cancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone)]
pub struct IconBatchRequest {
    generation: IconGeneration,
    sources: Vec<IconSource>,
    cancellation: Cancellation,
}

impl IconBatchRequest {
    pub fn generation(&self) -> IconGeneration {
        self.generation
    }

    pub fn sources(&self) -> &[IconSource] {
        &self.sources
    }
}

impl fmt::Debug for IconBatchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IconBatchRequest")
            .field("generation", &self.generation)
            .field("sources", &self.sources)
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconRequestError {
    InvalidSource,
    EmptyBatch,
    TooManyItems,
    TooManyOutputBytes,
    DuplicateKey,
    GenerationExhausted,
}

impl fmt::Display for IconRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSource => "an icon source is invalid",
            Self::EmptyBatch => "the icon batch is empty",
            Self::TooManyItems => "the icon batch contains too many items",
            Self::TooManyOutputBytes => "the icon batch output exceeds its memory limit",
            Self::DuplicateKey => "the icon batch contains a duplicate key",
            Self::GenerationExhausted => "the icon generation is exhausted",
        })
    }
}

impl std::error::Error for IconRequestError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IconOutcome {
    Ready(Arc<rmac_dock_system::icons::DecodedIcon>),
    Fallback(rmac_dock_system::icons::ErrorKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconResult {
    pub key: IconKey,
    pub outcome: IconOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconBatchEvent {
    pub generation: IconGeneration,
    pub results: Vec<IconResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IconState {
    Empty,
    Loading {
        generation: IconGeneration,
        keys: Vec<IconKey>,
    },
    Ready {
        generation: IconGeneration,
        results: Vec<IconResult>,
    },
}

pub struct IconSession {
    next_generation: u64,
    active: Option<Cancellation>,
    state: IconState,
}

impl IconSession {
    pub fn new() -> Self {
        Self {
            next_generation: 1,
            active: None,
            state: IconState::Empty,
        }
    }

    pub fn state(&self) -> &IconState {
        &self.state
    }

    pub fn request(
        &mut self,
        sources: Vec<IconSource>,
    ) -> Result<IconBatchRequest, IconRequestError> {
        validate_sources(&sources)?;
        let generation = IconGeneration::new(self.next_generation)
            .ok_or(IconRequestError::GenerationExhausted)?;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .filter(|next| *next != 0)
            .ok_or(IconRequestError::GenerationExhausted)?;
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        let cancellation = Cancellation::default();
        self.active = Some(cancellation.clone());
        self.state = IconState::Loading {
            generation,
            keys: sources.iter().map(|source| source.key.clone()).collect(),
        };
        Ok(IconBatchRequest {
            generation,
            sources,
            cancellation,
        })
    }

    pub fn cancel(&mut self, generation: IconGeneration) -> bool {
        if !matches!(
            self.state,
            IconState::Loading {
                generation: current,
                ..
            } if current == generation
        ) {
            return false;
        }
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        self.state = IconState::Empty;
        true
    }

    pub fn clear(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        self.state = IconState::Empty;
    }

    /// Apply only the exact requested generation, key order, and cardinality.
    pub fn apply(&mut self, event: IconBatchEvent) -> bool {
        let IconState::Loading { generation, keys } = &self.state else {
            return false;
        };
        if *generation != event.generation
            || keys.len() != event.results.len()
            || !keys.iter().zip(&event.results).all(|(expected, result)| {
                expected == &result.key
                    && match &result.outcome {
                        IconOutcome::Ready(icon) => icon.edge() == result.key.edge,
                        IconOutcome::Fallback(_) => true,
                    }
            })
        {
            return false;
        }
        self.active = None;
        self.state = IconState::Ready {
            generation: event.generation,
            results: event.results,
        };
        true
    }
}

impl Default for IconSession {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for IconSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IconSession")
            .field("state", &self.state)
            .finish()
    }
}

fn validate_sources(sources: &[IconSource]) -> Result<(), IconRequestError> {
    if sources.is_empty() {
        return Err(IconRequestError::EmptyBatch);
    }
    if sources.len() > MAX_ICON_BATCH_ITEMS {
        return Err(IconRequestError::TooManyItems);
    }
    let mut output_bytes = 0_u64;
    for (index, source) in sources.iter().enumerate() {
        let edge = u64::from(source.request.edge());
        output_bytes = output_bytes
            .checked_add(edge * edge * 4)
            .ok_or(IconRequestError::TooManyOutputBytes)?;
        if output_bytes > MAX_ICON_BATCH_RGBA_BYTES {
            return Err(IconRequestError::TooManyOutputBytes);
        }
        if sources[..index]
            .iter()
            .any(|candidate| candidate.key == source.key)
        {
            return Err(IconRequestError::DuplicateKey);
        }
    }
    Ok(())
}

enum IconWorkerCommand {
    Run(IconBatchRequest),
    Invalidate(PathBuf),
    ClearCache,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconWorkerSendError {
    Full,
    Closed,
}

impl fmt::Display for IconWorkerSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Full => "the Dock icon worker queue is full",
            Self::Closed => "the Dock icon worker is unavailable",
        })
    }
}

impl std::error::Error for IconWorkerSendError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IconWorkerStartError(pub io::ErrorKind);

impl fmt::Display for IconWorkerStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the Dock icon worker could not start")
    }
}

impl std::error::Error for IconWorkerStartError {}

pub struct IconWorker {
    commands: Option<SyncSender<IconWorkerCommand>>,
    events: Option<Receiver<IconBatchEvent>>,
    active: Arc<Mutex<Option<Cancellation>>>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct IconWorkerClient {
    commands: SyncSender<IconWorkerCommand>,
}

impl IconWorkerClient {
    pub fn try_run(&self, request: IconBatchRequest) -> Result<(), IconWorkerSendError> {
        try_send(&self.commands, IconWorkerCommand::Run(request))
    }

    pub fn try_invalidate(&self, path: PathBuf) -> Result<(), IconWorkerSendError> {
        try_send(&self.commands, IconWorkerCommand::Invalidate(path))
    }

    pub fn try_clear_cache(&self) -> Result<(), IconWorkerSendError> {
        try_send(&self.commands, IconWorkerCommand::ClearCache)
    }

    pub fn try_shutdown(&self) -> Result<(), IconWorkerSendError> {
        try_send(&self.commands, IconWorkerCommand::Shutdown)
    }
}

impl fmt::Debug for IconWorkerClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("IconWorkerClient").finish()
    }
}

pub struct IconWorkerEvents {
    events: Option<Receiver<IconBatchEvent>>,
    shutdown: Option<SyncSender<IconWorkerCommand>>,
    active: Arc<Mutex<Option<Cancellation>>>,
    thread: Option<JoinHandle<()>>,
}

impl IconWorkerEvents {
    pub fn recv(&self) -> Result<IconBatchEvent, RecvError> {
        self.events.as_ref().ok_or(RecvError)?.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<IconBatchEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<IconBatchEvent, TryRecvError> {
        self.events
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }
}

impl fmt::Debug for IconWorkerEvents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("IconWorkerEvents").finish()
    }
}

impl Drop for IconWorkerEvents {
    fn drop(&mut self) {
        self.events.take();
        cancel_active(&self.active);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.try_send(IconWorkerCommand::Shutdown);
            drop(shutdown);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl IconWorker {
    pub fn start(cache_bytes: usize) -> Result<Self, IconWorkerStartError> {
        let (command_sender, command_receiver) = mpsc::sync_channel(ICON_COMMAND_CAPACITY);
        let (event_sender, event_receiver) = mpsc::sync_channel(ICON_EVENT_CAPACITY);
        let active = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let thread = thread::Builder::new()
            .name("rmac-dock-icons".into())
            .spawn(move || {
                run_worker(
                    rmac_dock_system::icons::Cache::new(cache_bytes),
                    command_receiver,
                    event_sender,
                    worker_active,
                );
            })
            .map_err(|error| IconWorkerStartError(error.kind()))?;
        Ok(Self {
            commands: Some(command_sender),
            events: Some(event_receiver),
            active,
            thread: Some(thread),
        })
    }

    pub fn try_run(&self, request: IconBatchRequest) -> Result<(), IconWorkerSendError> {
        try_send(
            self.commands.as_ref().ok_or(IconWorkerSendError::Closed)?,
            IconWorkerCommand::Run(request),
        )
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<IconBatchEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn into_parts(mut self) -> (IconWorkerClient, IconWorkerEvents) {
        let commands = self
            .commands
            .take()
            .expect("a live Dock icon worker owns its command endpoint");
        let events = self
            .events
            .take()
            .expect("a live Dock icon worker owns its event endpoint");
        let thread = self
            .thread
            .take()
            .expect("a live Dock icon worker owns its thread");
        (
            IconWorkerClient {
                commands: commands.clone(),
            },
            IconWorkerEvents {
                events: Some(events),
                shutdown: Some(commands),
                active: self.active.clone(),
                thread: Some(thread),
            },
        )
    }
}

impl fmt::Debug for IconWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("IconWorker").finish()
    }
}

impl Drop for IconWorker {
    fn drop(&mut self) {
        self.events.take();
        cancel_active(&self.active);
        if let Some(commands) = self.commands.take() {
            let _ = commands.try_send(IconWorkerCommand::Shutdown);
            drop(commands);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn try_send(
    commands: &SyncSender<IconWorkerCommand>,
    command: IconWorkerCommand,
) -> Result<(), IconWorkerSendError> {
    commands.try_send(command).map_err(|error| match error {
        TrySendError::Full(_) => IconWorkerSendError::Full,
        TrySendError::Disconnected(_) => IconWorkerSendError::Closed,
    })
}

fn cancel_active(active: &Mutex<Option<Cancellation>>) {
    if let Some(cancellation) = active
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        cancellation.cancel();
    }
}

fn run_worker(
    cache: rmac_dock_system::icons::Cache,
    commands: Receiver<IconWorkerCommand>,
    events: SyncSender<IconBatchEvent>,
    active: Arc<Mutex<Option<Cancellation>>>,
) {
    while let Ok(command) = commands.recv() {
        match command {
            IconWorkerCommand::Run(request) => {
                if request.cancellation.is_cancelled() {
                    continue;
                }
                *active
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some(request.cancellation.clone());
                let mut results = Vec::with_capacity(request.sources.len());
                for source in &request.sources {
                    if request.cancellation.is_cancelled() {
                        break;
                    }
                    let outcome = match cache.get_or_decode(&source.path, source.request) {
                        Ok(icon) => IconOutcome::Ready(icon),
                        Err(error) => IconOutcome::Fallback(error.kind()),
                    };
                    results.push(IconResult {
                        key: source.key.clone(),
                        outcome,
                    });
                }
                active
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                if request.cancellation.is_cancelled() {
                    continue;
                }
                if events
                    .send(IconBatchEvent {
                        generation: request.generation,
                        results,
                    })
                    .is_err()
                {
                    break;
                }
            }
            IconWorkerCommand::Invalidate(path) => cache.invalidate_path(&path),
            IconWorkerCommand::ClearCache => cache.clear(),
            IconWorkerCommand::Shutdown => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use image::ImageEncoder as _;

    use super::*;

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rmac-dock-icon-worker-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_png(path: &Path, pixel: [u8; 4]) {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixel, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn source(id: &str, path: &Path, edge: u32) -> IconSource {
        IconSource::new(id, path.to_path_buf(), edge).unwrap()
    }

    #[test]
    fn source_and_batch_limits_fail_before_worker_admission() {
        assert_eq!(
            IconSource::new("demo", PathBuf::from("relative.png"), 64).unwrap_err(),
            IconRequestError::InvalidSource
        );
        let root = root("limits");
        let path = root.join("icon.png");
        write_png(&path, [1, 2, 3, 255]);
        let mut session = IconSession::new();
        assert_eq!(
            session.request(Vec::new()).unwrap_err(),
            IconRequestError::EmptyBatch
        );
        assert_eq!(
            session
                .request(vec![source("demo", &path, 64), source("demo", &path, 64)])
                .unwrap_err(),
            IconRequestError::DuplicateKey
        );
        let oversized = (0..33)
            .map(|index| source(&format!("app-{index}"), &path, 512))
            .collect();
        assert_eq!(
            session.request(oversized).unwrap_err(),
            IconRequestError::TooManyOutputBytes
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_returns_exact_generation_and_safe_fallbacks() {
        let root = root("result");
        let valid = root.join("valid-private.png");
        let invalid = root.join("invalid-private.xpm");
        write_png(&valid, [12, 34, 56, 255]);
        std::fs::write(&invalid, b"/* XPM */").unwrap();
        let mut session = IconSession::new();
        let request = session
            .request(vec![
                source("valid.desktop", &valid, 32),
                source("invalid.desktop", &invalid, 32),
            ])
            .unwrap();
        let worker = IconWorker::start(1024 * 1024).unwrap();
        worker.try_run(request).unwrap();
        let event = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        let debug = format!("{event:?}");
        assert!(!debug.contains("valid-private"));
        assert!(!debug.contains("invalid-private"));
        assert!(matches!(event.results[0].outcome, IconOutcome::Ready(_)));
        assert!(matches!(
            event.results[1].outcome,
            IconOutcome::Fallback(rmac_dock_system::icons::ErrorKind::Malformed)
        ));
        assert!(session.apply(event));
        assert!(matches!(session.state(), IconState::Ready { .. }));
        drop(worker);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancelled_stale_generation_never_publishes_over_the_latest() {
        let root = root("stale");
        let path = root.join("icon.png");
        write_png(&path, [90, 80, 70, 255]);
        let mut session = IconSession::new();
        let stale = session
            .request(vec![source("stale.desktop", &path, 32)])
            .unwrap();
        let latest = session
            .request(vec![source("latest.desktop", &path, 64)])
            .unwrap();
        let latest_generation = latest.generation();
        let worker = IconWorker::start(1024 * 1024).unwrap();
        worker.try_run(stale).unwrap();
        worker.try_run(latest).unwrap();
        let event = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event.generation, latest_generation);
        assert_eq!(event.results[0].key.application_id(), "latest.desktop");
        assert!(session.apply(event));
        assert!(matches!(
            session.state(),
            IconState::Ready { generation, .. } if *generation == latest_generation
        ));
        drop(worker);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_rejects_wrong_or_malformed_completion() {
        let root = root("revalidation");
        let path = root.join("icon.png");
        write_png(&path, [1, 2, 3, 255]);
        let mut session = IconSession::new();
        let request = session
            .request(vec![source("expected.desktop", &path, 32)])
            .unwrap();
        let wrong = IconBatchEvent {
            generation: request.generation(),
            results: vec![IconResult {
                key: IconKey {
                    application_id: "other.desktop".into(),
                    edge: 32,
                },
                outcome: IconOutcome::Fallback(rmac_dock_system::icons::ErrorKind::Unsupported),
            }],
        };
        assert!(!session.apply(wrong));
        assert!(matches!(session.state(), IconState::Loading { .. }));
        assert!(session.cancel(request.generation()));
        assert!(matches!(session.state(), IconState::Empty));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn split_endpoints_shutdown_and_join_without_path_diagnostics() {
        let worker = IconWorker::start(0).unwrap();
        let (client, events) = worker.into_parts();
        assert_eq!(format!("{client:?}"), "IconWorkerClient");
        assert_eq!(format!("{events:?}"), "IconWorkerEvents");
        client.try_clear_cache().unwrap();
        drop(events);
        assert_eq!(client.try_clear_cache(), Err(IconWorkerSendError::Closed));
    }

    #[test]
    fn command_backpressure_is_nonblocking_and_typed() {
        let (sender, receiver) = mpsc::sync_channel(1);
        try_send(&sender, IconWorkerCommand::ClearCache).unwrap();
        assert_eq!(
            try_send(&sender, IconWorkerCommand::ClearCache),
            Err(IconWorkerSendError::Full)
        );
        drop(receiver);
        assert_eq!(
            try_send(&sender, IconWorkerCommand::ClearCache),
            Err(IconWorkerSendError::Closed)
        );
    }
}
