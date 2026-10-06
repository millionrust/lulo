// rmac: opt-in per-frame timing trace for the Beta performance audit
// (docs/beta-checklist.md: "Input to visible response p95 <= 50 ms" and the
// 60/120 Hz animation frame budget rows). Set RMAC_FRAME_TRACE=<path> before
// launching a GPUI window on the Wayland backend to record, for every
// frame:
//   - `frame_callback`: the compositor's wl_callback::Done firing, i.e. the
//     start of `WaylandWindow::frame`;
//   - `draw_start`: just before GPUI's scene is handed to the renderer;
//   - `present` / `draw_skip`: right after the renderer returns, depending
//     on whether it actually presented a swapchain image;
// and, for every input event GPUI's window receives:
//   - `input`: the start of `WaylandWindow::handle_input`;
// and, for every keyboard focus change the compositor reports:
//   - `focus_in` / `focus_out`: `WaylandWindowStatePtr::set_focused`, whether
//     or not it changes what the window draws (a window drawn active from
//     its first frame records `focus_in` and draws nothing for it);
// and, for every toplevel configure and size or scale change:
//   - `configure` / `resize`;
// and when a drawn frame leaves the next one to re-render the whole scene
// (a dropped or suboptimal swapchain image):
//   - `force_render`.
// and, once each, the startup phases before the first frame (for
// scripts/behavior/run_speed_sweep.py's cold-launch breakdown):
//   - `common_start` / `common_ready`: around `LinuxCommon::new`, which
//     builds the text system (the system font database scan);
//   - `open_window`: the app asked for its first (or a later) window;
//   - `renderer_ready`: that window's wgpu renderer exists (the GPU device
//     is created lazily with the first window);
//   - `first_configure`: the compositor's first configure was acknowledged.
//
// scripts/behavior/run_frame_timing.py correlates `input` timestamps with
// the next `present` to compute input-to-present latency, and uses
// `draw_start`/`present` pairs to compute the share of frames drawn within
// the 16.7 ms (60 Hz) / 8.3 ms (120 Hz) budget.
//
// With the variable unset (the default), every call site below is a single
// relaxed atomic load that returns immediately: no file is opened, nothing
// is formatted or written.

use std::cell::RefCell;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();

thread_local! {
    // GPUI's Wayland backend runs its event loop and all window callbacks on
    // a single thread, so a thread-local writer needs no locking.
    static WRITER: RefCell<Option<BufWriter<File>>> = const { RefCell::new(None) };
}

/// Opens the trace file named by `RMAC_FRAME_TRACE`, if set. Call once, on
/// the backend's own thread, before the first window is created. A missing
/// or empty variable leaves tracing disabled; a file that cannot be created
/// logs a warning and leaves tracing disabled rather than failing startup.
pub(crate) fn init() {
    let path = match std::env::var("RMAC_FRAME_TRACE") {
        Ok(path) if !path.is_empty() => path,
        _ => return,
    };
    match File::create(&path) {
        Ok(file) => {
            let mut writer = BufWriter::new(file);
            let _ = writeln!(writer, "event,micros");
            WRITER.with(|cell| *cell.borrow_mut() = Some(writer));
            let _ = START.set(Instant::now());
            ENABLED.store(true, Ordering::Relaxed);
        }
        Err(err) => {
            log::warn!("RMAC_FRAME_TRACE: failed to create {path}: {err}");
        }
    }
}

/// Records `event` at the current time, relative to `init`. A no-op unless
/// `RMAC_FRAME_TRACE` was set.
#[inline]
pub(crate) fn record(event: &str) {
    if ENABLED.load(Ordering::Relaxed) {
        record_enabled(event);
    }
}

#[cold]
fn record_enabled(event: &str) {
    let Some(start) = START.get() else { return };
    let micros = start.elapsed().as_micros();
    WRITER.with(|cell| {
        if let Some(writer) = cell.borrow_mut().as_mut() {
            let _ = writeln!(writer, "{event},{micros}");
            // Flushed per line: traced sessions are short (a few minutes at
            // most) and this makes the file usable even if the nested
            // session is killed mid-run instead of exiting cleanly.
            let _ = writer.flush();
        }
    });
}
