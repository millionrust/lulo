use std::fmt;

#[cfg(any(target_os = "linux", test))]
use crate::{Coordinator, Update};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    WatchShortcut,
    WatchInvocation,
    Consume,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Error {
    operation: Operation,
    detail: String,
}

impl Error {
    #[cfg(any(target_os = "linux", test))]
    fn new(operation: Operation, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("operation", &self.operation)
            .field("detail", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Could not maintain the shell activation endpoint")
    }
}

impl std::error::Error for Error {}

/// Own one shortcut endpoint and publish activations paired with one immutable
/// live compositor/seat snapshot. Readiness is emitted exactly once after both
/// the endpoint and sources are usable.
#[cfg(target_os = "linux")]
pub async fn watch(
    shortcut: rmac_shortcuts::ShortcutId,
    sender: async_channel::Sender<Update>,
) -> Result<(), Error> {
    use futures_util::FutureExt as _;

    let (shortcut_tx, shortcut_rx) = async_channel::bounded(8);
    let (endpoint_tx, endpoint_rx) = async_channel::bounded(1);
    let (runtime_tx, runtime_rx) = async_channel::bounded(8);
    let shortcut_source = async {
        rmac_shortcuts::watch_dispatches_ready(shortcut, shortcut_tx, endpoint_tx)
            .await
            .map_err(|error| Error::new(Operation::WatchShortcut, error.to_string()))
    };
    let runtime_source = async {
        rmac_shell_invocation_runtime::watch(runtime_tx)
            .await
            .map_err(|error| Error::new(Operation::WatchInvocation, error.detail()))
    };
    let sources = async {
        let (_, _) = futures_util::try_join!(shortcut_source, runtime_source)?;
        Ok::<(), Error>(())
    }
    .fuse();
    let consumer = consume(sender, shortcut_rx, endpoint_rx, runtime_rx).fuse();
    futures_util::pin_mut!(sources, consumer);
    futures_util::select! {
        result = sources => result,
        result = consumer => result,
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) async fn consume(
    sender: async_channel::Sender<Update>,
    shortcuts: async_channel::Receiver<rmac_shortcuts::Event>,
    endpoint: async_channel::Receiver<()>,
    runtime: async_channel::Receiver<rmac_shell_invocation_runtime::Snapshot>,
) -> Result<(), Error> {
    use futures_util::FutureExt as _;

    let mut coordinator = Coordinator::default();
    let endpoint_ready = endpoint.recv().fuse();
    futures_util::pin_mut!(endpoint_ready);
    loop {
        let shortcut = shortcuts.recv().fuse();
        let runtime = runtime.recv().fuse();
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(shortcut, runtime, closed);
        let update = futures_util::select! {
            event = shortcut => Some(Update::Activated(Box::new(coordinator.activate(
                event.map_err(|_| {
                    Error::new(Operation::WatchShortcut, "the shortcut endpoint stopped")
                })?,
            )))),
            ready = endpoint_ready => {
                ready.map_err(|_| Error::new(
                    Operation::WatchShortcut,
                    "the shortcut endpoint stopped before readiness",
                ))?;
                coordinator.endpoint_ready().then_some(Update::Ready)
            },
            snapshot = runtime => {
                let snapshot = snapshot.map_err(|_| Error::new(
                    Operation::WatchInvocation,
                    "the invocation source stopped",
                ))?;
                coordinator.apply_runtime(snapshot).then_some(Update::Ready)
            },
            _ = closed => return Ok(()),
        };
        if let Some(update) = update {
            sender
                .send(update)
                .await
                .map_err(|_| Error::new(Operation::Consume, "the activation consumer stopped"))?;
        }
    }
}
