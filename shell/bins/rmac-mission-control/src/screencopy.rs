//! In-process output capture for Mission Control (SPEED-03).
//!
//! Spawning `grim` for every open cost a process start, a fresh Wayland
//! connection, and a whole-output PPM through a pipe before the overlay could
//! map. The service now keeps one Wayland connection open on its own thread
//! and asks niri's wlr-screencopy for the output directly into a reused
//! shared-memory buffer. The picture is the same as `grim -o OUTPUT`'s
//! (no cursor, the output's physical pixels, top row first). When the
//! protocol, the output or the buffer format is unavailable, the caller falls
//! back to `grim` (`capture::grab_output`).
//!
//! Idle cost: the thread blocks on its request channel; nothing polls and the
//! connection is only read while a capture is in flight.

use std::io;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use crate::capture::Picture;

/// How long a caller waits for one capture before using `grim` instead.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

type Request = (String, mpsc::Sender<io::Result<Picture>>);

static REQUESTS: OnceLock<Mutex<Option<mpsc::Sender<Request>>>> = OnceLock::new();

/// Connect and start the capture thread. Call once when the service starts,
/// so the connection is ready before the first open. Does nothing (and the
/// caller keeps using `grim`) if the compositor lacks wlr-screencopy.
pub fn start() {
    let slot = REQUESTS.get_or_init(|| Mutex::new(None));
    let Ok(mut capturer) = Capturer::connect() else {
        return;
    };
    let (sender, receiver) = mpsc::channel::<Request>();
    let spawned = std::thread::Builder::new()
        .name("mc-screencopy".into())
        .spawn(move || {
            while let Ok((output, reply)) = receiver.recv() {
                let _ = reply.send(capturer.capture(&output));
            }
        });
    if spawned.is_ok() {
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(sender);
    }
}

/// Capture `output` (a compositor output name) through the open connection.
pub fn capture(output: &str) -> io::Result<Picture> {
    let sender = REQUESTS
        .get()
        .and_then(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no screencopy connection"))?;
    let (reply, result) = mpsc::channel();
    sender
        .send((output.to_owned(), reply))
        .map_err(|_| io::Error::other("the screencopy thread stopped"))?;
    result
        .recv_timeout(CAPTURE_TIMEOUT)
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "screencopy timed out"))?
}

struct Output {
    global: u32,
    output: wl_output::WlOutput,
    name: Option<String>,
}

#[derive(Default)]
struct Frame {
    /// (format, width, height, stride) of the shared-memory buffer offered.
    shm: Option<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    y_invert: bool,
    ready: bool,
    failed: bool,
}

#[derive(Default)]
struct State {
    outputs: Vec<Output>,
    shm: Option<wl_shm::WlShm>,
    manager: Option<(ZwlrScreencopyManagerV1, u32)>,
    frame: Frame,
}

/// One reusable shared-memory buffer.
struct Buffer {
    key: (wl_shm::Format, u32, u32, u32),
    memory: *mut u8,
    length: usize,
    _fd: OwnedFd,
    _pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
}

// Safety: the mapping is owned by this struct and only touched on the
// capture thread that owns the `Capturer`.
unsafe impl Send for Buffer {}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        // Safety: `memory`/`length` are exactly what `mmap` returned.
        unsafe { libc::munmap(self.memory.cast(), self.length) };
    }
}

struct Capturer {
    _connection: Connection,
    queue: EventQueue<State>,
    state: State,
    buffer: Option<Buffer>,
}

impl Capturer {
    fn connect() -> io::Result<Self> {
        let connection = Connection::connect_to_env().map_err(io::Error::other)?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut state = State::default();
        // Globals, then the outputs' names.
        queue.roundtrip(&mut state).map_err(io::Error::other)?;
        queue.roundtrip(&mut state).map_err(io::Error::other)?;
        if state.manager.is_none() || state.shm.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the compositor offers no wlr-screencopy",
            ));
        }
        Ok(Self {
            _connection: connection,
            queue,
            state,
            buffer: None,
        })
    }

    fn dispatch_until(&mut self, done: impl Fn(&Frame) -> bool) -> io::Result<()> {
        while !done(&self.state.frame) && !self.state.frame.failed {
            self.queue
                .blocking_dispatch(&mut self.state)
                .map_err(io::Error::other)?;
        }
        if self.state.frame.failed {
            return Err(io::Error::other("the compositor could not copy the output"));
        }
        Ok(())
    }

    fn capture(&mut self, output_name: &str) -> io::Result<Picture> {
        // Pick up output changes that arrived while idle.
        self.queue
            .roundtrip(&mut self.state)
            .map_err(io::Error::other)?;
        let handle = self.queue.handle();
        let output = self
            .state
            .outputs
            .iter()
            .find(|output| output.name.as_deref() == Some(output_name))
            .map(|output| output.output.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such output"))?;
        let (manager, version) = self.state.manager.clone().expect("checked in connect");
        self.state.frame = Frame::default();
        let frame = manager.capture_output(0, &output, &handle, ());
        let result = self.copy(&frame, version, &handle);
        frame.destroy();
        result
    }

    fn copy(
        &mut self,
        frame: &ZwlrScreencopyFrameV1,
        version: u32,
        handle: &QueueHandle<State>,
    ) -> io::Result<Picture> {
        // Version 3 lists every buffer type and then sends buffer_done;
        // older versions send just the shared-memory one.
        if version >= 3 {
            self.dispatch_until(|frame| frame.buffer_done)?;
        } else {
            self.dispatch_until(|frame| frame.shm.is_some())?;
        }
        let key = self
            .state
            .frame
            .shm
            .ok_or_else(|| io::Error::other("no shared-memory buffer offered"))?;
        if !matches!(
            key.0,
            wl_shm::Format::Xrgb8888
                | wl_shm::Format::Argb8888
                | wl_shm::Format::Xbgr8888
                | wl_shm::Format::Abgr8888
        ) {
            return Err(io::Error::other("unsupported screencopy format"));
        }
        if self.buffer.as_ref().is_none_or(|buffer| buffer.key != key) {
            self.buffer = None;
            self.buffer = Some(self.allocate(key, handle)?);
        }
        let buffer = self.buffer.as_ref().expect("allocated above");
        frame.copy(&buffer.buffer);
        self.dispatch_until(|frame| frame.ready)?;
        let buffer = self.buffer.as_ref().expect("allocated above");
        // Safety: the compositor finished writing (`ready`), and the mapping
        // stays valid while `buffer` lives.
        let pixels = unsafe { std::slice::from_raw_parts(buffer.memory, buffer.length) };
        Ok(to_rgb(pixels, key, self.state.frame.y_invert))
    }

    fn allocate(
        &self,
        key: (wl_shm::Format, u32, u32, u32),
        handle: &QueueHandle<State>,
    ) -> io::Result<Buffer> {
        let (format, width, height, stride) = key;
        let length = stride as usize * height as usize;
        // Safety: plain syscalls; every result is checked.
        let fd = unsafe {
            let raw = libc::memfd_create(c"rmac-mission-control".as_ptr(), libc::MFD_CLOEXEC);
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            OwnedFd::from_raw_fd(raw)
        };
        use std::os::fd::AsRawFd;
        if unsafe { libc::ftruncate(fd.as_raw_fd(), length as libc::off_t) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let memory = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if memory == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let shm = self.state.shm.as_ref().expect("checked in connect");
        let pool = shm.create_pool(fd.as_fd(), length as i32, handle, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            format,
            handle,
            (),
        );
        Ok(Buffer {
            key,
            memory: memory.cast(),
            length,
            _fd: fd,
            _pool: pool,
            buffer,
        })
    }
}

/// Convert a 32-bit little-endian shm picture to `Picture`'s RGB rows,
/// flipping it when the compositor wrote it bottom row first.
fn to_rgb(pixels: &[u8], key: (wl_shm::Format, u32, u32, u32), y_invert: bool) -> Picture {
    let (format, width, height, stride) = key;
    // Memory order of the 32-bit little-endian formats.
    let bgr = matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888);
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for row in 0..height as usize {
        let source = if y_invert {
            height as usize - 1 - row
        } else {
            row
        };
        let start = source * stride as usize;
        for pixel in pixels[start..start + width as usize * 4].chunks_exact(4) {
            if bgr {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            } else {
                rgb.extend_from_slice(&[pixel[0], pixel[1], pixel[2]]);
            }
        }
    }
    Picture { width, height, rgb }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_output" if version >= 4 => {
                    let output = registry.bind(name, 4, handle, ());
                    state.outputs.push(Output {
                        global: name,
                        output,
                        name: None,
                    });
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, handle, ())),
                "zwlr_screencopy_manager_v1" => {
                    let version = version.min(3);
                    state.manager = Some((registry.bind(name, version, handle, ()), version));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.retain(|output| output.global != name);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(entry) = state
                .outputs
                .iter_mut()
                .find(|entry| &entry.output == output)
            {
                entry.name = Some(name);
            }
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => state.frame.shm = Some((format, width, height, stride)),
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                state.frame.y_invert = matches!(
                    flags,
                    WEnum::Value(flags) if flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert)
                );
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => state.frame.buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.frame.ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => state.frame.failed = true,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(State: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(State: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(State: ZwlrScreencopyManagerV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_xrgb_rows_and_honours_y_invert() {
        // 2x2, stride 12 (one pad pixel per row), XRGB8888 little-endian:
        // memory order B, G, R, X.
        let mut pixels = Vec::new();
        for row in [[1u8, 2, 3], [4, 5, 6]] {
            for _ in 0..2 {
                pixels.extend_from_slice(&[row[2], row[1], row[0], 0xFF]);
            }
            pixels.extend_from_slice(&[0, 0, 0, 0]);
        }
        let key = (wl_shm::Format::Xrgb8888, 2, 2, 12);
        let upright = to_rgb(&pixels, key, false);
        assert_eq!((upright.width, upright.height), (2, 2));
        assert_eq!(upright.rgb, [1, 2, 3, 1, 2, 3, 4, 5, 6, 4, 5, 6]);
        let flipped = to_rgb(&pixels, key, true);
        assert_eq!(flipped.rgb, [4, 5, 6, 4, 5, 6, 1, 2, 3, 1, 2, 3]);
    }

    #[test]
    fn converts_xbgr_without_swapping() {
        let pixels = [10u8, 20, 30, 0];
        let picture = to_rgb(&pixels, (wl_shm::Format::Xbgr8888, 1, 1, 4), false);
        assert_eq!(picture.rgb, [10, 20, 30]);
    }
}
