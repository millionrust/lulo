//! rmac: `PlatformWindow::set_input_region` on Windows (ADR 0023, "Phase 3
//! revised: shared shell views"; docs/decisions/0025-vendor-gpui-windows.md).
//!
//! Lulo's shell surfaces cover their whole display and take input only
//! where they draw something (the menu bar's strip and open menus, the
//! Dock's shelf), as wlr-layer-shell's input regions do on Lulo OS. On
//! Windows the region becomes the window's region (`SetWindowRgn`): outside
//! it a click reaches the window below, and nothing is drawn there either,
//! which is the same thing for a surface that is clear outside its region.
//! A region equal to the last one is not set again, since surfaces set it on
//! every frame.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{Bounds, Pixels};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_OR};

type Rect = (i32, i32, i32, i32);

thread_local! {
    static LAST: RefCell<HashMap<isize, Option<Vec<Rect>>>> = RefCell::new(HashMap::new());
}

pub(crate) fn set(hwnd: HWND, region: Option<&[Bounds<Pixels>]>, scale: f32) {
    let rects = region.map(|bounds| {
        bounds
            .iter()
            .map(|bounds| {
                let left = (bounds.origin.x.as_f32() * scale).floor() as i32;
                let top = (bounds.origin.y.as_f32() * scale).floor() as i32;
                let right = ((bounds.origin.x + bounds.size.width).as_f32() * scale).ceil() as i32;
                let bottom =
                    ((bounds.origin.y + bounds.size.height).as_f32() * scale).ceil() as i32;
                (left, top, right, bottom)
            })
            .collect::<Vec<_>>()
    });
    let key = hwnd.0 as isize;
    let unchanged = LAST.with(|last| last.borrow().get(&key) == Some(&rects));
    if unchanged {
        return;
    }
    LAST.with(|last| last.borrow_mut().insert(key, rects.clone()));
    // SAFETY: regions are created here and either handed to the window
    // (which then owns it) or deleted; the window belongs to this process.
    unsafe {
        let Some(rects) = rects else {
            let _ = SetWindowRgn(hwnd, None, true);
            return;
        };
        let combined = CreateRectRgn(0, 0, 0, 0);
        for (left, top, right, bottom) in rects {
            let part = CreateRectRgn(left, top, right, bottom);
            let _ = CombineRgn(Some(combined), Some(combined), Some(part), RGN_OR);
            let _ = DeleteObject(part.into());
        }
        if SetWindowRgn(hwnd, Some(combined), true) == 0 {
            let _ = DeleteObject(combined.into());
        }
    }
}

/// The window is gone: forget its last region.
pub(crate) fn forget(hwnd: HWND) {
    LAST.with(|last| last.borrow_mut().remove(&(hwnd.0 as isize)));
}
