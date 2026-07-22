//! Bounded off-UI icon execution for the future E2 layer-surface host.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rmac_notifications::{AppId, NotificationId};

use crate::icon::{ApplicationIcons, Outcome, Renderer, Request};
use crate::media::{Icon, MAX_ICON_BYTES, MAX_ICON_NAME_BYTES, MAX_THEMED_NAMES};

pub const ICON_COMMAND_CAPACITY: usize = 2;
pub const ICON_EVENT_CAPACITY: usize = 2;
pub const MAX_ICON_BATCH_ITEMS: usize = rmac_notifications_runtime::presentation::MAX_CARDS;
pub const MAX_ICON_BATCH_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ICON_BATCH_RGBA_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Generation(u64);

impl Generation {
    pub fn new(value: u64) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Key {
    notification: NotificationId,
    logical_edge: u16,
    pixel_edge: u32,
}

impl Key {
    pub fn notification(&self) -> NotificationId {
        self.notification
    }

    pub fn logical_edge(&self) -> u16 {
        self.logical_edge
    }

    pub fn pixel_edge(&self) -> u32 {
        self.pixel_edge
    }
}

#[derive(Clone)]
pub struct Source {
    key: Key,
    app_id: String,
    portal: Option<Icon>,
    request: Request,
}

impl Source {
    pub fn new(
        notification: NotificationId,
        app_id: impl Into<String>,
        portal: Option<Icon>,
        request: Request,
    ) -> Result<Self, RequestError> {
        let app_id = app_id.into();
        AppId::parse(app_id.clone()).map_err(|_| RequestError::InvalidSource)?;
        if portal.as_ref().is_some_and(|icon| !valid_portal_icon(icon)) {
            return Err(RequestError::InvalidSource);
        }
        Ok(Self {
            key: Key {
                notification,
                logical_edge: request.logical_edge(),
                pixel_edge: request.pixel_edge(),
            },
            app_id,
            portal,
            request,
        })
    }

    pub fn key(&self) -> &Key {
        &self.key
    }
}

impl fmt::Debug for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Source")
            .field("key", &self.key)
            .field("app_id", &"<redacted>")
            .field("portal", &self.portal)
            .finish()
    }
}

fn valid_portal_icon(icon: &Icon) -> bool {
    match icon {
        Icon::Themed(names) => {
            !names.is_empty()
                && names.len() <= MAX_THEMED_NAMES
                && names.iter().all(|name| {
                    !name.is_empty()
                        && name.len() <= MAX_ICON_NAME_BYTES
                        && !name.chars().any(char::is_control)
                        && !name.contains(['/', '\\'])
                        && name != "."
                        && name != ".."
                })
        }
        Icon::File { bytes, .. } => !bytes.is_empty() && bytes.len() <= MAX_ICON_BYTES,
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
pub struct BatchRequest {
    generation: Generation,
    sources: Vec<Source>,
    cancellation: Cancellation,
}

impl BatchRequest {
    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }
}

impl fmt::Debug for BatchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BatchRequest")
            .field("generation", &self.generation)
            .field("sources", &self.sources)
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestError {
    InvalidSource,
    EmptyBatch,
    TooManyItems,
    TooManySourceBytes,
    TooManyOutputBytes,
    DuplicateKey,
    GenerationExhausted,
}

impl fmt::Display for RequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSource => "a notification icon source is invalid",
            Self::EmptyBatch => "the notification icon batch is empty",
            Self::TooManyItems => "the notification icon batch contains too many items",
            Self::TooManySourceBytes => "the notification icon sources exceed their byte limit",
            Self::TooManyOutputBytes => "the notification icon output exceeds its byte limit",
            Self::DuplicateKey => "the notification icon batch contains a duplicate key",
            Self::GenerationExhausted => "the notification icon generation is exhausted",
        })
    }
}

impl std::error::Error for RequestError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResultItem {
    pub key: Key,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchEvent {
    pub generation: Generation,
    pub results: Vec<ResultItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum State {
    Empty,
    Loading {
        generation: Generation,
        keys: Vec<Key>,
        previous_generation: Option<Generation>,
        previous: Vec<ResultItem>,
    },
    Ready {
        generation: Generation,
        results: Vec<ResultItem>,
    },
}

pub struct Session {
    next_generation: u64,
    active: Option<Cancellation>,
    state: State,
}

impl Session {
    pub fn new() -> Self {
        Self {
            next_generation: 1,
            active: None,
            state: State::Empty,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Last accepted pixels remain visible while a replacement resolves.
    pub fn visible_results(&self) -> &[ResultItem] {
        match &self.state {
            State::Loading { previous, .. } => previous,
            State::Ready { results, .. } => results,
            State::Empty => &[],
        }
    }

    pub fn request(&mut self, sources: Vec<Source>) -> Result<BatchRequest, RequestError> {
        validate_sources(&sources)?;
        let generation =
            Generation::new(self.next_generation).ok_or(RequestError::GenerationExhausted)?;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .filter(|next| *next != 0)
            .ok_or(RequestError::GenerationExhausted)?;
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        let keys = sources
            .iter()
            .map(|source| source.key.clone())
            .collect::<Vec<_>>();
        let requested = keys.iter().collect::<HashSet<_>>();
        let (previous_generation, previous) = match &self.state {
            State::Ready {
                generation,
                results,
            } => (Some(*generation), results.clone()),
            State::Loading {
                previous_generation,
                previous,
                ..
            } => (*previous_generation, previous.clone()),
            State::Empty => (None, Vec::new()),
        };
        let previous = previous
            .into_iter()
            .filter(|result| requested.contains(&result.key))
            .collect::<Vec<_>>();
        let previous_generation = if previous.is_empty() {
            None
        } else {
            previous_generation
        };
        let cancellation = Cancellation::default();
        self.active = Some(cancellation.clone());
        self.state = State::Loading {
            generation,
            keys,
            previous_generation,
            previous,
        };
        Ok(BatchRequest {
            generation,
            sources,
            cancellation,
        })
    }

    pub fn cancel(&mut self, generation: Generation) -> bool {
        if !matches!(
            self.state,
            State::Loading {
                generation: current,
                ..
            } if current == generation
        ) {
            return false;
        }
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        let previous = match std::mem::replace(&mut self.state, State::Empty) {
            State::Loading {
                previous_generation: Some(previous_generation),
                previous,
                ..
            } if !previous.is_empty() => Some((previous_generation, previous)),
            _ => None,
        };
        if let Some((generation, results)) = previous {
            self.state = State::Ready {
                generation,
                results,
            };
        }
        true
    }

    pub fn clear(&mut self) {
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        self.state = State::Empty;
    }

    /// Apply only the exact current generation, key order, and cardinality.
    pub fn apply(&mut self, event: BatchEvent) -> bool {
        let State::Loading {
            generation, keys, ..
        } = &self.state
        else {
            return false;
        };
        if *generation != event.generation
            || keys.len() != event.results.len()
            || !keys.iter().zip(&event.results).all(|(expected, result)| {
                expected == &result.key && valid_outcome(expected, &result.outcome)
            })
        {
            return false;
        }
        self.active = None;
        self.state = State::Ready {
            generation: event.generation,
            results: event.results,
        };
        true
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("state", &self.state)
            .finish()
    }
}

fn valid_outcome(key: &Key, outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Ready(ready) => {
            ready.logical_edge() == key.logical_edge && ready.pixel_edge() == key.pixel_edge
        }
        Outcome::Fallback(_) => true,
    }
}

fn validate_sources(sources: &[Source]) -> Result<(), RequestError> {
    if sources.is_empty() {
        return Err(RequestError::EmptyBatch);
    }
    if sources.len() > MAX_ICON_BATCH_ITEMS {
        return Err(RequestError::TooManyItems);
    }
    let mut source_bytes = 0_usize;
    let mut output_bytes = 0_u64;
    let mut keys = HashSet::with_capacity(sources.len());
    for source in sources {
        if let Some(Icon::File { bytes, .. }) = &source.portal {
            source_bytes = source_bytes
                .checked_add(bytes.len())
                .ok_or(RequestError::TooManySourceBytes)?;
            if source_bytes > MAX_ICON_BATCH_SOURCE_BYTES {
                return Err(RequestError::TooManySourceBytes);
            }
        }
        let edge = u64::from(source.request.pixel_edge());
        output_bytes = output_bytes
            .checked_add(edge * edge * 4)
            .ok_or(RequestError::TooManyOutputBytes)?;
        if output_bytes > MAX_ICON_BATCH_RGBA_BYTES {
            return Err(RequestError::TooManyOutputBytes);
        }
        if !keys.insert(&source.key) {
            return Err(RequestError::DuplicateKey);
        }
    }
    Ok(())
}

enum Command {
    Run(BatchRequest),
    ReplaceApplications(ApplicationIcons),
    RefreshTheme,
    RetainNotifications(BTreeSet<NotificationId>),
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendError {
    Full,
    Closed,
}

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Full => "the notification icon worker queue is full",
            Self::Closed => "the notification icon worker is unavailable",
        })
    }
}

impl std::error::Error for SendError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartError(pub io::ErrorKind);

impl fmt::Display for StartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the notification icon worker could not start")
    }
}

impl std::error::Error for StartError {}

pub struct Worker {
    commands: Option<SyncSender<Command>>,
    events: Option<Receiver<BatchEvent>>,
    active: Arc<Mutex<Option<Cancellation>>>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct WorkerClient {
    commands: SyncSender<Command>,
}

impl WorkerClient {
    pub fn try_run(&self, request: BatchRequest) -> Result<(), SendError> {
        try_send(&self.commands, Command::Run(request))
    }

    pub fn try_replace_applications(
        &self,
        applications: ApplicationIcons,
    ) -> Result<(), SendError> {
        try_send(&self.commands, Command::ReplaceApplications(applications))
    }

    pub fn try_refresh_theme(&self) -> Result<(), SendError> {
        try_send(&self.commands, Command::RefreshTheme)
    }

    pub fn try_retain_notifications(
        &self,
        notifications: BTreeSet<NotificationId>,
    ) -> Result<(), SendError> {
        try_send(&self.commands, Command::RetainNotifications(notifications))
    }

    pub fn try_shutdown(&self) -> Result<(), SendError> {
        try_send(&self.commands, Command::Shutdown)
    }
}

impl fmt::Debug for WorkerClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("WorkerClient").finish()
    }
}

pub struct WorkerEvents {
    events: Option<Receiver<BatchEvent>>,
    shutdown: Option<SyncSender<Command>>,
    active: Arc<Mutex<Option<Cancellation>>>,
    thread: Option<JoinHandle<()>>,
}

impl WorkerEvents {
    pub fn recv(&self) -> Result<BatchEvent, RecvError> {
        self.events.as_ref().ok_or(RecvError)?.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<BatchEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<BatchEvent, TryRecvError> {
        self.events
            .as_ref()
            .ok_or(TryRecvError::Disconnected)?
            .try_recv()
    }
}

impl fmt::Debug for WorkerEvents {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("WorkerEvents").finish()
    }
}

impl Drop for WorkerEvents {
    fn drop(&mut self) {
        self.events.take();
        cancel_active(&self.active);
        if let Some(shutdown) = self.shutdown.take() {
            // The result receiver is already closed, so the worker can always
            // drain a full command queue and admit this terminal command.
            let _ = shutdown.send(Command::Shutdown);
            drop(shutdown);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Worker {
    pub fn start(applications: ApplicationIcons, cache_bytes: usize) -> Result<Self, StartError> {
        let (command_sender, command_receiver) = mpsc::sync_channel(ICON_COMMAND_CAPACITY);
        let (event_sender, event_receiver) = mpsc::sync_channel(ICON_EVENT_CAPACITY);
        let active = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let thread = thread::Builder::new()
            .name("rmac-notification-icons".into())
            .spawn(move || {
                run_worker(
                    Renderer::with_budget(applications, cache_bytes),
                    command_receiver,
                    event_sender,
                    worker_active,
                );
            })
            .map_err(|error| StartError(error.kind()))?;
        Ok(Self {
            commands: Some(command_sender),
            events: Some(event_receiver),
            active,
            thread: Some(thread),
        })
    }

    pub fn try_run(&self, request: BatchRequest) -> Result<(), SendError> {
        try_send(
            self.commands.as_ref().ok_or(SendError::Closed)?,
            Command::Run(request),
        )
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<BatchEvent, RecvTimeoutError> {
        self.events
            .as_ref()
            .ok_or(RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn into_parts(mut self) -> (WorkerClient, WorkerEvents) {
        let commands = self
            .commands
            .take()
            .expect("a live notification icon worker owns its command endpoint");
        let events = self
            .events
            .take()
            .expect("a live notification icon worker owns its event endpoint");
        let thread = self
            .thread
            .take()
            .expect("a live notification icon worker owns its thread");
        (
            WorkerClient {
                commands: commands.clone(),
            },
            WorkerEvents {
                events: Some(events),
                shutdown: Some(commands),
                active: self.active.clone(),
                thread: Some(thread),
            },
        )
    }
}

impl fmt::Debug for Worker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Worker").finish()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.events.take();
        cancel_active(&self.active);
        if let Some(commands) = self.commands.take() {
            // Closing results first prevents a full event queue from blocking
            // the worker while this waits for command admission.
            let _ = commands.send(Command::Shutdown);
            drop(commands);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn try_send(commands: &SyncSender<Command>, command: Command) -> Result<(), SendError> {
    commands.try_send(command).map_err(|error| match error {
        TrySendError::Full(_) => SendError::Full,
        TrySendError::Disconnected(_) => SendError::Closed,
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
    renderer: Renderer,
    commands: Receiver<Command>,
    events: SyncSender<BatchEvent>,
    active: Arc<Mutex<Option<Cancellation>>>,
) {
    while let Ok(command) = commands.recv() {
        match command {
            Command::Run(request) => {
                if request.cancellation.is_cancelled() {
                    continue;
                }
                *active
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some(request.cancellation.clone());
                renderer.retain_notifications(
                    &request
                        .sources
                        .iter()
                        .map(|source| source.key.notification)
                        .collect(),
                );
                let mut results = Vec::with_capacity(request.sources.len());
                for source in &request.sources {
                    if request.cancellation.is_cancelled() {
                        break;
                    }
                    let outcome = renderer.render(
                        source.key.notification,
                        &source.app_id,
                        source.portal.as_ref(),
                        source.request,
                    );
                    results.push(ResultItem {
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
                    .send(BatchEvent {
                        generation: request.generation,
                        results,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Command::ReplaceApplications(applications) => {
                renderer.replace_applications(applications);
            }
            Command::RefreshTheme => renderer.refresh_theme(),
            Command::RetainNotifications(notifications) => {
                renderer.retain_notifications(&notifications);
            }
            Command::Shutdown => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use image::ImageEncoder as _;

    use super::*;
    use crate::icon::FallbackReason;
    use crate::media::IconFormat;

    fn id(value: u32) -> NotificationId {
        NotificationId::from_protocol(value).unwrap()
    }

    fn request(logical: u16, scale: f64) -> Request {
        Request::new(logical, scale).unwrap()
    }

    fn png(pixel: [u8; 4]) -> Arc<[u8]> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixel, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes.into()
    }

    fn source(value: u32, logical: u16, scale: f64, icon: Option<Icon>) -> Source {
        Source::new(
            id(value),
            "org.example.Private",
            icon,
            request(logical, scale),
        )
        .unwrap()
    }

    #[test]
    fn source_and_batch_limits_fail_before_worker_admission() {
        assert_eq!(
            Source::new(id(1), "bad\napp", None, request(32, 1.0)).unwrap_err(),
            RequestError::InvalidSource
        );
        assert_eq!(
            Source::new(
                id(1),
                "org.example.Private",
                Some(Icon::Themed(vec!["icon".into(); MAX_THEMED_NAMES + 1])),
                request(32, 1.0),
            )
            .unwrap_err(),
            RequestError::InvalidSource
        );
        let mut session = Session::new();
        assert_eq!(
            session.request(Vec::new()).unwrap_err(),
            RequestError::EmptyBatch
        );
        let duplicate = source(1, 32, 1.0, None);
        assert_eq!(
            session
                .request(vec![duplicate.clone(), duplicate])
                .unwrap_err(),
            RequestError::DuplicateKey
        );
        let too_many = (1..=MAX_ICON_BATCH_ITEMS + 1)
            .map(|value| source(value as u32, 16, 1.0, None))
            .collect();
        assert_eq!(
            session.request(too_many).unwrap_err(),
            RequestError::TooManyItems
        );
        let oversized = (1..=33)
            .map(|value| source(value, 128, 4.0, None))
            .collect();
        assert_eq!(
            session.request(oversized).unwrap_err(),
            RequestError::TooManyOutputBytes
        );
        let maximum_source: Arc<[u8]> = vec![0; MAX_ICON_BYTES].into();
        let too_many_source_bytes = (1..=5)
            .map(|value| {
                source(
                    value,
                    16,
                    1.0,
                    Some(Icon::File {
                        format: IconFormat::Png,
                        bytes: maximum_source.clone(),
                    }),
                )
            })
            .collect();
        assert_eq!(
            session.request(too_many_source_bytes).unwrap_err(),
            RequestError::TooManySourceBytes
        );
    }

    #[test]
    fn worker_returns_exact_generation_and_private_renderer_results() {
        let icon = Icon::File {
            format: IconFormat::Png,
            bytes: png([12, 34, 56, 255]),
        };
        let mut session = Session::new();
        let request = session
            .request(vec![source(1, 40, 1.25, Some(icon))])
            .unwrap();
        let worker = Worker::start(ApplicationIcons::default(), 1024 * 1024).unwrap();
        worker.try_run(request).unwrap();
        let event = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        let diagnostics = format!("{event:?}");
        assert!(!diagnostics.contains("org.example.Private"));
        assert!(!diagnostics.contains("12, 34, 56"));
        assert!(matches!(
            event.results[0].outcome,
            Outcome::Ready(ref ready)
                if ready.logical_edge() == 40 && ready.pixel_edge() == 50
        ));
        assert!(session.apply(event));
        assert!(matches!(session.state(), State::Ready { .. }));
    }

    #[test]
    fn cancelled_generation_never_publishes_over_latest() {
        let mut session = Session::new();
        let stale = session
            .request(vec![source(
                1,
                32,
                1.0,
                Some(Icon::File {
                    format: IconFormat::Png,
                    bytes: png([1, 2, 3, 255]),
                }),
            )])
            .unwrap();
        let latest = session
            .request(vec![source(
                2,
                64,
                1.0,
                Some(Icon::File {
                    format: IconFormat::Png,
                    bytes: png([4, 5, 6, 255]),
                }),
            )])
            .unwrap();
        let latest_generation = latest.generation();
        let worker = Worker::start(ApplicationIcons::default(), 1024 * 1024).unwrap();
        worker.try_run(stale).unwrap();
        worker.try_run(latest).unwrap();
        let event = worker.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event.generation, latest_generation);
        assert_eq!(event.results[0].key.notification(), id(2));
        assert!(session.apply(event));
    }

    #[test]
    fn session_rejects_wrong_pixels_and_restores_previous_on_cancel() {
        let mut session = Session::new();
        let first = session.request(vec![source(1, 32, 1.0, None)]).unwrap();
        let key = first.sources()[0].key().clone();
        assert!(session.apply(BatchEvent {
            generation: first.generation(),
            results: vec![ResultItem {
                key: key.clone(),
                outcome: Outcome::Fallback(FallbackReason::NoSource),
            }],
        }));
        let refresh = session.request(vec![source(1, 32, 1.0, None)]).unwrap();
        assert_eq!(session.visible_results().len(), 1);
        let renderer = Renderer::new(ApplicationIcons::default());
        let wrong_icon = Icon::File {
            format: IconFormat::Png,
            bytes: png([1, 2, 3, 255]),
        };
        let Outcome::Ready(wrong_ready) = renderer.render(
            id(1),
            "org.example.Private",
            Some(&wrong_icon),
            request(64, 1.0),
        ) else {
            panic!("test icon must decode");
        };
        assert!(!session.apply(BatchEvent {
            generation: refresh.generation(),
            results: vec![ResultItem {
                key,
                outcome: Outcome::Ready(wrong_ready),
            }],
        }));
        assert!(session.cancel(refresh.generation()));
        assert!(matches!(session.state(), State::Ready { .. }));
    }

    #[test]
    fn command_backpressure_and_disconnect_are_explicit() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.try_send(Command::RefreshTheme).unwrap();
        assert_eq!(
            try_send(&sender, Command::RefreshTheme),
            Err(SendError::Full)
        );
        drop(receiver);
        assert_eq!(
            try_send(&sender, Command::RefreshTheme),
            Err(SendError::Closed)
        );
    }
}
