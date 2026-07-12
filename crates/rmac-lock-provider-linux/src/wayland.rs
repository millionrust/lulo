//! Non-mutating Wayland preflight for the future session-lock adapter.
//!
//! Public APIs only probe or prepare the connection and cannot issue
//! `ext_session_lock_manager_v1.lock`. A crate-internal typestate wires the
//! complete object lifecycle for the unshipped provider runtime. The safe probe
//! proves only that the compositor advertises the rendering and keyboard
//! globals, session-lock protocol version 1, and at least one output.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::io;
use std::num::NonZeroU64;
use std::os::fd::{AsFd as _, AsRawFd as _};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use wayland_client::globals::{registry_queue_init, GlobalError, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

#[cfg(test)]
use wayland_client::protocol::wl_output::WlOutput;
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ext_session_lock_v1::ExtSessionLockV1,
};

use crate::key_repeat::RepeatScheduler;
use crate::keyboard::DecodedKey;
use crate::paint::{LockPalette, LockVisualState};
use crate::pointer::{hit_test, PointerGesture, PointerTarget};
use crate::prompt_label::PromptText;
use crate::registry_probe;
use crate::shm::{Error as ShmError, ShmFrame};
use crate::surface::{BufferId, Error as SurfaceError, SurfaceSet};
use crate::text_renderer::{Error as TextRendererError, LockTextRenderer};
use crate::xkb_keyboard::{Error as XkbError, KeyboardDecoder};
use rmac_lock_provider::{OutputId, UnlockAuthorization};

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
/// Required globals and outputs are bound and hotplug is tracked, but no public
/// method can issue the session-lock request.
pub struct PreparedConnection {
    connection: Connection,
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
            connection,
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

    /// Internal typestate transition. It is intentionally unavailable outside
    /// this crate until the provider runtime owns recovery and authentication.
    #[allow(dead_code)]
    pub(crate) fn acquire_for_runtime(mut self) -> Result<LockConnection, WireError> {
        let queue_handle = self.event_queue.handle();
        self.state.begin_lock(&queue_handle)?;
        self.event_queue.flush().map_err(WireError::Flush)?;
        Ok(LockConnection { inner: self })
    }
}

/// Live session-lock connection. No constructor is exported outside this
/// crate; this type exists so wire ordering can compile before product enablement.
#[allow(dead_code)]
pub(crate) struct LockConnection {
    inner: PreparedConnection,
}

#[allow(dead_code)]
impl LockConnection {
    pub(crate) fn dispatch_pending(&mut self) -> Result<usize, WireError> {
        let dispatched = self
            .inner
            .event_queue
            .dispatch_pending(&mut self.inner.state)
            .map_err(WireError::Dispatch)?;
        self.inner.state.check_wire_failure()?;
        let queue_handle = self.inner.event_queue.handle();
        self.inner.state.render_pending(&queue_handle)?;
        self.inner.event_queue.flush().map_err(WireError::Flush)?;
        Ok(dispatched)
    }

    pub(crate) fn blocking_dispatch(&mut self) -> Result<usize, WireError> {
        let dispatched = self
            .inner
            .event_queue
            .blocking_dispatch(&mut self.inner.state)
            .map_err(WireError::Dispatch)?;
        self.inner.state.check_wire_failure()?;
        let queue_handle = self.inner.event_queue.handle();
        self.inner.state.render_pending(&queue_handle)?;
        self.inner.event_queue.flush().map_err(WireError::Flush)?;
        Ok(dispatched)
    }

    /// Read and dispatch Wayland events for at most `timeout`, allowing the
    /// runtime to poll the PAM worker without blocking indefinitely.
    pub(crate) fn poll_dispatch(&mut self, timeout: Duration) -> Result<usize, WireError> {
        let now = Instant::now();
        self.inner.state.emit_due_repeats(now);
        let timeout = self.inner.state.repeat_wait(timeout, now);
        let dispatched = self.dispatch_pending()?;
        if dispatched > 0 {
            return Ok(dispatched);
        }
        let Some(guard) = self.inner.connection.prepare_read() else {
            return self.dispatch_pending();
        };
        let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd: self.inner.connection.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor references the live Wayland connection for this
        // call only, and the one-element array has the advertised length.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                self.inner.state.emit_due_repeats(Instant::now());
                return Ok(0);
            }
            return Err(WireError::Poll(error));
        }
        if result == 0 {
            self.inner.state.emit_due_repeats(Instant::now());
            return Ok(0);
        }
        if descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(WireError::Poll(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Wayland connection poll failed",
            )));
        }
        match guard.read() {
            Ok(_) => {}
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == io::ErrorKind::WouldBlock =>
            {
                self.inner.state.emit_due_repeats(Instant::now());
                return Ok(0);
            }
            Err(error) => return Err(WireError::Read(error)),
        }
        let dispatched = self.dispatch_pending()?;
        self.inner.state.emit_due_repeats(Instant::now());
        Ok(dispatched)
    }

    pub(crate) fn drain_events(&mut self) -> impl Iterator<Item = PreparedEvent> + '_ {
        self.inner.state.events.drain(..)
    }

    pub(crate) fn set_visual_state(
        &mut self,
        visual: LockVisualState,
        prompt: Option<PromptText<'_>>,
    ) {
        self.inner.state.set_visual_state(visual, prompt);
    }

    /// Send the authenticated unlock request and wait for a display-sync
    /// barrier before allowing the provider to exit.
    pub(crate) fn unlock_and_flush(
        &mut self,
        _authorization: UnlockAuthorization,
    ) -> Result<(), WireError> {
        self.inner.state.send_unlock()?;
        self.inner
            .event_queue
            .roundtrip(&mut self.inner.state)
            .map_err(WireError::Dispatch)?;
        self.inner.state.check_wire_failure()?;
        self.inner
            .state
            .events
            .push_back(PreparedEvent::UnlockFlushed);
        Ok(())
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
    PointerAvailabilityChanged {
        available: bool,
    },
    PointerFocusChanged {
        seat: SeatId,
        focused: bool,
    },
    PointerInput {
        seat: SeatId,
        input: DecodedKey,
    },
    LockAcquired,
    LockFinished,
    FrameCommitted(OutputId),
    UnlockFlushed,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct SeatId(NonZeroU64);

impl SeatId {
    pub(crate) fn new(value: u64) -> Option<Self> {
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
    proxy: T,
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
    repeat: RepeatScheduler<u32>,
    pointer: Option<PointerBinding>,
}

struct PointerBinding {
    proxy: wl_pointer::WlPointer,
    output: Option<OutputId>,
    position: Option<(f64, f64)>,
    gesture: PointerGesture,
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

#[derive(Clone, Copy)]
struct PointerData {
    seat_global_name: u32,
    seat: SeatId,
}

struct LockData;

#[derive(Clone, Copy)]
struct LockSurfaceData {
    output: OutputId,
}

#[derive(Clone, Copy)]
struct WireSurfaceData {
    output: OutputId,
}

#[derive(Clone, Copy)]
struct BufferData {
    buffer: BufferId,
}

struct LockingState {
    lock: ExtSessionLockV1,
    phase: WireLockPhase,
    lock_surfaces: BTreeMap<OutputId, LockSurfaceBinding>,
    buffers: BTreeMap<BufferId, BufferBinding>,
    pending_renders: BTreeSet<OutputId>,
    visual: LockVisualState,
}

struct LockSurfaceBinding {
    surface: wl_surface::WlSurface,
    role: ExtSessionLockSurfaceV1,
}

struct BufferBinding {
    proxy: wl_buffer::WlBuffer,
    _frame: ShmFrame,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WireLockPhase {
    AwaitingDecision,
    Locked,
    Finished,
    UnlockSent,
}

impl WireLockPhase {
    fn accepts_surfaces(self) -> bool {
        matches!(self, Self::AwaitingDecision | Self::Locked)
    }
}

struct PreparedState {
    compositor: Option<RequiredBinding<wl_compositor::WlCompositor>>,
    shm: Option<RequiredBinding<wl_shm::WlShm>>,
    manager: Option<RequiredBinding<ExtSessionLockManagerV1>>,
    outputs: BTreeMap<u32, OutputBinding>,
    seats: BTreeMap<u32, SeatBinding>,
    surfaces: SurfaceSet,
    events: VecDeque<PreparedEvent>,
    failure: Option<PreparedStateError>,
    locking: Option<LockingState>,
    text_renderer: LockTextRenderer,
}

impl Default for PreparedState {
    fn default() -> Self {
        Self {
            compositor: None,
            shm: None,
            manager: None,
            outputs: BTreeMap::new(),
            seats: BTreeMap::new(),
            surfaces: SurfaceSet::new(),
            events: VecDeque::new(),
            failure: None,
            locking: None,
            text_renderer: LockTextRenderer::default(),
        }
    }
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
                    proxy: registry.bind(
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
                    proxy: registry.bind(
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
                    proxy: registry.bind(
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
                if self
                    .locking
                    .as_ref()
                    .is_some_and(|locking| locking.phase.accepts_surfaces())
                {
                    if let Err(error) = self.create_lock_surface(output, queue_handle) {
                        match error {
                            WireError::State(failure) => {
                                self.record_failure(PreparedStateError::Wire(failure))
                            }
                            _ => self.record_failure(PreparedStateError::Wire(
                                WireStateError::UnexpectedWireFailure,
                            )),
                        }
                    }
                }
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
                        repeat: RepeatScheduler::default(),
                        pointer: None,
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
                let was_pointer_available = seat.pointer.is_some() || self.pointer_count() > 0;
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
                if let Some(pointer) = seat.pointer.take() {
                    if pointer.output.is_some() {
                        self.events.push_back(PreparedEvent::PointerFocusChanged {
                            seat: seat.seat,
                            focused: false,
                        });
                    }
                    if pointer.proxy.version() >= 3 {
                        pointer.proxy.release();
                    }
                }
                if seat.proxy.version() >= 5 {
                    seat.proxy.release();
                }
                let available = self.keyboard_count() > 0;
                if available != was_available {
                    self.events
                        .push_back(PreparedEvent::KeyboardAvailabilityChanged { available });
                }
                let pointer_available = self.pointer_count() > 0;
                if pointer_available != was_pointer_available {
                    self.events
                        .push_back(PreparedEvent::PointerAvailabilityChanged {
                            available: pointer_available,
                        });
                }
            }
            return;
        };
        self.destroy_lock_surface(binding.output);
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

    fn pointer_count(&self) -> usize {
        self.seats
            .values()
            .filter(|seat| seat.pointer.is_some())
            .count()
    }

    fn pointer_target(&self, seat_global_name: u32) -> Option<PointerTarget> {
        let pointer = self.seats.get(&seat_global_name)?.pointer.as_ref()?;
        let output = pointer.output?;
        let (x, y) = pointer.position?;
        let (width, height) = self.surfaces.logical_size(output)?;
        let visual = self.locking.as_ref()?.visual.prompt();
        hit_test(width, height, visual, x, y)
    }

    fn repeat_wait(&self, requested: Duration, now: Instant) -> Duration {
        self.seats
            .values()
            .filter_map(|seat| seat.repeat.deadline())
            .map(|deadline| deadline.saturating_duration_since(now))
            .min()
            .map_or(requested, |until_repeat| requested.min(until_repeat))
    }

    fn emit_due_repeats(&mut self, now: Instant) {
        let due = self
            .seats
            .values_mut()
            .filter_map(|seat| seat.repeat.take_due(now).map(|key| (seat.seat, key)))
            .collect::<Vec<_>>();
        for (seat_id, key) in due {
            let result = self
                .seats
                .values_mut()
                .find(|seat| seat.seat == seat_id)
                .map(|seat| catch_unwind(AssertUnwindSafe(|| seat.decoder.decode_press(key))));
            match result {
                Some(Ok(Ok(Some(input)))) => {
                    self.events.push_back(PreparedEvent::KeyboardInput {
                        seat: seat_id,
                        input,
                    });
                }
                Some(Ok(Ok(None))) => {}
                Some(Ok(Err(error))) => {
                    self.record_failure(PreparedStateError::Keyboard(error.into()));
                }
                Some(Err(_)) => self.record_failure(PreparedStateError::Keyboard(
                    KeyboardFailure::DecoderPanicked,
                )),
                None => {
                    self.record_failure(PreparedStateError::Keyboard(KeyboardFailure::SeatMissing))
                }
            }
        }
    }

    fn begin_lock(&mut self, queue_handle: &QueueHandle<Self>) -> Result<(), WireError> {
        if self.locking.is_some() {
            return Err(WireError::State(WireStateError::AlreadyLocking));
        }
        self.require_input_ready().map_err(WireError::Prepare)?;
        let manager = self
            .manager
            .as_ref()
            .ok_or(WireError::State(WireStateError::MissingRequiredBinding))?
            .proxy
            .clone();
        let lock = manager.lock(queue_handle, LockData);
        self.locking = Some(LockingState {
            lock,
            phase: WireLockPhase::AwaitingDecision,
            lock_surfaces: BTreeMap::new(),
            buffers: BTreeMap::new(),
            pending_renders: BTreeSet::new(),
            visual: LockVisualState::default(),
        });

        let outputs: Vec<_> = self
            .outputs
            .values()
            .map(|binding| binding.output)
            .collect();
        for output in outputs {
            self.create_lock_surface(output, queue_handle)?;
        }
        Ok(())
    }

    fn create_lock_surface(
        &mut self,
        output: OutputId,
        queue_handle: &QueueHandle<Self>,
    ) -> Result<(), WireError> {
        let compositor = self
            .compositor
            .as_ref()
            .ok_or(WireError::State(WireStateError::MissingRequiredBinding))?
            .proxy
            .clone();
        let output_proxy = self
            .outputs
            .values()
            .find(|binding| binding.output == output)
            .ok_or(WireError::State(WireStateError::MissingOutput))?
            .proxy
            .clone();
        let locking = self
            .locking
            .as_mut()
            .ok_or(WireError::State(WireStateError::NotLocking))?;
        if locking.lock_surfaces.contains_key(&output) {
            return Err(WireError::State(WireStateError::DuplicateLockSurface));
        }
        let surface = compositor.create_surface(queue_handle, WireSurfaceData { output });
        let role = locking.lock.get_lock_surface(
            &surface,
            &output_proxy,
            queue_handle,
            LockSurfaceData { output },
        );
        locking
            .lock_surfaces
            .insert(output, LockSurfaceBinding { surface, role });
        Ok(())
    }

    fn destroy_lock_surface(&mut self, output: OutputId) {
        let mut unfocused = Vec::new();
        for seat in self.seats.values_mut() {
            let Some(pointer) = &mut seat.pointer else {
                continue;
            };
            if pointer.output == Some(output) {
                pointer.output = None;
                pointer.position = None;
                pointer.gesture.clear();
                unfocused.push(seat.seat);
            }
        }
        self.events.extend(
            unfocused
                .into_iter()
                .map(|seat| PreparedEvent::PointerFocusChanged {
                    seat,
                    focused: false,
                }),
        );
        let Some(locking) = &mut self.locking else {
            return;
        };
        locking.pending_renders.remove(&output);
        if let Some(binding) = locking.lock_surfaces.remove(&output) {
            binding.role.destroy();
            binding.surface.destroy();
        }
    }

    fn destroy_all_lock_surfaces(&mut self) {
        let outputs = self
            .locking
            .as_ref()
            .map(|locking| locking.lock_surfaces.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for output in outputs {
            self.destroy_lock_surface(output);
        }
    }

    fn queue_render(&mut self, output: OutputId) {
        if let Some(locking) = &mut self.locking {
            if locking.phase.accepts_surfaces() && locking.lock_surfaces.contains_key(&output) {
                locking.pending_renders.insert(output);
            }
        }
    }

    fn set_visual_state(&mut self, visual: LockVisualState, prompt: Option<PromptText<'_>>) {
        let text_changed = self
            .text_renderer
            .update(prompt, visual.authentication_failed());
        let Some(locking) = &mut self.locking else {
            return;
        };
        if (locking.visual == visual && !text_changed) || !locking.phase.accepts_surfaces() {
            return;
        }
        locking.visual = visual;
        locking
            .pending_renders
            .extend(locking.lock_surfaces.keys().copied());
    }

    fn render_pending(&mut self, queue_handle: &QueueHandle<Self>) -> Result<(), WireError> {
        let pending = self
            .locking
            .as_mut()
            .map(|locking| std::mem::take(&mut locking.pending_renders))
            .unwrap_or_default();
        for output in pending {
            match self.render_output(output, queue_handle) {
                Ok(()) | Err(WireError::Surface(SurfaceError::AwaitingConfigure)) => {}
                Err(WireError::Surface(
                    SurfaceError::BufferBackpressure | SurfaceError::GlobalBufferBudget,
                )) => self.queue_render(output),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn render_output(
        &mut self,
        output: OutputId,
        queue_handle: &QueueHandle<Self>,
    ) -> Result<(), WireError> {
        let shm = self
            .shm
            .as_ref()
            .ok_or(WireError::State(WireStateError::MissingRequiredBinding))?
            .proxy
            .clone();
        let (wire_surface, role) = self
            .locking
            .as_ref()
            .ok_or(WireError::State(WireStateError::NotLocking))?
            .lock_surfaces
            .get(&output)
            .map(|binding| (binding.surface.clone(), binding.role.clone()))
            .ok_or(WireError::State(WireStateError::MissingLockSurface))?;
        let plan = self
            .surfaces
            .begin_render(output)
            .map_err(WireError::Surface)?;
        let buffer_id = plan.buffer();
        if self
            .locking
            .as_ref()
            .is_some_and(|locking| locking.buffers.contains_key(&buffer_id))
        {
            self.surfaces
                .abandon_render(plan)
                .map_err(WireError::Surface)?;
            return Err(WireError::State(WireStateError::DuplicateBuffer));
        }
        let visual = self
            .locking
            .as_ref()
            .map(|locking| locking.visual)
            .unwrap_or_default();
        let text = match self.text_renderer.raster(plan.layout()) {
            Ok(text) => text,
            Err(error) => {
                self.surfaces
                    .abandon_render(plan)
                    .map_err(WireError::Surface)?;
                return Err(WireError::Text(error));
            }
        };
        let frame = match ShmFrame::paint(&plan, LockPalette::MIDNIGHT, visual, text.as_ref()) {
            Ok(frame) => frame,
            Err(error) => {
                self.surfaces
                    .abandon_render(plan)
                    .map_err(WireError::Surface)?;
                return Err(WireError::Shm(error));
            }
        };
        let layout = frame.layout();
        let pool = shm.create_pool(frame.as_fd(), frame.pool_size(), queue_handle, ());
        let buffer = pool.create_buffer(
            0,
            layout.width() as i32,
            layout.height() as i32,
            layout.stride() as i32,
            wl_shm::Format::Argb8888,
            queue_handle,
            BufferData { buffer: buffer_id },
        );
        pool.destroy();

        let commit = match self.surfaces.commit_render(&plan) {
            Ok(commit) => commit,
            Err(error) => {
                buffer.destroy();
                let _ = self.surfaces.abandon_render(plan);
                return Err(WireError::Surface(error));
            }
        };
        if let Some(serial) = commit.ack_serial() {
            role.ack_configure(serial);
        }
        wire_surface.set_buffer_scale(commit.layout().scale() as i32);
        wire_surface.attach(Some(&buffer), 0, 0);
        wire_surface.damage_buffer(
            0,
            0,
            commit.layout().width() as i32,
            commit.layout().height() as i32,
        );
        let Some(locking) = &mut self.locking else {
            buffer.destroy();
            let _ = self.surfaces.release_buffer(buffer_id);
            return Err(WireError::State(WireStateError::NotLocking));
        };
        if locking.buffers.contains_key(&buffer_id) {
            buffer.destroy();
            let _ = self.surfaces.release_buffer(buffer_id);
            return Err(WireError::State(WireStateError::DuplicateBuffer));
        }
        locking.buffers.insert(
            buffer_id,
            BufferBinding {
                proxy: buffer,
                _frame: frame,
            },
        );
        wire_surface.commit();
        self.events.push_back(PreparedEvent::FrameCommitted(output));
        Ok(())
    }

    fn send_unlock(&mut self) -> Result<(), WireError> {
        let locking = self
            .locking
            .as_mut()
            .ok_or(WireError::State(WireStateError::NotLocking))?;
        if locking.phase != WireLockPhase::Locked {
            return Err(WireError::State(WireStateError::UnlockBeforeLocked));
        }
        locking.lock.unlock_and_destroy();
        locking.phase = WireLockPhase::UnlockSent;
        self.destroy_all_lock_surfaces();
        Ok(())
    }

    fn check_wire_failure(&self) -> Result<(), WireError> {
        match self.failure {
            Some(failure) => Err(WireError::PreparedState(failure)),
            None => Ok(()),
        }
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
                Ok(true) => {
                    state.events.push_back(PreparedEvent::OutputScaleChanged {
                        output: data.output,
                        scale,
                    });
                    state.queue_render(data.output);
                }
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
        let was_pointer_available = state.pointer_count() > 0;
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
            binding.repeat.clear();
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
        if capabilities.contains(wl_seat::Capability::Pointer) {
            if binding.pointer.is_none() {
                binding.pointer = Some(PointerBinding {
                    proxy: seat.get_pointer(
                        queue_handle,
                        PointerData {
                            seat_global_name: data.global_name,
                            seat: data.seat,
                        },
                    ),
                    output: None,
                    position: None,
                    gesture: PointerGesture::default(),
                });
            }
        } else if let Some(pointer) = binding.pointer.take() {
            if pointer.output.is_some() {
                state.events.push_back(PreparedEvent::PointerFocusChanged {
                    seat: binding.seat,
                    focused: false,
                });
            }
            if pointer.proxy.version() >= 3 {
                pointer.proxy.release();
            }
        }
        let available = state.keyboard_count() > 0;
        if available != was_available {
            state
                .events
                .push_back(PreparedEvent::KeyboardAvailabilityChanged { available });
        }
        let pointer_available = state.pointer_count() > 0;
        if pointer_available != was_pointer_available {
            state
                .events
                .push_back(PreparedEvent::PointerAvailabilityChanged {
                    available: pointer_available,
                });
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, PointerData> for PreparedState {
    fn event(
        state: &mut Self,
        _pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        data: &PointerData,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        const BTN_LEFT: u32 = 0x110;

        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                let Some(output) = surface.data::<WireSurfaceData>().map(|data| data.output) else {
                    state.record_failure(PreparedStateError::Pointer(
                        PointerFailure::UnknownSurface,
                    ));
                    return;
                };
                if !state.locking.as_ref().is_some_and(|locking| {
                    locking.phase.accepts_surfaces() && locking.lock_surfaces.contains_key(&output)
                }) {
                    return;
                }
                let Some(binding) = state.seats.get_mut(&data.seat_global_name) else {
                    state.record_failure(PreparedStateError::Pointer(PointerFailure::SeatMissing));
                    return;
                };
                let Some(pointer) = &mut binding.pointer else {
                    state.record_failure(PreparedStateError::Pointer(
                        PointerFailure::CapabilityMissing,
                    ));
                    return;
                };
                pointer.output = Some(output);
                pointer.position = Some((surface_x, surface_y));
                pointer.gesture.clear();
                state.events.push_back(PreparedEvent::PointerFocusChanged {
                    seat: data.seat,
                    focused: true,
                });
            }
            wl_pointer::Event::Leave { .. } => {
                let Some(binding) = state.seats.get_mut(&data.seat_global_name) else {
                    state.record_failure(PreparedStateError::Pointer(PointerFailure::SeatMissing));
                    return;
                };
                let Some(pointer) = &mut binding.pointer else {
                    state.record_failure(PreparedStateError::Pointer(
                        PointerFailure::CapabilityMissing,
                    ));
                    return;
                };
                let was_focused = pointer.output.take().is_some();
                pointer.position = None;
                pointer.gesture.clear();
                if was_focused {
                    state.events.push_back(PreparedEvent::PointerFocusChanged {
                        seat: data.seat,
                        focused: false,
                    });
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                let Some(pointer) = state
                    .seats
                    .get_mut(&data.seat_global_name)
                    .and_then(|binding| binding.pointer.as_mut())
                else {
                    state.record_failure(PreparedStateError::Pointer(
                        PointerFailure::CapabilityMissing,
                    ));
                    return;
                };
                if pointer.output.is_some() {
                    pointer.position = Some((surface_x, surface_y));
                }
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                ..
            } => {
                let WEnum::Value(button_state) = button_state else {
                    state.record_failure(PreparedStateError::Pointer(
                        PointerFailure::InvalidButtonState,
                    ));
                    return;
                };
                if button != BTN_LEFT {
                    return;
                }
                let target = state.pointer_target(data.seat_global_name);
                let Some(pointer) = state
                    .seats
                    .get_mut(&data.seat_global_name)
                    .and_then(|binding| binding.pointer.as_mut())
                else {
                    state.record_failure(PreparedStateError::Pointer(
                        PointerFailure::CapabilityMissing,
                    ));
                    return;
                };
                match button_state {
                    wl_pointer::ButtonState::Pressed => pointer.gesture.press(target),
                    wl_pointer::ButtonState::Released => {
                        if let Some(input) = pointer.gesture.release(target) {
                            state.events.push_back(PreparedEvent::PointerInput {
                                seat: data.seat,
                                input,
                            });
                        }
                    }
                    _ => state.record_failure(PreparedStateError::Pointer(
                        PointerFailure::InvalidButtonState,
                    )),
                }
            }
            _ => {}
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
                    binding.repeat.clear();
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
                binding.repeat.clear();
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
                WEnum::Value(wl_keyboard::KeyState::Pressed) => {
                    let repeatable =
                        match catch_unwind(AssertUnwindSafe(|| binding.decoder.key_repeats(key))) {
                            Ok(Ok(repeatable)) => repeatable,
                            Ok(Err(error)) => {
                                state.record_failure(PreparedStateError::Keyboard(error.into()));
                                return;
                            }
                            Err(_) => {
                                state.record_failure(PreparedStateError::Keyboard(
                                    KeyboardFailure::DecoderPanicked,
                                ));
                                return;
                            }
                        };
                    binding.repeat.pressed(key, repeatable, Instant::now());
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
                WEnum::Value(wl_keyboard::KeyState::Repeated) => {
                    binding.repeat.clear();
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
                WEnum::Value(wl_keyboard::KeyState::Released) => binding.repeat.released(key),
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
                let config = binding.repeat.configure(rate, delay_ms, Instant::now());
                state.events.push_back(PreparedEvent::KeyboardRepeat {
                    seat: data.seat,
                    rate: config.rate(),
                    delay_ms: config.delay_ms(),
                });
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockV1, LockData> for PreparedState {
    fn event(
        state: &mut Self,
        lock: &ExtSessionLockV1,
        event: wayland_protocols::ext::session_lock::v1::client::ext_session_lock_v1::Event,
        _data: &LockData,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        let Some(locking) = &mut state.locking else {
            state.record_failure(PreparedStateError::Wire(WireStateError::NotLocking));
            return;
        };
        match event {
            wayland_protocols::ext::session_lock::v1::client::ext_session_lock_v1::Event::Locked
                if locking.phase == WireLockPhase::AwaitingDecision =>
            {
                locking.phase = WireLockPhase::Locked;
                state.events.push_back(PreparedEvent::LockAcquired);
            }
            wayland_protocols::ext::session_lock::v1::client::ext_session_lock_v1::Event::Finished
                if matches!(
                    locking.phase,
                    WireLockPhase::AwaitingDecision | WireLockPhase::Locked
                ) =>
            {
                let denied = locking.phase == WireLockPhase::AwaitingDecision;
                locking.phase = WireLockPhase::Finished;
                locking.pending_renders.clear();
                state.events.push_back(PreparedEvent::LockFinished);
                if denied {
                    lock.destroy();
                    state.destroy_all_lock_surfaces();
                }
            }
            _ => state.record_failure(PreparedStateError::Wire(
                WireStateError::InvalidLockEvent,
            )),
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, LockSurfaceData> for PreparedState {
    fn event(
        state: &mut Self,
        _role: &ExtSessionLockSurfaceV1,
        event: wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::Event,
        data: &LockSurfaceData,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if !state.locking.as_ref().is_some_and(|locking| {
            locking.phase.accepts_surfaces() && locking.lock_surfaces.contains_key(&data.output)
        }) {
            return;
        }
        if let wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            match state.surfaces.configure(data.output, serial, width, height) {
                Ok(()) => state.queue_render(data.output),
                Err(error) => state.record_failure(PreparedStateError::Surface(error)),
            }
        }
    }
}

impl Dispatch<wl_surface::WlSurface, WireSurfaceData> for PreparedState {
    fn event(
        state: &mut Self,
        _surface: &wl_surface::WlSurface,
        event: wl_surface::Event,
        data: &WireSurfaceData,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if !state.locking.as_ref().is_some_and(|locking| {
            locking.phase.accepts_surfaces() && locking.lock_surfaces.contains_key(&data.output)
        }) {
            return;
        }
        if let wl_surface::Event::PreferredBufferScale { factor } = event {
            let Ok(scale) = u32::try_from(factor) else {
                state.record_failure(PreparedStateError::InvalidOutputScale);
                return;
            };
            match state.surfaces.set_scale(data.output, scale) {
                Ok(true) => state.queue_render(data.output),
                Ok(false) => {}
                Err(error) => state.record_failure(PreparedStateError::Surface(error)),
            }
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, BufferData> for PreparedState {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        data: &BufferData,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if !matches!(event, wl_buffer::Event::Release) {
            return;
        }
        let Some(locking) = &mut state.locking else {
            state.record_failure(PreparedStateError::Wire(WireStateError::NotLocking));
            return;
        };
        let Some(binding) = locking.buffers.remove(&data.buffer) else {
            state.record_failure(PreparedStateError::Surface(SurfaceError::UnknownBuffer));
            return;
        };
        if binding.proxy != *buffer {
            state.record_failure(PreparedStateError::Wire(
                WireStateError::BufferIdentityMismatch,
            ));
            return;
        }
        binding.proxy.destroy();
        if let Err(error) = state.surfaces.release_buffer(data.buffer) {
            state.record_failure(PreparedStateError::Surface(error));
        }
    }
}

delegate_noop!(PreparedState: ignore wl_compositor::WlCompositor);
delegate_noop!(PreparedState: ignore wl_shm::WlShm);
delegate_noop!(PreparedState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(PreparedState: ignore ExtSessionLockManagerV1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedStateError {
    InvalidOutputId,
    InvalidSeatId,
    InvalidOutputScale,
    RequiredGlobalRemoved,
    Surface(SurfaceError),
    Keyboard(KeyboardFailure),
    Pointer(PointerFailure),
    Wire(WireStateError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerFailure {
    SeatMissing,
    CapabilityMissing,
    UnknownSurface,
    InvalidButtonState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireStateError {
    AlreadyLocking,
    NotLocking,
    MissingRequiredBinding,
    MissingOutput,
    DuplicateLockSurface,
    MissingLockSurface,
    DuplicateBuffer,
    BufferIdentityMismatch,
    UnlockBeforeLocked,
    InvalidLockEvent,
    UnexpectedWireFailure,
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

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum WireError {
    Prepare(PrepareError),
    Dispatch(wayland_client::DispatchError),
    Flush(wayland_client::backend::WaylandError),
    Read(wayland_client::backend::WaylandError),
    Poll(io::Error),
    PreparedState(PreparedStateError),
    State(WireStateError),
    Surface(SurfaceError),
    Shm(ShmError),
    Text(TextRendererError),
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("secure Wayland lock wire transition failed")
    }
}

impl std::error::Error for WireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Prepare(error) => Some(error),
            Self::Dispatch(error) => Some(error),
            Self::Flush(error) => Some(error),
            Self::Read(error) => Some(error),
            Self::Poll(error) => Some(error),
            Self::Surface(error) => Some(error),
            Self::Shm(error) => Some(error),
            Self::Text(error) => Some(error),
            Self::PreparedState(_) | Self::State(_) => None,
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
        assert_eq!(ExtSessionLockV1::interface().name, "ext_session_lock_v1");
        assert_eq!(
            ExtSessionLockSurfaceV1::interface().name,
            "ext_session_lock_surface_v1"
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
