//! The rounded shape of a blurred material's window on Windows.
//!
//! On Lulo OS niri shapes the blur from the surface itself. On Windows the
//! view draws its blur ([`super::backdrop`]) with rounded corners, and the
//! window is cut to the same shape so presses just outside the corners
//! reach what is below, as they do on Lulo OS.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};

thread_local! {
    static SHAPES: RefCell<HashMap<isize, (i32, i32, i32)>> = RefCell::new(HashMap::new());
}

/// Cut `hwnd` (`width` × `height` physical pixels) to a rounded rectangle
/// with corners of `radius` physical pixels; the same shape again is not
/// set twice.
pub fn shape(hwnd: HWND, width: i32, height: i32, radius: i32) {
    let key = hwnd.0 as isize;
    let shape = (width, height, radius);
    if SHAPES.with(|shapes| shapes.borrow().get(&key) == Some(&shape)) {
        return;
    }
    SHAPES.with(|shapes| shapes.borrow_mut().insert(key, shape));
    // SAFETY: the region is handed to Windows, which owns it from then on.
    unsafe {
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, 2 * radius, 2 * radius);
        if !region.is_invalid() {
            SetWindowRgn(hwnd, Some(region), true);
        }
    }
}

/// The window is gone.
pub fn forget(hwnd: isize) {
    SHAPES.with(|shapes| shapes.borrow_mut().remove(&hwnd));
}
