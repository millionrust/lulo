//! Linux Wayland adapter for complete live seat inventories.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io;
use std::os::fd::OwnedFd;

use async_channel::Sender;
use async_io::Async;
use futures_util::FutureExt as _;
use wayland_client::backend::{ReadEventsGuard, WaylandError};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

use crate::{RegistryError, SeatInventory, SeatRegistry};

pub struct Watcher {
    _connection: Connection,
    _registry: wl_registry::WlRegistry,
    event_queue: EventQueue<State>,
    state: State,
}

impl Watcher {
    pub fn connect() -> Result<Self, Error> {
        let connection = Connection::connect_to_env().map_err(Error::Connect)?;
        let mut event_queue = connection.new_event_queue();
        let queue_handle = event_queue.handle();
        let registry = connection.display().get_registry(&queue_handle, ());
        let mut state = State::default();
        event_queue.roundtrip(&mut state).map_err(Error::Dispatch)?;
        state.check_failure()?;
        // Seat binds are issued during the first roundtrip. The second receives
        // each required wl_seat.name event.
        event_queue.roundtrip(&mut state).map_err(Error::Dispatch)?;
        state.check_failure()?;
        state.registry.require_complete().map_err(Error::Registry)?;
        state.updates.clear();
        Ok(Self {
            _connection: connection,
            _registry: registry,
            event_queue,
            state,
        })
    }

    pub fn snapshot(&self) -> &SeatInventory {
        self.state
            .registry
            .snapshot()
            .expect("connect publishes one complete seat inventory")
    }

    /// Block for protocol work and return only the newest complete changed
    /// inventory. A newly announced seat remains unpublished until its name
    /// arrives.
    pub fn blocking_dispatch(&mut self) -> Result<Option<SeatInventory>, Error> {
        self.event_queue
            .blocking_dispatch(&mut self.state)
            .map_err(Error::Dispatch)?;
        self.state.check_failure()?;
        Ok(self.state.take_latest())
    }

    pub fn dispatch_pending(&mut self) -> Result<Option<SeatInventory>, Error> {
        self.event_queue
            .dispatch_pending(&mut self.state)
            .map_err(Error::Dispatch)?;
        self.state.check_failure()?;
        Ok(self.state.take_latest())
    }

    fn poll_fd(&mut self) -> Result<OwnedFd, Error> {
        loop {
            if let Some(guard) = self.event_queue.prepare_read() {
                return guard
                    .connection_fd()
                    .try_clone_to_owned()
                    .map_err(Error::Io);
            }
            self.dispatch_pending()?;
        }
    }

    fn prepare_read(&self) -> Option<ReadEventsGuard> {
        self.event_queue.prepare_read()
    }

    fn flush(&self) -> Result<(), Error> {
        self.event_queue.flush().map_err(Error::Wayland)
    }
}

/// Publish complete changed seat inventories using Wayland socket readiness.
/// The future exits cleanly when its consumer closes and performs no polling.
pub async fn watch(sender: Sender<SeatInventory>) -> Result<(), Error> {
    let mut watcher = Watcher::connect()?;
    let readiness = Async::new(watcher.poll_fd()?).map_err(Error::Io)?;
    if sender.send(watcher.snapshot().clone()).await.is_err() {
        return Ok(());
    }

    loop {
        if let Some(update) = watcher.dispatch_pending()? {
            if sender.send(update).await.is_err() {
                return Ok(());
            }
        }
        let Some(read) = watcher.prepare_read() else {
            continue;
        };
        watcher.flush()?;

        let readable = readiness.readable().fuse();
        let closed = sender.closed().fuse();
        futures_util::pin_mut!(readable, closed);
        futures_util::select! {
            result = readable => result.map_err(Error::Io)?,
            _ = closed => return Ok(()),
        }
        read_events(read)?;
    }
}

fn read_events(read: ReadEventsGuard) -> Result<(), Error> {
    match read.read() {
        Ok(_) => Ok(()),
        Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(error) => Err(Error::Wayland(error)),
    }
}

#[derive(Default)]
struct State {
    registry: SeatRegistry,
    bindings: BTreeMap<u32, wl_seat::WlSeat>,
    updates: VecDeque<SeatInventory>,
    failure: Option<RegistryError>,
}

impl State {
    fn record(&mut self, result: Result<Option<SeatInventory>, RegistryError>) {
        match result {
            Ok(Some(snapshot)) => self.updates.push_back(snapshot),
            Ok(None) => {}
            Err(error) if self.failure.is_none() => self.failure = Some(error),
            Err(_) => {}
        }
    }

    fn check_failure(&self) -> Result<(), Error> {
        self.failure
            .map_or(Ok(()), |error| Err(Error::Registry(error)))
    }

    fn take_latest(&mut self) -> Option<SeatInventory> {
        let latest = self.updates.pop_back();
        self.updates.clear();
        latest
    }
}

#[derive(Clone, Copy)]
struct SeatData {
    global: u32,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _connection: &Connection,
        queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == wl_seat::WlSeat::interface().name => {
                if let Err(error) = state.registry.add(name, version) {
                    if state.failure.is_none() {
                        state.failure = Some(error);
                    }
                    return;
                }
                let seat = registry.bind(
                    name,
                    version.min(wl_seat::WlSeat::interface().version),
                    queue_handle,
                    SeatData { global: name },
                );
                state.bindings.insert(name, seat);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if state.bindings.remove(&name).is_some() {
                    let update = state.registry.remove(name);
                    state.record(update);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, SeatData> for State {
    fn event(
        state: &mut Self,
        _seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        data: &SeatData,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Name { name } = event {
            let update = state.registry.name(data.global, name);
            state.record(update);
        }
    }
}

pub enum Error {
    Connect(wayland_client::ConnectError),
    Dispatch(wayland_client::DispatchError),
    Io(io::Error),
    Wayland(WaylandError),
    Registry(RegistryError),
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Connect(_) => "Error::Connect(<redacted>)",
            Self::Dispatch(_) => "Error::Dispatch(<redacted>)",
            Self::Io(_) => "Error::Io(<redacted>)",
            Self::Wayland(_) => "Error::Wayland(<redacted>)",
            Self::Registry(_) => "Error::Registry(<redacted>)",
        })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(_) => formatter.write_str("could not connect to the Wayland compositor"),
            Self::Dispatch(_) => formatter.write_str("could not read Wayland seat events"),
            Self::Io(_) => formatter.write_str("could not watch the Wayland connection"),
            Self::Wayland(_) => formatter.write_str("the Wayland connection stopped"),
            Self::Registry(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {}
