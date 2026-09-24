//! libmpv, loaded at run time.
//!
//! Why libmpv rather than GStreamer: Ubuntu's libmpv2 links FFmpeg, so
//! H.264/HEVC/AAC/MP3/FLAC/Opus/VP9/AV1 all decode without the
//! "restricted extras" GStreamer needs, hardware decoding (VA-API/VDPAU)
//! is one option, and seeking, A/V sync and PipeWire output are mpv's own
//! well-tested code. GPUI has no GL/Vulkan texture sharing, so frames come
//! from mpv's software render API ("sw") at the displayed size (capped at
//! 1080p), with hardware decoding copied back ("auto-copy-safe").
//!
//! The library is opened with `dlopen` so the build needs no mpv headers
//! and a missing `libmpv2` shows a message instead of failing to start.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use libloading::Library;

const LIBRARY_NAMES: [&str; 3] = ["libmpv.so.2", "libmpv.so.1", "libmpv.so"];

// mpv_format
const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;
// mpv_event_id
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PROPERTY_CHANGE: c_int = 22;
// mpv_render_param_type
const RENDER_PARAM_INVALID: c_int = 0;
const RENDER_PARAM_API_TYPE: c_int = 1;
const RENDER_PARAM_SW_SIZE: c_int = 17;
const RENDER_PARAM_SW_FORMAT: c_int = 18;
const RENDER_PARAM_SW_STRIDE: c_int = 19;
const RENDER_PARAM_SW_POINTER: c_int = 20;
const RENDER_UPDATE_FRAME: u64 = 1;
// mpv_end_file_reason
const END_FILE_EOF: c_int = 0;
const END_FILE_ERROR: c_int = 4;

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct RawEndFile {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

type UpdateCallback = unsafe extern "C" fn(*mut c_void);

struct Api {
    _library: Library,
    create: unsafe extern "C" fn() -> *mut c_void,
    initialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    terminate_destroy: unsafe extern "C" fn(*mut c_void),
    set_option_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    command: unsafe extern "C" fn(*mut c_void, *mut *const c_char) -> c_int,
    observe_property: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
    wait_event: unsafe extern "C" fn(*mut c_void, f64) -> *mut RawEvent,
    wakeup: unsafe extern "C" fn(*mut c_void),
    render_context_create:
        unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut RenderParam) -> c_int,
    render_context_set_update_callback:
        unsafe extern "C" fn(*mut c_void, Option<UpdateCallback>, *mut c_void),
    render_context_update: unsafe extern "C" fn(*mut c_void) -> u64,
    render_context_render: unsafe extern "C" fn(*mut c_void, *mut RenderParam) -> c_int,
    render_context_free: unsafe extern "C" fn(*mut c_void),
}

impl Api {
    fn load() -> Result<Self, String> {
        let library = LIBRARY_NAMES
            .iter()
            // SAFETY: loading libmpv runs only its own initialisers.
            .find_map(|name| unsafe { Library::new(name) }.ok())
            .ok_or_else(|| "Media Player needs libmpv. Install the libmpv2 package.".to_owned())?;
        macro_rules! symbol {
            ($name:literal) => {
                // SAFETY: the signatures match mpv/client.h and mpv/render.h
                // (client API 2.x).
                *unsafe { library.get(concat!($name, "\0").as_bytes()) }
                    .map_err(|_| format!("libmpv lacks {}", $name))?
            };
        }
        Ok(Self {
            create: symbol!("mpv_create"),
            initialize: symbol!("mpv_initialize"),
            terminate_destroy: symbol!("mpv_terminate_destroy"),
            set_option_string: symbol!("mpv_set_option_string"),
            set_property_string: symbol!("mpv_set_property_string"),
            command: symbol!("mpv_command"),
            observe_property: symbol!("mpv_observe_property"),
            wait_event: symbol!("mpv_wait_event"),
            wakeup: symbol!("mpv_wakeup"),
            render_context_create: symbol!("mpv_render_context_create"),
            render_context_set_update_callback: symbol!("mpv_render_context_set_update_callback"),
            render_context_update: symbol!("mpv_render_context_update"),
            render_context_render: symbol!("mpv_render_context_render"),
            render_context_free: symbol!("mpv_render_context_free"),
            _library: library,
        })
    }
}

/// What the player reports back to the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    TimePosition(f64),
    Duration(f64),
    Paused(bool),
    Volume(f64),
    Muted(bool),
    Title(String),
    Artist(String),
    Album(String),
    VideoSize(f64, f64),
    FileLoaded,
    /// The file ended by itself (true) or failed (false).
    Ended(bool),
    Shutdown,
}

const OBSERVED: [(&str, c_int); 9] = [
    ("time-pos", FORMAT_DOUBLE),
    ("duration", FORMAT_DOUBLE),
    ("pause", FORMAT_FLAG),
    ("volume", FORMAT_DOUBLE),
    ("mute", FORMAT_FLAG),
    ("media-title", FORMAT_STRING),
    ("metadata/by-key/artist", FORMAT_STRING),
    ("metadata/by-key/album", FORMAT_STRING),
    ("video-params/dw", FORMAT_INT64),
];
const HEIGHT_PROPERTY: &str = "video-params/dh";

/// A rendered frame in GPUI's BGRA layout, opaque.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Shared between the render thread and the window.
struct RenderShared {
    /// Packed (width << 32 | height) target size in pixels; 0 = no video.
    target: AtomicU64,
    /// Set by mpv's update callback; the render thread waits on it.
    dirty: Mutex<bool>,
    wake: Condvar,
    stop: AtomicBool,
    latest: Mutex<Option<Frame>>,
}

struct Handle(*mut c_void);
// SAFETY: the mpv client API is thread-safe for one handle; the render
// context is used only from the render thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

pub struct Player {
    api: Arc<Api>,
    handle: Arc<Handle>,
    render: Arc<RenderShared>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Player {
    /// Start mpv; events arrive on `events`, and `frames` is signalled when
    /// a new frame is ready (collect it with [`Player::take_frame`]).
    pub fn start(
        events: async_channel::Sender<Event>,
        frames: async_channel::Sender<()>,
    ) -> Result<Self, String> {
        let api = Arc::new(Api::load()?);
        // SAFETY: plain constructor.
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("libmpv could not start.".into());
        }
        let handle = Arc::new(Handle(handle));
        let set_option = |name: &str, value: &str| {
            let (name, value) = (cstring(name), cstring(value));
            // SAFETY: valid handle and NUL-terminated strings.
            unsafe { (api.set_option_string)(handle.0, name.as_ptr(), value.as_ptr()) };
        };
        for (name, value) in [
            ("vo", "libmpv"),
            ("hwdec", "auto-copy-safe"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("config", "no"),
            ("terminal", "no"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("ytdl", "no"),
            ("audio-client-name", "rmac-player"),
            ("title", "Media Player"),
        ] {
            set_option(name, value);
        }
        // SAFETY: valid handle.
        if unsafe { (api.initialize)(handle.0) } < 0 {
            // SAFETY: the handle is not used again.
            unsafe { (api.terminate_destroy)(handle.0) };
            return Err("libmpv could not start.".into());
        }
        for (index, (name, format)) in OBSERVED
            .iter()
            .copied()
            .chain([(HEIGHT_PROPERTY, FORMAT_INT64)])
            .enumerate()
        {
            let name = cstring(name);
            // SAFETY: valid handle and property name.
            unsafe { (api.observe_property)(handle.0, index as u64, name.as_ptr(), format) };
        }
        let render = Arc::new(RenderShared {
            target: AtomicU64::new(0),
            dirty: Mutex::new(false),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            latest: Mutex::new(None),
        });
        let mut threads = Vec::new();
        {
            let (api, handle) = (api.clone(), handle.clone());
            threads.push(std::thread::spawn(move || {
                event_loop(&api, &handle, &events)
            }));
        }
        {
            let (api, handle, render) = (api.clone(), handle.clone(), render.clone());
            threads.push(std::thread::spawn(move || {
                render_loop(&api, &handle, &render, &frames)
            }));
        }
        Ok(Self {
            api,
            handle,
            render,
            threads,
        })
    }

    pub fn command(&self, arguments: &[&str]) {
        let owned = arguments
            .iter()
            .map(|argument| cstring(argument))
            .collect::<Vec<_>>();
        let mut pointers = owned.iter().map(|value| value.as_ptr()).collect::<Vec<_>>();
        pointers.push(std::ptr::null());
        // SAFETY: NULL-terminated array of valid C strings.
        unsafe { (self.api.command)(self.handle.0, pointers.as_mut_ptr()) };
    }

    pub fn set(&self, name: &str, value: &str) {
        let (name, value) = (cstring(name), cstring(value));
        // SAFETY: valid handle and strings.
        unsafe { (self.api.set_property_string)(self.handle.0, name.as_ptr(), value.as_ptr()) };
    }

    pub fn load(&self, path: &std::path::Path) {
        self.command(&["loadfile", &path.to_string_lossy(), "replace"]);
        self.set("pause", "no");
    }

    pub fn set_paused(&self, paused: bool) {
        self.set("pause", if paused { "yes" } else { "no" });
    }

    pub fn seek_to(&self, seconds: f64) {
        self.command(&["seek", &format!("{:.3}", seconds.max(0.0)), "absolute"]);
    }

    pub fn set_volume(&self, volume: f64) {
        self.set("volume", &format!("{:.1}", volume.clamp(0.0, 100.0)));
    }

    pub fn set_muted(&self, muted: bool) {
        self.set("mute", if muted { "yes" } else { "no" });
    }

    /// Size in device pixels the next frames should be rendered at.
    pub fn set_render_size(&self, size: Option<(u32, u32)>) {
        let packed = size.map_or(0, |(width, height)| {
            u64::from(width) << 32 | u64::from(height)
        });
        if self.render.target.swap(packed, Ordering::AcqRel) != packed {
            mark_dirty(&self.render);
        }
    }

    pub fn take_frame(&self) -> Option<Frame> {
        self.render
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.render.stop.store(true, Ordering::Release);
        mark_dirty(&self.render);
        self.command(&["quit"]);
        // SAFETY: wakes the event thread so it sees the shutdown.
        unsafe { (self.api.wakeup)(self.handle.0) };
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        // SAFETY: both threads have finished with the handle.
        unsafe { (self.api.terminate_destroy)(self.handle.0) };
    }
}

fn cstring(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

fn mark_dirty(render: &RenderShared) {
    let mut dirty = render
        .dirty
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *dirty = true;
    render.wake.notify_one();
}

fn event_loop(api: &Api, handle: &Handle, events: &async_channel::Sender<Event>) {
    let mut width = 0.0;
    let mut height = 0.0;
    loop {
        // SAFETY: valid handle; the event stays valid until the next wait.
        let event = unsafe { &*(api.wait_event)(handle.0, -1.0) };
        let message = match event.event_id {
            EVENT_SHUTDOWN => {
                let _ = events.send_blocking(Event::Shutdown);
                return;
            }
            EVENT_FILE_LOADED => Some(Event::FileLoaded),
            EVENT_END_FILE if !event.data.is_null() => {
                // SAFETY: END_FILE carries an mpv_event_end_file.
                let end = unsafe { &*(event.data as *const RawEndFile) };
                match end.reason {
                    END_FILE_EOF => Some(Event::Ended(true)),
                    END_FILE_ERROR => Some(Event::Ended(false)),
                    _ => None,
                }
            }
            EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
                // SAFETY: PROPERTY_CHANGE carries an mpv_event_property.
                let property = unsafe { &*(event.data as *const RawProperty) };
                // SAFETY: mpv passes a NUL-terminated name.
                let name = unsafe { CStr::from_ptr(property.name) }.to_string_lossy();
                property_event(&name, property, &mut width, &mut height)
            }
            _ => None,
        };
        if let Some(message) = message {
            if events.send_blocking(message).is_err() {
                return;
            }
        }
    }
}

fn property_event(
    name: &str,
    property: &RawProperty,
    width: &mut f64,
    height: &mut f64,
) -> Option<Event> {
    if property.data.is_null() {
        return match name {
            "video-params/dw" | "video-params/dh" => {
                *width = 0.0;
                *height = 0.0;
                Some(Event::VideoSize(0.0, 0.0))
            }
            _ => None,
        };
    }
    // SAFETY: the data matches the format mpv reports for this change.
    let double = || unsafe { *(property.data as *const f64) };
    let flag = || unsafe { *(property.data as *const c_int) != 0 };
    let int = || unsafe { *(property.data as *const i64) } as f64;
    let text = || {
        // SAFETY: FORMAT_STRING data is a `char **`.
        let pointer = unsafe { *(property.data as *const *const c_char) };
        if pointer.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(pointer) }
                .to_string_lossy()
                .into_owned()
        }
    };
    match (name, property.format) {
        ("time-pos", FORMAT_DOUBLE) => Some(Event::TimePosition(double())),
        ("duration", FORMAT_DOUBLE) => Some(Event::Duration(double())),
        ("pause", FORMAT_FLAG) => Some(Event::Paused(flag())),
        ("volume", FORMAT_DOUBLE) => Some(Event::Volume(double())),
        ("mute", FORMAT_FLAG) => Some(Event::Muted(flag())),
        ("media-title", FORMAT_STRING) => Some(Event::Title(text())),
        ("metadata/by-key/artist", FORMAT_STRING) => Some(Event::Artist(text())),
        ("metadata/by-key/album", FORMAT_STRING) => Some(Event::Album(text())),
        ("video-params/dw", FORMAT_INT64) => {
            *width = int();
            Some(Event::VideoSize(*width, *height))
        }
        ("video-params/dh", FORMAT_INT64) => {
            *height = int();
            Some(Event::VideoSize(*width, *height))
        }
        _ => None,
    }
}

unsafe extern "C" fn on_render_update(context: *mut c_void) {
    // SAFETY: `context` is the RenderShared the render thread keeps alive
    // until the render context is freed.
    let render = unsafe { &*(context as *const RenderShared) };
    mark_dirty(render);
}

fn render_loop(
    api: &Api,
    handle: &Handle,
    render: &Arc<RenderShared>,
    frames: &async_channel::Sender<()>,
) {
    let mut context: *mut c_void = std::ptr::null_mut();
    let api_type = cstring("sw");
    let mut params = [
        RenderParam {
            kind: RENDER_PARAM_API_TYPE,
            data: api_type.as_ptr() as *mut c_void,
        },
        RenderParam {
            kind: RENDER_PARAM_INVALID,
            data: std::ptr::null_mut(),
        },
    ];
    // SAFETY: valid handle; params are terminated by INVALID.
    if unsafe { (api.render_context_create)(&mut context, handle.0, params.as_mut_ptr()) } < 0 {
        return;
    }
    // SAFETY: `render` outlives the context (freed below before returning).
    unsafe {
        (api.render_context_set_update_callback)(
            context,
            Some(on_render_update),
            Arc::as_ptr(render) as *mut c_void,
        )
    };
    let format = cstring("bgr0");
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        {
            let mut dirty = render
                .dirty
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while !*dirty {
                dirty = render
                    .wake
                    .wait(dirty)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            *dirty = false;
        }
        if render.stop.load(Ordering::Acquire) {
            break;
        }
        // SAFETY: render thread owns the context.
        let flags = unsafe { (api.render_context_update)(context) };
        let packed = render.target.load(Ordering::Acquire);
        if flags & RENDER_UPDATE_FRAME == 0 || packed == 0 {
            continue;
        }
        let (width, height) = ((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32);
        let stride = width as usize * 4;
        buffer.resize(stride * height as usize, 0);
        let mut size = [width as c_int, height as c_int];
        let mut stride_value = stride;
        let mut params = [
            RenderParam {
                kind: RENDER_PARAM_SW_SIZE,
                data: size.as_mut_ptr() as *mut c_void,
            },
            RenderParam {
                kind: RENDER_PARAM_SW_FORMAT,
                data: format.as_ptr() as *mut c_void,
            },
            RenderParam {
                kind: RENDER_PARAM_SW_STRIDE,
                data: &mut stride_value as *mut usize as *mut c_void,
            },
            RenderParam {
                kind: RENDER_PARAM_SW_POINTER,
                data: buffer.as_mut_ptr() as *mut c_void,
            },
            RenderParam {
                kind: RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        // SAFETY: the buffer holds stride × height bytes.
        if unsafe { (api.render_context_render)(context, params.as_mut_ptr()) } < 0 {
            continue;
        }
        // "bgr0" leaves the fourth byte undefined; GPUI needs it opaque.
        for pixel in buffer.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }
        let frame = Frame {
            width,
            height,
            pixels: buffer.clone(),
        };
        *render
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(frame);
        // The window collects the newest frame; a full channel means it
        // has not yet, which is fine.
        let _ = frames.try_send(());
    }
    // SAFETY: stops callbacks, then frees the context on its own thread.
    unsafe {
        (api.render_context_set_update_callback)(context, None, std::ptr::null_mut());
        (api.render_context_free)(context);
    }
}
