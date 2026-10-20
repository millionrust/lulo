//! rmac: a new window appears with its first frame, not before it
//! (WIN-OS-99).
//!
//! Upstream shows a window as soon as it is created. GPUI then builds the
//! window's views and draws, and until that first `Present` Direct
//! Composition has nothing to show: DWM drew the window's shadow and frame
//! around an empty, see-through rectangle for 30 to 130 ms on the reference
//! laptop. The Mac never shows a window before its content. A window that
//! opens shown is now cloaked (`DWMWA_CLOAK`: placed, active and composed,
//! just not on screen) and uncloaked by the frame that first draws it. A
//! one-shot timer uncloaks it anyway if no frame draws within
//! [`UNCLOAK_AFTER_MS`], so a window can never stay invisible.

use std::cell::Cell;

use gpui_util::ResultExt;
use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute},
    UI::WindowsAndMessaging::{KillTimer, SetTimer},
};
use windows::core::BOOL;

/// The fallback timer's id ("RMFF"), distinct from the other window timers.
pub(crate) const UNCLOAK_TIMER_ID: usize = 0x524D_4646;
/// Longer than any first frame measured (about 130 ms on the reference
/// laptop, 400 ms on CI's software renderer), short enough that a window
/// that cannot draw still shows promptly.
const UNCLOAK_AFTER_MS: u32 = 750;

#[derive(Default)]
pub(crate) struct FirstFrame {
    cloaked: Cell<bool>,
}

fn set_cloak(hwnd: HWND, cloak: bool) -> bool {
    let value = BOOL::from(cloak);
    // SAFETY: `value` outlives the call and its size is passed with it.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &value as *const BOOL as *const _,
            std::mem::size_of::<BOOL>() as u32,
        )
    }
    .is_ok()
}

impl FirstFrame {
    /// Before a new window is first shown. True when it is now cloaked.
    pub(crate) fn cloak(&self, hwnd: HWND) -> bool {
        if std::env::var_os("RMAC_GPUI_NO_FIRST_FRAME_CLOAK").is_some() || !set_cloak(hwnd, true) {
            return false;
        }
        self.cloaked.set(true);
        // SAFETY: a timer on our own window; `handle_timer_msg` kills it.
        unsafe { SetTimer(Some(hwnd), UNCLOAK_TIMER_ID, UNCLOAK_AFTER_MS, None) };
        true
    }

    /// After a frame drew, or when the fallback timer fires. True when the
    /// window was cloaked until now (its Mac frame's shadow may show).
    pub(crate) fn reveal(&self, hwnd: HWND) -> bool {
        if !self.cloaked.take() {
            return false;
        }
        set_cloak(hwnd, false);
        // SAFETY: our own window's timer; killing a fired one is harmless.
        unsafe { KillTimer(Some(hwnd), UNCLOAK_TIMER_ID) }.log_err();
        crate::rmac_trace::startup("first_frame_shown");
        true
    }
}
