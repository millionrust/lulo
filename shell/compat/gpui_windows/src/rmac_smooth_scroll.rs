//! rmac: smooth wheel scrolling (docs/decisions/0025-vendor-gpui-windows.md,
//! "Smooth scrolling").
//!
//! Upstream hands every `WM_MOUSEWHEEL` straight to GPUI as
//! `ScrollDelta::Lines`, the moment it arrives. A mouse notch then jumps
//! three lines in one frame, and a precision touchpad that Direct
//! Manipulation does not own (its legacy wheel messages: 120 units split
//! into many small deltas at the touchpad's own report rate, about 125 Hz)
//! lands two messages in one 60 Hz frame and three in another: the scroll
//! steps unevenly ("zig zag") and stops dead when the fingers lift.
//!
//! Here wheel input is queued in pixels and handed out once per frame,
//! timed by the vblank that started the frame:
//!
//! - a mouse notch eases out over about 120 ms (`NOTCH_TAU_MS`), as on the
//!   Mac and in Edge; notches that arrive during the glide add to it;
//! - high-resolution deltas (not whole notches, or within a stream of such
//!   deltas) are handed out at the stream's velocity over the last 100 ms
//!   (`STREAM_VELOCITY_MS`), with a slow correction that keeps about one
//!   frame of backlog (`STREAM_BACKLOG_MS`), so the per-frame step follows
//!   the finger instead of the message count; when the stream stops the
//!   scroll glides on with the same momentum as `gpui_linux` (ADR 0013):
//!   decay 325 ms, from the last 60 ms of motion.
//!
//! Direct Manipulation's own pixel pan and inertia are not touched. While a
//! glide runs the window keeps its vblanks; when it ends the frame loop
//! parks as before, so an idle window still wakes for nothing.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    sync::{
        LazyLock, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gpui::{Modifiers, Pixels, Point, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px};

/// Logical pixels per wheel line: three lines (Windows' default) make a
/// 72 px notch, close to upstream's three `window.line_height()` lines.
pub(crate) const PIXELS_PER_LINE: f32 = 24.0;
/// One `WHEEL_DELTA`.
const NOTCH: f32 = 120.0;
/// A mouse notch's ease-out time constant: 95 % of it within ~110 ms.
const NOTCH_TAU_MS: f32 = 37.0;
/// A whole-notch delta this soon after a high-resolution one is still the
/// touchpad's stream.
const STREAM_WINDOW_MS: f64 = 100.0;
/// No touchpad delta for this long: the fingers lifted.
const STREAM_END_MS: f64 = 45.0;
/// The touchpad stream's velocity is measured over this long.
const STREAM_VELOCITY_MS: f64 = 100.0;
/// The backlog a running stream keeps, so steps stay even.
const STREAM_BACKLOG_MS: f32 = 16.7;
/// How much of the backlog's error one frame corrects.
const STREAM_CORRECTION: f32 = 0.15;
/// What is left of a stream's backlog drains with this time constant once
/// the fingers lift.
const DRAIN_TAU_MS: f32 = 33.0;
/// The motion the fling velocity is measured over.
const FLING_WINDOW_MS: f64 = 60.0;
/// The same momentum as `gpui_linux`'s Wayland client.
const MOMENTUM_DECAY_MS: f32 = 325.0;
/// Logical px per ms below which a lifted scroll does not glide.
const MOMENTUM_MIN_VELOCITY: f32 = 0.08;
/// A momentum step this small ends the glide.
const MOMENTUM_STOP_PX: f32 = 0.25;
/// What is left of a notch below this is applied at once.
const SETTLE_PX: f32 = 0.5;
/// The step a frame may cover; longer gaps (a stalled frame, the first
/// frame after parking) count as at most this.
const MAX_FRAME_MS: f32 = 50.0;
const DEFAULT_FRAME_MS: f32 = 16.667;

static BASE: LazyLock<Instant> = LazyLock::new(Instant::now);
/// The last vblank the vsync thread saw, in µs since `BASE` (0: none yet).
static LAST_VBLANK_US: AtomicU64 = AtomicU64::new(0);
static VSYNC_INTERVAL_US: AtomicU64 = AtomicU64::new(16_667);

fn now_ms() -> f64 {
    BASE.elapsed().as_secs_f64() * 1000.0
}

/// The vsync thread calls this after each vblank.
pub(crate) fn note_vblank() {
    let us = BASE.elapsed().as_micros() as u64;
    LAST_VBLANK_US.store(us.max(1), Ordering::Relaxed);
}

/// The vsync thread's refresh interval, once.
pub(crate) fn note_vsync_interval(interval: Duration) {
    VSYNC_INTERVAL_US.store(interval.as_micros().max(1) as u64, Ordering::Relaxed);
    trace(format_args!(
        "vsync interval {:.3} ms",
        interval.as_secs_f64() * 1000.0
    ));
}

/// The time a frame stands for: the vblank that woke it when it runs
/// within one interval of it, so steps are even however late the
/// `WM_PAINT` was dispatched; otherwise now.
fn frame_time_ms() -> f64 {
    let now = now_ms();
    let vblank = LAST_VBLANK_US.load(Ordering::Relaxed);
    if vblank == 0 {
        return now;
    }
    let interval = VSYNC_INTERVAL_US.load(Ordering::Relaxed) as f64 / 1000.0;
    let vblank = vblank as f64 / 1000.0;
    if now >= vblank && now - vblank < interval {
        vblank
    } else {
        now
    }
}

/// `RMAC_GPUI_SMOOTH_SCROLL=0` restores upstream's immediate line deltas,
/// for before/after measurements (`scripts/windows/launch_smoke.py`).
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| !std::env::var("RMAC_GPUI_SMOOTH_SCROLL").is_ok_and(|v| v == "0"))
}

/// `RMAC_GPUI_SCROLL_TRACE=1` prints a `gpui_windows scroll:` line for each
/// wheel message and each frame, read by `launch_smoke.py`'s scroll check.
pub(crate) fn trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("RMAC_GPUI_SCROLL_TRACE").is_ok_and(|v| v == "1"))
}

pub(crate) fn trace(line: std::fmt::Arguments<'_>) {
    if trace_enabled() {
        use std::io::Write;
        let text = format!(
            "gpui_windows scroll: {line} at {:.2} ms\n",
            crate::rmac_trace::since_process_start_ms()
        );
        let _ = std::io::stderr().lock().write_all(text.as_bytes());
    }
}

/// One window's queued wheel scrolling. Main thread only.
#[derive(Default)]
pub(crate) struct SmoothScroll {
    /// Logical px still to hand out.
    pending: Cell<Point<f32>>,
    tau_ms: Cell<f32>,
    /// Momentum after a touchpad stream, logical px per ms.
    velocity: Cell<Point<f32>>,
    momentum: Cell<bool>,
    /// The touchpad stream's recent deltas: (time, px).
    samples: RefCell<VecDeque<(f64, Point<f32>)>>,
    last_touchpad_input: Cell<Option<f64>>,
    stream_start: Cell<Option<f64>>,
    last_frame: Cell<Option<f64>>,
    position: Cell<Point<Pixels>>,
    modifiers: Cell<Modifiers>,
}

fn is_zero(p: Point<f32>) -> bool {
    p.x == 0.0 && p.y == 0.0
}

fn length(p: Point<f32>) -> f32 {
    p.x.hypot(p.y)
}

fn add(a: Point<f32>, b: Point<f32>) -> Point<f32> {
    point(a.x + b.x, a.y + b.y)
}

fn sub(a: Point<f32>, b: Point<f32>) -> Point<f32> {
    point(a.x - b.x, a.y - b.y)
}

fn scale(a: Point<f32>, by: f32) -> Point<f32> {
    point(a.x * by, a.y * by)
}

fn opposes(a: f32, b: f32) -> bool {
    a * b < 0.0
}

/// One axis of a running stream's step: the stream's velocity, plus a
/// slow correction toward `STREAM_BACKLOG_MS` of backlog, never past the
/// backlog itself.
fn stream_axis(backlog: f32, velocity: f32, dt: f32) -> f32 {
    let out = velocity * dt + STREAM_CORRECTION * (backlog - velocity * (dt + STREAM_BACKLOG_MS));
    if backlog >= 0.0 {
        out.clamp(0.0, backlog)
    } else {
        out.clamp(backlog, 0.0)
    }
}

impl SmoothScroll {
    /// One wheel message: `raw` is its signed wheel delta (120 a notch) and
    /// `lines` the distance a notch scrolls, along x when `horizontal`, in
    /// GPUI's sign (a positive `raw` is a positive line delta).
    pub(crate) fn wheel(
        &self,
        raw: f32,
        horizontal: bool,
        lines: f32,
        position: Point<Pixels>,
        modifiers: Modifiers,
    ) {
        self.wheel_at(now_ms(), raw, horizontal, lines, position, modifiers);
    }

    fn wheel_at(
        &self,
        now: f64,
        raw: f32,
        horizontal: bool,
        lines: f32,
        position: Point<Pixels>,
        modifiers: Modifiers,
    ) {
        let distance = raw / NOTCH * lines * PIXELS_PER_LINE;
        let delta = if horizontal {
            point(distance, 0.0)
        } else {
            point(0.0, distance)
        };
        let high_resolution = raw % NOTCH != 0.0;
        let in_stream = self
            .last_touchpad_input
            .get()
            .is_some_and(|last| now - last < STREAM_WINDOW_MS);
        let touchpad = high_resolution || in_stream;
        trace(format_args!(
            "input raw={raw} touchpad={} px={distance:.2}",
            touchpad as u8
        ));
        // New input takes over from a glide.
        self.momentum.set(false);
        self.velocity.set(point(0.0, 0.0));
        let mut pending = self.pending.get();
        if touchpad {
            self.tau_ms.set(DRAIN_TAU_MS);
            if self.last_touchpad_input.get().is_none() {
                self.stream_start.set(Some(now));
            }
            self.last_touchpad_input.set(Some(now));
            let mut samples = self.samples.borrow_mut();
            samples.push_back((now, delta));
            while samples
                .front()
                .is_some_and(|(time, _)| now - time > STREAM_VELOCITY_MS)
            {
                samples.pop_front();
            }
        } else {
            self.tau_ms.set(NOTCH_TAU_MS);
            self.last_touchpad_input.set(None);
            self.stream_start.set(None);
            self.samples.borrow_mut().clear();
            // A notch the other way cancels the rest of the glide, as on
            // the Mac.
            if opposes(pending.x, delta.x) {
                pending.x = 0.0;
            }
            if opposes(pending.y, delta.y) {
                pending.y = 0.0;
            }
        }
        self.pending.set(add(pending, delta));
        self.position.set(position);
        self.modifiers.set(modifiers);
    }

    /// Whether frames must keep coming: a glide or a touchpad stream runs.
    pub(crate) fn is_active(&self) -> bool {
        !is_zero(self.pending.get())
            || self.momentum.get()
            || self.last_touchpad_input.get().is_some()
    }

    /// Stop at once: a click, or Direct Manipulation took a contact.
    pub(crate) fn stop(&self) {
        self.pending.set(point(0.0, 0.0));
        self.momentum.set(false);
        self.velocity.set(point(0.0, 0.0));
        self.samples.borrow_mut().clear();
        self.last_touchpad_input.set(None);
        self.stream_start.set(None);
        self.last_frame.set(None);
    }

    /// The scroll this frame hands out, if any.
    pub(crate) fn frame(&self) -> Option<ScrollWheelEvent> {
        // A frame that ran late used the time it ran; the vblank after it
        // can be older than that, so such a frame uses its own time too
        // rather than handing out nothing.
        let mut now = frame_time_ms();
        if self.last_frame.get().is_some_and(|last| now <= last) {
            now = now_ms();
        }
        let step = self.step_at(now);
        (!is_zero(step)).then(|| ScrollWheelEvent {
            position: self.position.get(),
            delta: ScrollDelta::Pixels(point(px(step.x), px(step.y))),
            modifiers: self.modifiers.get(),
            touch_phase: TouchPhase::Moved,
        })
    }

    fn step_at(&self, now: f64) -> Point<f32> {
        if !self.is_active() {
            self.last_frame.set(None);
            return point(0.0, 0.0);
        }
        let dt = self
            .last_frame
            .get()
            .map_or(DEFAULT_FRAME_MS, |last| (now - last) as f32);
        if dt <= 0.0 {
            // A second frame for the same vblank hands out nothing more.
            return point(0.0, 0.0);
        }
        self.last_frame.set(Some(now));
        let dt = dt.min(MAX_FRAME_MS);
        let mut step = point(0.0, 0.0);

        let pending = self.pending.get();
        let streaming = self
            .last_touchpad_input
            .get()
            .is_some_and(|last| now - last <= STREAM_END_MS);
        if streaming {
            // Hand out the stream's velocity, so a frame that got one
            // message more than the last does not step further.
            let start = self.stream_start.get().unwrap_or(now);
            let window = ((now - start) + 8.0).clamp(8.0, STREAM_VELOCITY_MS);
            let recent = self
                .samples
                .borrow()
                .iter()
                .filter(|(time, _)| now - time < window)
                .fold(point(0.0, 0.0), |sum, (_, delta)| add(sum, *delta));
            let velocity = scale(recent, 1.0 / window as f32);
            step = point(
                stream_axis(pending.x, velocity.x, dt),
                stream_axis(pending.y, velocity.y, dt),
            );
            self.pending.set(sub(pending, step));
        } else if !is_zero(pending) {
            let tau = self.tau_ms.get().max(1.0);
            let mut out = scale(pending, 1.0 - (-dt / tau).exp());
            if length(sub(pending, out)) < SETTLE_PX {
                out = pending;
            }
            self.pending.set(sub(pending, out));
            step = out;
        }

        if let Some(last) = self.last_touchpad_input.get()
            && !streaming
        {
            // The fingers lifted: glide on from the last motion.
            self.last_touchpad_input.set(None);
            self.stream_start.set(None);
            let mut samples = self.samples.borrow_mut();
            let fling: Vec<_> = samples
                .drain(..)
                .filter(|(time, _)| last - time < FLING_WINDOW_MS)
                .collect();
            if let Some((first, _)) = fling.first() {
                let total = fling
                    .iter()
                    .fold(point(0.0, 0.0), |sum, (_, delta)| add(sum, *delta));
                // The span the deltas cover, counting the first one's own
                // report interval.
                let span = ((last - first) as f32 + 8.0).max(16.0);
                let velocity = scale(total, 1.0 / span);
                if length(velocity) >= MOMENTUM_MIN_VELOCITY {
                    self.velocity.set(velocity);
                    self.momentum.set(true);
                    trace(format_args!(
                        "momentum vx={:.3} vy={:.3}",
                        velocity.x, velocity.y
                    ));
                }
            }
        }

        if self.momentum.get() {
            let velocity = self.velocity.get();
            let decay = (-dt / MOMENTUM_DECAY_MS).exp();
            let out = scale(velocity, MOMENTUM_DECAY_MS * (1.0 - decay));
            self.velocity.set(scale(velocity, decay));
            if length(out) < MOMENTUM_STOP_PX {
                self.momentum.set(false);
                self.velocity.set(point(0.0, 0.0));
            } else {
                step = add(step, out);
            }
        }

        if !self.is_active() {
            self.last_frame.set(None);
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(scroll: &SmoothScroll, now: f64, raw: f32) {
        scroll.wheel_at(now, raw, false, 3.0, Point::default(), Modifiers::default());
    }

    /// Frames at 60 Hz from `start` until the scroll settles; each step.
    fn run(scroll: &SmoothScroll, start: f64, limit: usize) -> Vec<f32> {
        let mut steps = Vec::new();
        let mut now = start;
        while scroll.is_active() && steps.len() < limit {
            steps.push(scroll.step_at(now).y);
            now += 16.667;
        }
        steps
    }

    #[test]
    fn rmac_a_notch_eases_out_over_about_120_ms_and_covers_its_distance() {
        let scroll = SmoothScroll::default();
        wheel(&scroll, 0.0, 120.0);
        let steps = run(&scroll, 16.667, 100);
        let total: f32 = steps.iter().sum();
        assert!((total - 72.0).abs() < 0.01, "{total}");
        // Not a jump: the first frame covers well under half the notch.
        assert!(steps[0] < 36.0 && steps[0] > 10.0, "{steps:?}");
        // Each step smaller than the one before (ease-out), but the last,
        // which settles the remainder.
        let easing = &steps[..steps.len() - 1];
        assert!(
            easing.windows(2).all(|pair| pair[1] <= pair[0]),
            "{steps:?}"
        );
        // 95 % within 8 frames (133 ms), settled within 12.
        let first8: f32 = steps.iter().take(8).sum();
        assert!(first8 > 0.95 * 72.0, "{steps:?}");
        assert!(steps.len() <= 12, "{steps:?}");
        assert!(!scroll.is_active());
    }

    #[test]
    fn rmac_a_notch_the_other_way_cancels_the_glide() {
        let scroll = SmoothScroll::default();
        wheel(&scroll, 0.0, 120.0);
        scroll.step_at(16.667);
        wheel(&scroll, 20.0, -120.0);
        let total: f32 = run(&scroll, 33.3, 100).iter().sum();
        assert!((total + 72.0).abs() < 0.01, "{total}");
    }

    #[test]
    fn rmac_an_uneven_touchpad_stream_scrolls_evenly_and_glides_on() {
        let scroll = SmoothScroll::default();
        // 15 units every 8 ms (a 125 Hz touchpad) for 400 ms: frames get
        // two messages each, and three every twelfth frame (+50 %).
        let mut steps = Vec::new();
        let mut next_input = 0.0;
        let mut frame = 16.667;
        let mut sent = 0.0;
        while frame < 400.0 {
            while next_input <= frame {
                wheel(&scroll, next_input, 15.0);
                sent += 9.0;
                next_input += 8.0;
            }
            steps.push(scroll.step_at(frame).y);
            frame += 16.667;
        }
        let steady = &steps[6..];
        let mean = steady.iter().sum::<f32>() / steady.len() as f32;
        let worst = steady
            .iter()
            .map(|step| (step - mean).abs() / mean)
            .fold(0.0, f32::max);
        assert!(worst < 0.15, "mean {mean}, worst {worst}, {steps:?}");
        // The fingers lifted: it glides on for many frames, then stops,
        // never backwards, and covers at least what was sent.
        let glide = run(&scroll, frame, 400);
        let moving = glide.iter().filter(|step| **step > 0.0).count();
        assert!(moving > 20, "{glide:?}");
        assert!(!scroll.is_active());
        assert!(glide.iter().all(|step| *step >= 0.0));
        let total: f32 = steps.iter().chain(glide.iter()).sum();
        assert!(total > sent, "{total} {sent}");
    }

    #[test]
    fn rmac_a_slow_touchpad_stop_does_not_glide() {
        let scroll = SmoothScroll::default();
        wheel(&scroll, 0.0, 1.0);
        wheel(&scroll, 30.0, 1.0);
        let glide = run(&scroll, 46.0, 400);
        // Only the two deltas themselves (0.6 px each), no momentum.
        let total: f32 = glide.iter().sum();
        assert!((total - 1.2).abs() < 0.01, "{total} {glide:?}");
    }

    #[test]
    fn rmac_a_second_frame_for_one_vblank_steps_nothing() {
        let scroll = SmoothScroll::default();
        wheel(&scroll, 0.0, 120.0);
        assert!(scroll.step_at(16.0).y > 0.0);
        assert_eq!(scroll.step_at(16.0), point(0.0, 0.0));
        assert!(scroll.step_at(32.0).y > 0.0);
    }

    #[test]
    fn rmac_stop_ends_everything() {
        let scroll = SmoothScroll::default();
        wheel(&scroll, 0.0, 120.0);
        scroll.stop();
        assert!(!scroll.is_active());
        assert_eq!(scroll.step_at(16.0), point(0.0, 0.0));
    }
}
