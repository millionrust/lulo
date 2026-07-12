//! Non-mutating Wayland preflight for the future session-lock adapter.
//!
//! This module deliberately does not issue `ext_session_lock_manager_v1.lock`.
//! Acquiring a session lock before output surfaces, input, and authentication
//! are wired could leave a development session unusable. The probe proves only
//! that the compositor advertises the rendering and keyboard globals,
//! session-lock protocol version 1, and at least one output.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::num::NonZeroU64;
use std::panic::{catch_unwind, AssertUnwindSafe};

use wayland_client::globals::{registry_queue_init, GlobalError, GlobalListContents};
use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_output, wl_registry, wl_seat, wl_shm,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

#[cfg(test)]
use wayland_client::protocol::wl_output::WlOutput;
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1;

use crate::keyboard::DecodedKey;
use crate::registry_probe;
use crate::surface::{Error as SurfaceError, SurfaceSet};
use crate::xkb_keyboard::{Error as XkbError, KeyboardDecoder};
use rmac_lock_provider::OutputId;

/// Capabilities observed in one initial registry snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    compositor_version: u32,
    session_lock_version: u32,
    shm_version: u32,
    output_count: usize,
    seat_version: u32,
    seat_count: usize,
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

    pub fn seat_version(self) -> u32 {
        self.seat_version
    }

    pub fn seat_count(self) -> usize {
        self.seat_count
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
        state.require_registry_ready()?;
        // Bind requests are emitted while dispatching the first roundtrip.
        // A second one receives each output's initial scale before return.
        event_queue
            .roundtrip(&mut state)
            .map_err(PrepareError::Dispatch)?;
        state.require_keyboard_ready()?;
        // The keyboard request is emitted while processing seat capabilities;
        // this final roundtrip receives its keymap before input is accepted.
        event_queue
            .roundtrip(&mut state)
            .map_err(PrepareError::Dispatch)?;
        state.require_input_ready()?;

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

    pub fn keyboard_count(&self) -> usize {
        self.state.keyboard_count()
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

#[derive(Debug)]
pub enum PreparedEvent {
    OutputAdded(OutputId),
    OutputRemoved(OutputId),
    OutputScaleChanged {
        output: OutputId,
        scale: u32,
    },
    KeyboardAvailabilityChanged {
        available: bool,
    },
    KeyboardFocusChanged {
        seat: SeatId,
        focused: bool,
    },
    KeyboardInput {
        seat: SeatId,
        input: DecodedKey,
    },
    KeyboardRepeat {
        seat: SeatId,
        rate: u32,
        delay_ms: u32,
    },
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct SeatId(NonZeroU64);

impl SeatId {
    fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }
}

impl fmt::Debug for SeatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SeatId(<redacted>)")
    }
}

struct RequiredBinding<T> {
    global_name: u32,
    _proxy: T,
}

struct OutputBinding {
    output: OutputId,
    proxy: wl_output::WlOutput,
}

struct SeatBinding {
    seat: SeatId,
    proxy: wl_seat::WlSeat,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    decoder: KeyboardDecoder,
    focused: bool,
}

#[derive(Clone, Copy)]
struct OutputData {
    output: OutputId,
}

#[derive(Clone, Copy)]
struct SeatData {
    global_name: u32,
    seat: SeatId,
}

#[derive(Clone, Copy)]
struct KeyboardData {
    seat_global_name: u32,
    seat: SeatId,
}

#[derive(Default)]
struct PreparedState {
    compositor: Option<RequiredBinding<wl_compositor::WlCompositor>>,
    shm: Option<RequiredBinding<wl_shm::WlShm>>,
    manager: Option<RequiredBinding<ExtSessionLockManagerV1>>,
    outputs: BTreeMap<u32, OutputBinding>,
    seats: BTreeMap<u32, SeatBinding>,
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
            registry_probe::SEAT_INTERFACE
                if !self.seats.contains_key(&name)
                    && version >= registry_probe::REQUIRED_SEAT_VERSION =>
            {
                let Some(seat) = SeatId::new(u64::from(name)) else {
                    self.record_failure(PreparedStateError::InvalidSeatId);
                    return;
                };
                let proxy = registry.bind(
                    name,
                    version.min(wl_seat::WlSeat::interface().version),
                    queue_handle,
                    SeatData {
                        global_name: name,
                        seat,
                    },
                );
                self.seats.insert(
                    name,
                    SeatBinding {
                        seat,
                        proxy,
                        keyboard: None,
                        decoder: KeyboardDecoder::new(),
                        focused: false,
                    },
                );
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
            if let Some(mut seat) = self.seats.remove(&name) {
                let was_available = seat.keyboard.is_some() || self.keyboard_count() > 0;
                if let Some(keyboard) = seat.keyboard.take() {
                    if keyboard.version() >= 3 {
                        keyboard.release();
                    }
                }
                if seat.focused {
                    self.events.push_back(PreparedEvent::KeyboardFocusChanged {
                        seat: seat.seat,
                        focused: false,
                    });
                }
                if seat.proxy.version() >= 5 {
                    seat.proxy.release();
                }
                let available = self.keyboard_count() > 0;
                if available != was_available {
                    self.events
                        .push_back(PreparedEvent::KeyboardAvailabilityChanged { available });
                }
            }
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

    fn require_registry_ready(&self) -> Result<(), PrepareError> {
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
        if self.seats.is_empty() {
            return Err(PrepareError::NoKeyboardSeat);
        }
        Ok(())
    }

    fn require_keyboard_ready(&self) -> Result<(), PrepareError> {
        self.require_registry_ready()?;
        if self.keyboard_count() == 0 {
            return Err(PrepareError::NoKeyboardCapability);
        }
        Ok(())
    }

    fn require_input_ready(&self) -> Result<(), PrepareError> {
        self.require_keyboard_ready()?;
        if !self
            .seats
            .values()
            .any(|seat| seat.keyboard.is_some() && seat.decoder.is_ready())
        {
            return Err(PrepareError::NoKeyboardKeymap);
        }
        Ok(())
    }

    fn keyboard_count(&self) -> usize {
        self.seats
            .values()
            .filter(|seat| seat.keyboard.is_some())
            .count()
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

impl Dispatch<wl_seat::WlSeat, SeatData> for PreparedState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        data: &SeatData,
        _connection: &Connection,
        queue_handle: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities { capabilities } = event else {
            return;
        };
        let WEnum::Value(capabilities) = capabilities else {
            state.record_failure(PreparedStateError::Keyboard(
                KeyboardFailure::UnknownSeatCapabilities,
            ));
            return;
        };
        let was_available = state.keyboard_count() > 0;
        let Some(binding) = state.seats.get_mut(&data.global_name) else {
            state.record_failure(PreparedStateError::Keyboard(KeyboardFailure::SeatMissing));
            return;
        };
        if capabilities.contains(wl_seat::Capability::Keyboard) {
            if binding.keyboard.is_none() {
                binding.keyboard = Some(seat.get_keyboard(
                    queue_handle,
                    KeyboardData {
                        seat_global_name: data.global_name,
                        seat: data.seat,
                    },
                ));
            }
        } else if let Some(keyboard) = binding.keyboard.take() {
            binding.decoder.clear_keymap();
            if binding.focused {
                binding.focused = false;
                state.events.push_back(PreparedEvent::KeyboardFocusChanged {
                    seat: binding.seat,
                    focused: false,
                });
            }
            if keyboard.version() >= 3 {
                keyboard.release();
            }
        }
        let available = state.keyboard_count() > 0;
        if available != was_available {
            state
                .events
                .push_back(PreparedEvent::KeyboardAvailabilityChanged { available });
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, KeyboardData> for PreparedState {
    fn event(
        state: &mut Self,
        _keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        data: &KeyboardData,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        let Some(binding) = state.seats.get_mut(&data.seat_global_name) else {
            state.record_failure(PreparedStateError::Keyboard(KeyboardFailure::SeatMissing));
            return;
        };
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    state.record_failure(PreparedStateError::Keyboard(
                        KeyboardFailure::InvalidKeymapFormat,
                    ));
                } else {
                    match catch_unwind(AssertUnwindSafe(|| {
                        binding.decoder.install_keymap(fd, size)
                    })) {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            state.record_failure(PreparedStateError::Keyboard(error.into()))
                        }
                        Err(_) => state.record_failure(PreparedStateError::Keyboard(
                            KeyboardFailure::DecoderPanicked,
                        )),
                    }
                }
            }
            wl_keyboard::Event::Enter { .. } => {
                binding.focused = true;
                state.events.push_back(PreparedEvent::KeyboardFocusChanged {
                    seat: data.seat,
                    focused: true,
                });
            }
            wl_keyboard::Event::Leave { .. } => {
                if catch_unwind(AssertUnwindSafe(|| binding.decoder.reset_compose())).is_err() {
                    state.record_failure(PreparedStateError::Keyboard(
                        KeyboardFailure::DecoderPanicked,
                    ));
                    return;
                }
                binding.focused = false;
                state.events.push_back(PreparedEvent::KeyboardFocusChanged {
                    seat: data.seat,
                    focused: false,
                });
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => match key_state {
                WEnum::Value(wl_keyboard::KeyState::Pressed)
                | WEnum::Value(wl_keyboard::KeyState::Repeated) => {
                    match catch_unwind(AssertUnwindSafe(|| binding.decoder.decode_press(key))) {
                        Ok(Ok(Some(input))) => {
                            state.events.push_back(PreparedEvent::KeyboardInput {
                                seat: data.seat,
                                input,
                            })
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(error)) => {
                            state.record_failure(PreparedStateError::Keyboard(error.into()))
                        }
                        Err(_) => state.record_failure(PreparedStateError::Keyboard(
                            KeyboardFailure::DecoderPanicked,
                        )),
                    }
                }
                WEnum::Value(wl_keyboard::KeyState::Released) => {}
                WEnum::Unknown(_) => state.record_failure(PreparedStateError::Keyboard(
                    KeyboardFailure::InvalidKeyState,
                )),
                WEnum::Value(_) => state.record_failure(PreparedStateError::Keyboard(
                    KeyboardFailure::InvalidKeyState,
                )),
            },
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                match catch_unwind(AssertUnwindSafe(|| {
                    binding.decoder.update_modifiers(
                        mods_depressed,
                        mods_latched,
                        mods_locked,
                        group,
                    )
                })) {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        state.record_failure(PreparedStateError::Keyboard(error.into()))
                    }
                    Err(_) => state.record_failure(PreparedStateError::Keyboard(
                        KeyboardFailure::DecoderPanicked,
                    )),
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                let (Ok(rate), Ok(delay_ms)) = (u32::try_from(rate), u32::try_from(delay)) else {
                    state.record_failure(PreparedStateError::Keyboard(
                        KeyboardFailure::InvalidRepeatInfo,
                    ));
                    return;
                };
                state.events.push_back(PreparedEvent::KeyboardRepeat {
                    seat: data.seat,
                    rate,
                    delay_ms,
                });
            }
            _ => {}
        }
    }
}

delegate_noop!(PreparedState: ignore wl_compositor::WlCompositor);
delegate_noop!(PreparedState: ignore wl_shm::WlShm);
delegate_noop!(PreparedState: ignore ExtSessionLockManagerV1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedStateError {
    InvalidOutputId,
    InvalidSeatId,
    InvalidOutputScale,
    RequiredGlobalRemoved,
    Surface(SurfaceError),
    Keyboard(KeyboardFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardFailure {
    UnknownSeatCapabilities,
    SeatMissing,
    InvalidKeymapFormat,
    InvalidKeymapSize,
    MapKeymap,
    CompileKeymap,
    KeymapUnavailable,
    InvalidKeycode,
    InvalidKeyState,
    InvalidRepeatInfo,
    DecoderPanicked,
}

impl From<XkbError> for KeyboardFailure {
    fn from(error: XkbError) -> Self {
        match error {
            XkbError::InvalidKeymapSize => Self::InvalidKeymapSize,
            XkbError::MapKeymap => Self::MapKeymap,
            XkbError::CompileKeymap => Self::CompileKeymap,
            XkbError::KeymapUnavailable => Self::KeymapUnavailable,
            XkbError::InvalidKeycode => Self::InvalidKeycode,
        }
    }
}

#[derive(Debug)]
pub enum PrepareError {
    Connect(wayland_client::ConnectError),
    Dispatch(wayland_client::DispatchError),
    MissingRequiredGlobal(&'static str),
    NoOutputs,
    NoKeyboardSeat,
    NoKeyboardCapability,
    NoKeyboardKeymap,
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
            Self::NoKeyboardSeat => {
                formatter.write_str("Wayland compositor has no supported keyboard seat")
            }
            Self::NoKeyboardCapability => {
                formatter.write_str("Wayland compositor exposes no keyboard capability")
            }
            Self::NoKeyboardKeymap => {
                formatter.write_str("Wayland compositor did not provide a usable keyboard keymap")
            }
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
            Self::MissingRequiredGlobal(_)
            | Self::NoOutputs
            | Self::NoKeyboardSeat
            | Self::NoKeyboardCapability
            | Self::NoKeyboardKeymap
            | Self::State(_) => None,
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
        registry_probe::Error::SeatUnavailable => Error::SeatUnavailable,
        registry_probe::Error::SeatVersion {
            advertised,
            required,
        } => Error::SeatVersion {
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
        seat_version: capabilities.seat_version,
        seat_count: capabilities.seat_count,
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
    SeatUnavailable,
    SeatVersion { advertised: u32, required: u32 },
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
            Self::SeatUnavailable => {
                formatter.write_str("compositor does not advertise a keyboard seat")
            }
            Self::SeatVersion {
                advertised,
                required,
            } => write!(
                formatter,
                "compositor advertises wl_seat version {advertised}, but version {required} is required"
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
            | Self::SeatUnavailable
            | Self::SeatVersion { .. }
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
            wl_seat::WlSeat::interface().name,
            registry_probe::SEAT_INTERFACE
        );
        assert_eq!(
            wl_shm::WlShm::interface().name,
            registry_probe::SHM_INTERFACE
        );
        assert_eq!(
            format!("{:?}", SeatId::new(42).unwrap()),
            "SeatId(<redacted>)"
        );
    }
}
