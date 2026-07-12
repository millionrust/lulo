//! Non-mutating Wayland preflight for the future session-lock adapter.
//!
//! This module deliberately does not issue `ext_session_lock_manager_v1.lock`.
//! Acquiring a session lock before output surfaces, input, and authentication
//! are wired could leave a development session unusable. The probe proves only
//! that the compositor advertises the rendering globals, session-lock protocol
//! version 1, and at least one output.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use wayland_client::globals::{registry_queue_init, GlobalError, GlobalListContents};
use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_shm};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};

#[cfg(test)]
use wayland_client::protocol::wl_output::WlOutput;
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1;

use crate::registry_probe;
use crate::surface::{Error as SurfaceError, SurfaceSet};
use rmac_lock_provider::OutputId;

/// Capabilities observed in one initial registry snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    compositor_version: u32,
    session_lock_version: u32,
    shm_version: u32,
    output_count: usize,
}

impl Capabilities {
    pub fn compositor_version(self) -> u32 {
        self.compositor_version
    }

    pub fn session_lock_version(self) -> u32 {
        self.session_lock_version
    }

    pub fn shm_version(self) -> u32 {
        self.shm_version
    }

    pub fn output_count(self) -> usize {
        self.output_count
    }
}

/// Connect to the compositor and inspect its initial global registry.
///
/// This is a read-only preflight. The connection is dropped before returning,
/// no global is bound, and the session is never locked.
pub fn probe() -> Result<Capabilities, Error> {
    let connection = Connection::connect_to_env().map_err(Error::Connect)?;
    let (globals, _event_queue) =
        registry_queue_init::<ProbeState>(&connection).map_err(Error::ReadRegistry)?;

    globals.contents().with_list(classify_globals)
}

/// A safe Wayland connection prepared for future lock acquisition.
///
/// Required globals and outputs are bound and hotplug is tracked, but this
/// type has no method that can issue the session-lock request.
pub struct PreparedConnection {
    _connection: Connection,
    _registry: wl_registry::WlRegistry,
    event_queue: EventQueue<PreparedState>,
    state: PreparedState,
}

impl PreparedConnection {
    pub fn connect() -> Result<Self, PrepareError> {
        let connection = Connection::connect_to_env().map_err(PrepareError::Connect)?;
        let mut event_queue = connection.new_event_queue();
        let queue_handle = event_queue.handle();
        let registry = connection.display().get_registry(&queue_handle, ());
        let mut state = PreparedState::default();
        event_queue
            .roundtrip(&mut state)
            .map_err(PrepareError::Dispatch)?;
        state.require_ready()?;
        // Bind requests are emitted while dispatching the first roundtrip.
        // A second one receives each output's initial scale before return.
        event_queue
            .roundtrip(&mut state)
            .map_err(PrepareError::Dispatch)?;
        state.require_ready()?;

        Ok(Self {
            _connection: connection,
            _registry: registry,
            event_queue,
            state,
        })
    }

    pub fn output_count(&self) -> usize {
        self.state.outputs.len()
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = PreparedEvent> + '_ {
        self.state.events.drain(..)
    }

    pub fn dispatch_pending(&mut self) -> Result<usize, PrepareError> {
        let dispatched = self
            .event_queue
            .dispatch_pending(&mut self.state)
            .map_err(PrepareError::Dispatch)?;
        self.state.check_failure()?;
        Ok(dispatched)
    }

    pub fn blocking_dispatch(&mut self) -> Result<usize, PrepareError> {
        let dispatched = self
            .event_queue
            .blocking_dispatch(&mut self.state)
            .map_err(PrepareError::Dispatch)?;
        self.state.check_failure()?;
        Ok(dispatched)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedEvent {
    OutputAdded(OutputId),
    OutputRemoved(OutputId),
    OutputScaleChanged { output: OutputId, scale: u32 },
}

struct RequiredBinding<T> {
    global_name: u32,
    _proxy: T,
}

struct OutputBinding {
    output: OutputId,
    proxy: wl_output::WlOutput,
}

#[derive(Clone, Copy)]
struct OutputData {
    output: OutputId,
}

#[derive(Default)]
struct PreparedState {
    compositor: Option<RequiredBinding<wl_compositor::WlCompositor>>,
    shm: Option<RequiredBinding<wl_shm::WlShm>>,
    manager: Option<RequiredBinding<ExtSessionLockManagerV1>>,
    outputs: BTreeMap<u32, OutputBinding>,
    surfaces: SurfaceSet,
    events: VecDeque<PreparedEvent>,
    failure: Option<PreparedStateError>,
}

impl PreparedState {
    fn bind_global(
        &mut self,
        registry: &wl_registry::WlRegistry,
        name: u32,
        interface: &str,
        version: u32,
        queue_handle: &QueueHandle<Self>,
    ) {
        match interface {
            registry_probe::COMPOSITOR_INTERFACE
                if self.compositor.is_none()
                    && version >= registry_probe::REQUIRED_COMPOSITOR_VERSION =>
            {
                self.compositor = Some(RequiredBinding {
                    global_name: name,
                    _proxy: registry.bind(
                        name,
                        version.min(wl_compositor::WlCompositor::interface().version),
                        queue_handle,
                        (),
                    ),
                });
            }
            registry_probe::SHM_INTERFACE
                if self.shm.is_none() && version >= registry_probe::REQUIRED_SHM_VERSION =>
            {
                self.shm = Some(RequiredBinding {
                    global_name: name,
                    _proxy: registry.bind(
                        name,
                        version.min(wl_shm::WlShm::interface().version),
                        queue_handle,
                        (),
                    ),
                });
            }
            registry_probe::MANAGER_INTERFACE
                if self.manager.is_none()
                    && version >= registry_probe::REQUIRED_SESSION_LOCK_VERSION =>
            {
                self.manager = Some(RequiredBinding {
                    global_name: name,
                    _proxy: registry.bind(
                        name,
                        version.min(ExtSessionLockManagerV1::interface().version),
                        queue_handle,
                        (),
                    ),
                });
            }
            registry_probe::OUTPUT_INTERFACE if !self.outputs.contains_key(&name) => {
                let Some(output) = OutputId::new(u64::from(name)) else {
                    self.record_failure(PreparedStateError::InvalidOutputId);
                    return;
                };
                let proxy = registry.bind(
                    name,
                    version.min(wl_output::WlOutput::interface().version),
                    queue_handle,
                    OutputData { output },
                );
                if let Err(error) = self.surfaces.add_output(output) {
                    self.record_failure(PreparedStateError::Surface(error));
                    return;
                }
                self.outputs.insert(name, OutputBinding { output, proxy });
                self.events.push_back(PreparedEvent::OutputAdded(output));
            }
            _ => {}
        }
    }

    fn remove_global(&mut self, name: u32) {
        if self
            .compositor
            .as_ref()
            .is_some_and(|binding| binding.global_name == name)
            || self
                .shm
                .as_ref()
                .is_some_and(|binding| binding.global_name == name)
            || self
                .manager
                .as_ref()
                .is_some_and(|binding| binding.global_name == name)
        {
            self.record_failure(PreparedStateError::RequiredGlobalRemoved);
            return;
        }

        let Some(binding) = self.outputs.remove(&name) else {
            return;
        };
        if binding.proxy.version() >= 3 {
            binding.proxy.release();
        }
        if let Err(error) = self.surfaces.remove_output(binding.output) {
            self.record_failure(PreparedStateError::Surface(error));
            return;
        }
        self.events
            .push_back(PreparedEvent::OutputRemoved(binding.output));
    }

    fn require_ready(&self) -> Result<(), PrepareError> {
        self.check_failure()?;
        if self.compositor.is_none() {
            return Err(PrepareError::MissingRequiredGlobal("wl_compositor v4"));
        }
        if self.shm.is_none() {
            return Err(PrepareError::MissingRequiredGlobal("wl_shm v1"));
        }
        if self.manager.is_none() {
            return Err(PrepareError::MissingRequiredGlobal(
                "ext_session_lock_manager_v1 v1",
            ));
        }
        if self.outputs.is_empty() {
            return Err(PrepareError::NoOutputs);
        }
        Ok(())
    }

    fn record_failure(&mut self, failure: PreparedStateError) {
        if self.failure.is_none() {
            self.failure = Some(failure);
        }
    }

    fn check_failure(&self) -> Result<(), PrepareError> {
        match self.failure {
            Some(failure) => Err(PrepareError::State(failure)),
            None => Ok(()),
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for PreparedState {
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
            } => state.bind_global(registry, name, &interface, version, queue_handle),
            wl_registry::Event::GlobalRemove { name } => state.remove_global(name),
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, OutputData> for PreparedState {
    fn event(
        state: &mut Self,
        _proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        data: &OutputData,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Scale { factor } = event {
            let Ok(scale) = u32::try_from(factor) else {
                state.record_failure(PreparedStateError::InvalidOutputScale);
                return;
            };
            match state.surfaces.set_scale(data.output, scale) {
                Ok(true) => state.events.push_back(PreparedEvent::OutputScaleChanged {
                    output: data.output,
                    scale,
                }),
                Ok(false) => {}
                Err(error) => state.record_failure(PreparedStateError::Surface(error)),
            }
        }
    }
}

delegate_noop!(PreparedState: ignore wl_compositor::WlCompositor);
delegate_noop!(PreparedState: ignore wl_shm::WlShm);
delegate_noop!(PreparedState: ignore ExtSessionLockManagerV1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedStateError {
    InvalidOutputId,
    InvalidOutputScale,
    RequiredGlobalRemoved,
    Surface(SurfaceError),
}

#[derive(Debug)]
pub enum PrepareError {
    Connect(wayland_client::ConnectError),
    Dispatch(wayland_client::DispatchError),
    MissingRequiredGlobal(&'static str),
    NoOutputs,
    State(PreparedStateError),
}

impl fmt::Display for PrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(_) => formatter.write_str("cannot connect to the Wayland compositor"),
            Self::Dispatch(_) => formatter.write_str("Wayland event dispatch failed"),
            Self::MissingRequiredGlobal(interface) => {
                write!(
                    formatter,
                    "Wayland compositor is missing required {interface}"
                )
            }
            Self::NoOutputs => formatter.write_str("Wayland compositor has no outputs"),
            Self::State(failure) => {
                write!(formatter, "Wayland lock preflight failed ({failure:?})")
            }
        }
    }
}

impl std::error::Error for PrepareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(error) => Some(error),
            Self::Dispatch(error) => Some(error),
            Self::MissingRequiredGlobal(_) | Self::NoOutputs | Self::State(_) => None,
        }
    }
}

fn classify_globals(globals: &[wayland_client::globals::Global]) -> Result<Capabilities, Error> {
    let capabilities = registry_probe::classify(
        globals
            .iter()
            .map(|global| (global.interface.as_str(), global.version)),
    )
    .map_err(|error| match error {
        registry_probe::Error::CompositorUnavailable => Error::CompositorUnavailable,
        registry_probe::Error::CompositorVersion {
            advertised,
            required,
        } => Error::CompositorVersion {
            advertised,
            required,
        },
        registry_probe::Error::SessionLockUnavailable => Error::SessionLockUnavailable,
        registry_probe::Error::SessionLockVersion {
            advertised,
            required,
        } => Error::SessionLockVersion {
            advertised,
            required,
        },
        registry_probe::Error::ShmUnavailable => Error::ShmUnavailable,
        registry_probe::Error::ShmVersion {
            advertised,
            required,
        } => Error::ShmVersion {
            advertised,
            required,
        },
        registry_probe::Error::NoOutputs => Error::NoOutputs,
    })?;

    Ok(Capabilities {
        compositor_version: capabilities.compositor_version,
        session_lock_version: capabilities.session_lock_version,
        shm_version: capabilities.shm_version,
        output_count: capabilities.output_count,
    })
}

struct ProbeState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for ProbeState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` collects the initial snapshot before returning.
        // This one-shot probe ignores later changes; `PreparedConnection`
        // provides the live hotplug path.
    }
}

#[derive(Debug)]
pub enum Error {
    Connect(wayland_client::ConnectError),
    ReadRegistry(GlobalError),
    CompositorUnavailable,
    CompositorVersion { advertised: u32, required: u32 },
    SessionLockUnavailable,
    SessionLockVersion { advertised: u32, required: u32 },
    ShmUnavailable,
    ShmVersion { advertised: u32, required: u32 },
    NoOutputs,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(_) => formatter.write_str("cannot connect to the Wayland compositor"),
            Self::ReadRegistry(_) => formatter.write_str("cannot read the Wayland global registry"),
            Self::CompositorUnavailable => {
                formatter.write_str("compositor does not advertise wl_compositor")
            }
            Self::CompositorVersion {
                advertised,
                required,
            } => write!(
                formatter,
                "compositor advertises wl_compositor version {advertised}, but version {required} is required"
            ),
            Self::SessionLockUnavailable => {
                formatter.write_str("compositor does not advertise ext-session-lock-v1")
            }
            Self::SessionLockVersion {
                advertised,
                required,
            } => write!(
                formatter,
                "compositor advertises session-lock version {advertised}, but version {required} is required"
            ),
            Self::ShmUnavailable => formatter.write_str("compositor does not advertise wl_shm"),
            Self::ShmVersion {
                advertised,
                required,
            } => write!(
                formatter,
                "compositor advertises wl_shm version {advertised}, but version {required} is required"
            ),
            Self::NoOutputs => formatter.write_str("compositor does not advertise any outputs"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(error) => Some(error),
            Self::ReadRegistry(error) => Some(error),
            Self::CompositorUnavailable
            | Self::CompositorVersion { .. }
            | Self::SessionLockUnavailable
            | Self::SessionLockVersion { .. }
            | Self::ShmUnavailable
            | Self::ShmVersion { .. }
            | Self::NoOutputs => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_generated_protocol_interface_names() {
        assert_eq!(
            wl_compositor::WlCompositor::interface().name,
            registry_probe::COMPOSITOR_INTERFACE
        );
        assert_eq!(
            ExtSessionLockManagerV1::interface().name,
            registry_probe::MANAGER_INTERFACE
        );
        assert_eq!(WlOutput::interface().name, registry_probe::OUTPUT_INTERFACE);
        assert_eq!(
            wl_shm::WlShm::interface().name,
            registry_probe::SHM_INTERFACE
        );
    }
}
