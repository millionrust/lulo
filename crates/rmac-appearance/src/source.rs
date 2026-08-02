use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_channel::Sender;

use crate::{Event, Snapshot};

pub type SourceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    operation: String,
    detail: String,
}

impl Error {
    pub fn new(operation: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

/// Asynchronous appearance source used by apps and shell surfaces.
pub trait AppearanceSource: Send + Sync {
    fn snapshot(&self) -> SourceFuture<'_, Snapshot>;

    /// Forward source events until the consumer closes `events` or a terminal
    /// source error occurs. Reconnectable adapters should stay in this future
    /// and publish [`Event::Unavailable`] between successful snapshots.
    fn watch(&self, events: Sender<Event>) -> SourceFuture<'_, ()>;
}

/// Deterministic source for application and component tests.
#[derive(Clone)]
pub struct FakeAppearanceSource {
    state: Arc<Mutex<Snapshot>>,
    events: async_channel::Receiver<Event>,
}

#[derive(Clone)]
pub struct FakeAppearanceController {
    state: Arc<Mutex<Snapshot>>,
    events: async_channel::Sender<Event>,
}

impl FakeAppearanceSource {
    pub fn new(snapshot: Snapshot) -> (Self, FakeAppearanceController) {
        let state = Arc::new(Mutex::new(snapshot));
        let (events_tx, events_rx) = async_channel::unbounded();
        (
            Self {
                state: Arc::clone(&state),
                events: events_rx,
            },
            FakeAppearanceController {
                state,
                events: events_tx,
            },
        )
    }

    pub fn current(&self) -> Snapshot {
        self.state.lock().expect("fake state lock poisoned").clone()
    }
}

impl FakeAppearanceController {
    pub fn set_snapshot(&self, snapshot: Snapshot) {
        *self.state.lock().expect("fake state lock poisoned") = snapshot.clone();
        let _ = self.events.try_send(Event::Snapshot(snapshot));
    }

    pub fn set_unavailable(&self, error: Error) {
        let _ = self.events.try_send(Event::Unavailable(error));
    }
}

impl AppearanceSource for FakeAppearanceSource {
    fn snapshot(&self) -> SourceFuture<'_, Snapshot> {
        Box::pin(async { Ok(self.current()) })
    }

    fn watch(&self, events: Sender<Event>) -> SourceFuture<'_, ()> {
        Box::pin(async move {
            while let Ok(event) = self.events.recv().await {
                if events.send(event).await.is_err() {
                    return Ok(());
                }
            }
            Ok(())
        })
    }
}
