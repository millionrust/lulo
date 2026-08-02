use std::fmt;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use futures_util::FutureExt as _;

#[cfg(target_os = "linux")]
use crate::{Coordinator, Snapshot};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SeatEvent {
    Connecting,
    Snapshot(rmac_shell_invocation::SeatInventory),
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    WatchCompositor,
    WatchSeats,
    Consume,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Error {
    operation: Operation,
    detail: String,
}

impl Error {
    #[cfg(target_os = "linux")]
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
        formatter.write_str("Could not maintain shell invocation state")
    }
}

impl std::error::Error for Error {}

#[cfg(target_os = "linux")]
pub async fn watch(sender: async_channel::Sender<Snapshot>) -> Result<(), Error> {
    let (compositor_tx, compositor_rx) = async_channel::bounded(64);
    let (seat_tx, seat_rx) = async_channel::bounded(4);
    let compositor = watch_compositor(compositor_tx);
    let seats = watch_seats(seat_tx);
    let consumer = consume(sender, compositor_rx, seat_rx);
    let (_, _, _) = futures_util::try_join!(compositor, seats, consumer)?;
    Ok(())
}

#[cfg(target_os = "linux")]
async fn watch_compositor(
    sender: async_channel::Sender<rmac_compositor::Event>,
) -> Result<(), Error> {
    loop {
        let watcher = rmac_compositor_niri::watch(sender.clone()).fuse();
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(watcher, closed);
        futures_util::select! {
            result = watcher => match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    let _ = error;
                    if sender.send(rmac_compositor::Event::ConnectionChanged {
                        state: rmac_compositor::ConnectionState::Disconnected,
                    }).await.is_err() {
                        return Ok(());
                    }
                }
            },
            _ = closed => return Ok(()),
        }
        wait_or_closed(&sender, Duration::from_secs(1)).await;
        if sender.is_closed() {
            return Ok(());
        }
    }
}

#[cfg(target_os = "linux")]
async fn watch_seats(sender: async_channel::Sender<SeatEvent>) -> Result<(), Error> {
    loop {
        if sender.send(SeatEvent::Connecting).await.is_err() {
            return Ok(());
        }
        let (updates_tx, updates_rx) = async_channel::bounded(4);
        let watcher = rmac_shell_invocation::wayland::watch(updates_tx).fuse();
        futures_util::pin_mut!(watcher);
        loop {
            let update = updates_rx.recv().fuse();
            let closed = sender.closed().fuse();
            futures_util::pin_mut!(update, closed);
            futures_util::select! {
                result = watcher => {
                    match result {
                        Ok(()) if sender.is_closed() => return Ok(()),
                        Ok(()) | Err(_) => {
                            if sender.send(SeatEvent::Unavailable).await.is_err() {
                                return Ok(());
                            }
                            break;
                        }
                    }
                },
                update = update => match update {
                    Ok(snapshot) => {
                        if sender.send(SeatEvent::Snapshot(snapshot)).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(_) => {
                        if sender.send(SeatEvent::Unavailable).await.is_err() {
                            return Ok(());
                        }
                        break;
                    }
                },
                _ = closed => return Ok(()),
            }
        }
        wait_or_closed(&sender, Duration::from_secs(1)).await;
        if sender.is_closed() {
            return Ok(());
        }
    }
}

#[cfg(target_os = "linux")]
async fn wait_or_closed<T>(sender: &async_channel::Sender<T>, duration: Duration) {
    let timer = async_io::Timer::after(duration).fuse();
    let closed = sender.closed().fuse();
    futures_util::pin_mut!(timer, closed);
    futures_util::select! {
        _ = timer => {},
        _ = closed => {},
    }
}

#[cfg(target_os = "linux")]
async fn consume(
    sender: async_channel::Sender<Snapshot>,
    compositor: async_channel::Receiver<rmac_compositor::Event>,
    seats: async_channel::Receiver<SeatEvent>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut published: Option<Snapshot> = None;
    loop {
        let compositor_event = compositor.recv().fuse();
        let seat_event = seats.recv().fuse();
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(compositor_event, seat_event, closed);
        futures_util::select! {
            event = compositor_event => {
                let event = event.map_err(|_| Error::new(
                    Operation::WatchCompositor,
                    "the compositor source stopped",
                ))?;
                coordinator.apply_compositor(event);
            },
            event = seat_event => {
                let event = event.map_err(|_| Error::new(
                    Operation::WatchSeats,
                    "the Wayland seat source stopped",
                ))?;
                coordinator.apply_seats(event);
            },
            _ = closed => return Ok(()),
        }
        let next = coordinator.snapshot();
        if published.as_ref() == Some(&next) {
            continue;
        }
        if sender.send(next.clone()).await.is_err() {
            return Ok(());
        }
        published = Some(next);
    }
}
