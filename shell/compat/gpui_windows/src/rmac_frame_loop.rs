//! rmac: idle frame scheduling (docs/decisions/0025-vendor-gpui-windows.md).
//!
//! Upstream runs one `VSyncProvider` thread that waits for every DWM vblank
//! and invalidates every window, so each window gets a `WM_PAINT` and runs a
//! GPUI frame about 60 times a second even when nothing changed: two threads
//! wake per vblank for the whole life of the process.
//!
//! Here a window asks for vblanks only while its frames draw. After two
//! frames that draw nothing it *parks*: it leaves the vsync demand, and once
//! no window wants vblanks the vsync thread blocks on a condition variable
//! instead of waiting on DWM. A parked window is re-checked after each
//! message the main thread dispatches and after each batch of main-thread
//! tasks. GPUI only marks a window dirty on the main thread, inside a task,
//! a timer or a platform callback, and each of those arrives as a message,
//! so no change is missed while an idle process never wakes. This is the
//! same parked loop as `gpui_linux` (ADR 0013, "Idle frame loop").

use std::{
    cell::Cell,
    sync::LazyLock,
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};
use smallvec::SmallVec;
use windows::Win32::Foundation::HWND;

/// Frames that draw nothing in a row before a window parks.
const IDLE_FRAMES_BEFORE_PARKING: u32 = 2;

/// GPUI runs an inactive window's frame only if 33.333 ms passed since the
/// last frame it ran, whenever next-frame callbacks are queued; a sooner
/// frame returns without running them or drawing. A finished `img()` load
/// repaints through such a callback, so a window that parked right after
/// such a frame would keep it until the next input.
const INACTIVE_FRAME_INTERVAL: Duration = Duration::from_micros(33_334);

/// The one-shot retry for a frame GPUI may have skipped: longer than the
/// interval, so GPUI always runs it.
pub(crate) const THROTTLE_RETRY_DELAY_MS: u32 = 35;
const _: () = assert!(THROTTLE_RETRY_DELAY_MS as u128 * 1000 > INACTIVE_FRAME_INTERVAL.as_micros());

/// `WM_TIMER` id of the throttle retry; upstream's size-move timer is 1.
pub(crate) const THROTTLE_RETRY_TIMER_ID: usize = 0x524D_4143;

/// The windows that want the next vblank, shared with the vsync thread.
#[derive(Default)]
pub(crate) struct VsyncDemand {
    state: Mutex<DemandState>,
    changed: Condvar,
}

#[derive(Default)]
struct DemandState {
    windows: SmallVec<[isize; 4]>,
    device_check: bool,
}

impl VsyncDemand {
    fn request(&self, hwnd: isize) {
        let mut state = self.state.lock();
        if !state.windows.contains(&hwnd) {
            state.windows.push(hwnd);
            self.changed.notify_one();
        }
    }

    fn release(&self, hwnd: isize) {
        self.state.lock().windows.retain(|window| *window != hwnd);
    }

    /// Wakes the vsync thread once so it checks the DirectX device, which
    /// it otherwise only does while frames are being drawn.
    pub(crate) fn request_device_check(&self) {
        self.state.lock().device_check = true;
        self.changed.notify_one();
    }

    /// Blocks until a window wants a vblank or a device check was asked
    /// for. The vsync thread checks the device after every wait.
    pub(crate) fn wait(&self) {
        let mut state = self.state.lock();
        while state.windows.is_empty() && !state.device_check {
            self.changed.wait(&mut state);
        }
        state.device_check = false;
    }

    /// Whether `hwnd` wants the vblank that just happened.
    pub(crate) fn wants(&self, hwnd: HWND) -> bool {
        self.state.lock().windows.contains(&(hwnd.0 as isize))
    }
}

/// The process's vsync demand. One `WindowsPlatform` runs per process.
pub(crate) fn vsync_demand() -> &'static VsyncDemand {
    static DEMAND: LazyLock<VsyncDemand> = LazyLock::new(VsyncDemand::default);
    &DEMAND
}

/// Whether a window's frame loop is parked after `idle_streak` frames in a
/// row that drew nothing.
fn parked(idle_streak: u32) -> bool {
    idle_streak >= IDLE_FRAMES_BEFORE_PARKING
}

/// The idle streak after a frame: reset when it drew or must keep drawing.
fn next_idle_streak(idle_streak: u32, keeps_drawing: bool) -> u32 {
    if keeps_drawing {
        0
    } else {
        idle_streak.saturating_add(1)
    }
}

/// Whether GPUI may have skipped this frame for an inactive window and kept
/// its next-frame callbacks queued.
fn may_have_throttled(drew: bool, active: bool, since_previous: Option<Duration>) -> bool {
    !drew && !active && since_previous.is_some_and(|since| since < INACTIVE_FRAME_INTERVAL)
}

/// What the window must do after a frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AfterFrame {
    Nothing,
    /// Arm the one-shot `THROTTLE_RETRY_TIMER_ID` timer.
    ArmThrottleRetry,
}

/// One window's frame loop state. Main thread only.
pub(crate) struct FrameLoop {
    hwnd: isize,
    idle_streak: Cell<u32>,
    suspended: Cell<bool>,
    previous_frame: Cell<Option<Instant>>,
    throttle_retry_armed: Cell<bool>,
    /// The retry fired and nothing has drawn since: another idle frame
    /// soon after must not arm it again. Several inactive windows in one
    /// process (the Lulo layer's bar, Dock and panels) otherwise kept each
    /// other awake: each retry's message re-checked the other parked
    /// windows, whose idle checks then counted as "soon after" a frame and
    /// armed their own retries, about 40 timers a second for ever.
    throttle_retry_spent: Cell<bool>,
}

impl FrameLoop {
    /// A new window asks for vblanks until its first frames have drawn.
    pub(crate) fn new(hwnd: HWND) -> Self {
        let hwnd = hwnd.0 as isize;
        vsync_demand().request(hwnd);
        Self {
            hwnd,
            idle_streak: Cell::new(0),
            suspended: Cell::new(false),
            previous_frame: Cell::new(None),
            throttle_retry_armed: Cell::new(false),
            throttle_retry_spent: Cell::new(false),
        }
    }

    /// Call as a frame starts; returns the time since the previous one.
    pub(crate) fn begin_frame(&self) -> Option<Duration> {
        let now = Instant::now();
        self.previous_frame
            .replace(Some(now))
            .map(|previous| now.duration_since(previous))
    }

    /// Call after a frame. `keeps_drawing` is true when the frame drew or
    /// presented, or when something still needs frames without being dirty
    /// (a forced render pending, a touchpad gesture running).
    pub(crate) fn end_frame(
        &self,
        keeps_drawing: bool,
        drew: bool,
        active: bool,
        since_previous: Option<Duration>,
    ) -> AfterFrame {
        if drew {
            self.throttle_retry_spent.set(false);
        }
        let streak = next_idle_streak(self.idle_streak.get(), keeps_drawing);
        self.idle_streak.set(streak);
        if self.suspended.get() {
            return AfterFrame::Nothing;
        }
        if parked(streak) {
            vsync_demand().release(self.hwnd);
        } else {
            vsync_demand().request(self.hwnd);
        }
        if may_have_throttled(drew, active, since_previous)
            && !self.throttle_retry_armed.get()
            && !self.throttle_retry_spent.get()
        {
            self.throttle_retry_armed.set(true);
            return AfterFrame::ArmThrottleRetry;
        }
        AfterFrame::Nothing
    }

    /// The throttle retry timer fired.
    pub(crate) fn throttle_retry_fired(&self) {
        self.throttle_retry_armed.set(false);
        self.throttle_retry_spent.set(true);
    }

    /// Whether a frame should run now because the loop is parked and the
    /// main thread just did something that may have dirtied the window.
    pub(crate) fn is_parked(&self) -> bool {
        !self.suspended.get() && parked(self.idle_streak.get())
    }

    /// Something outside GPUI needs frames again (a touchpad contact).
    pub(crate) fn wake(&self) {
        if self.suspended.get() {
            return;
        }
        self.idle_streak.set(0);
        self.throttle_retry_spent.set(false);
        vsync_demand().request(self.hwnd);
    }

    /// Minimised or destroyed: no frames at all until `resume`.
    pub(crate) fn suspend(&self) {
        self.suspended.set(true);
        vsync_demand().release(self.hwnd);
    }

    /// Restored from minimised: draw again from the next vblank.
    pub(crate) fn resume(&self) {
        self.suspended.set(false);
        self.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rmac_a_drawing_window_keeps_its_vblanks() {
        assert_eq!(next_idle_streak(5, true), 0);
        assert!(!parked(next_idle_streak(5, true)));
    }

    #[test]
    fn rmac_two_idle_frames_park_the_loop() {
        let one = next_idle_streak(0, false);
        assert!(!parked(one));
        let two = next_idle_streak(one, false);
        assert!(parked(two));
        assert!(parked(next_idle_streak(u32::MAX, false)));
    }

    #[test]
    fn rmac_only_an_inactive_idle_frame_soon_after_another_retries() {
        let soon = Some(Duration::from_millis(16));
        let late = Some(Duration::from_millis(40));
        assert!(may_have_throttled(false, false, soon));
        assert!(!may_have_throttled(true, false, soon));
        assert!(!may_have_throttled(false, true, soon));
        assert!(!may_have_throttled(false, false, late));
        assert!(!may_have_throttled(false, false, None));
    }

    #[test]
    fn rmac_the_demand_tracks_parked_and_suspended_windows() {
        let window = HWND(0x1234 as _);
        let frame_loop = FrameLoop::new(window);
        assert!(vsync_demand().wants(window));
        frame_loop.end_frame(false, false, true, None);
        assert!(vsync_demand().wants(window));
        frame_loop.end_frame(false, false, true, None);
        assert!(!vsync_demand().wants(window));
        assert!(frame_loop.is_parked());
        frame_loop.end_frame(true, true, true, None);
        assert!(vsync_demand().wants(window));
        frame_loop.suspend();
        assert!(!vsync_demand().wants(window));
        assert!(!frame_loop.is_parked());
        frame_loop.end_frame(true, true, true, None);
        assert!(!vsync_demand().wants(window));
        frame_loop.resume();
        assert!(vsync_demand().wants(window));
    }

    #[test]
    fn rmac_the_throttle_retry_arms_once_until_it_fires() {
        let frame_loop = FrameLoop::new(HWND(0x5678 as _));
        let soon = Some(Duration::from_millis(16));
        assert_eq!(
            frame_loop.end_frame(false, false, false, soon),
            AfterFrame::ArmThrottleRetry
        );
        assert_eq!(
            frame_loop.end_frame(false, false, false, soon),
            AfterFrame::Nothing
        );
        frame_loop.throttle_retry_fired();
        // The retry's own frame drew nothing: no second retry until
        // something draws.
        assert_eq!(
            frame_loop.end_frame(false, false, false, soon),
            AfterFrame::Nothing
        );
        frame_loop.end_frame(true, true, false, soon);
        assert_eq!(
            frame_loop.end_frame(false, false, false, soon),
            AfterFrame::ArmThrottleRetry
        );
        frame_loop.suspend();
    }
}
