//! Window pictures on Windows (ADR 0023, "Mission Control on Windows").
//!
//! Each window draws itself into a bitmap through `PrintWindow` with
//! `PW_RENDERFULLCONTENT`, which DWM answers from the window's own
//! redirection surface, so a window another one covers still gets its real
//! picture (a screen crop cannot give it, which is why Lulo OS keeps the
//! last picture of a covered window, MC-01). The pictures are taken in
//! parallel off the UI thread, once per open; a window that does not
//! answer in time keeps its icon card.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use rmac_compositor::WindowId;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic};

use super::{Captured, Pixels};
use crate::model::Scene;

/// `PW_RENDERFULLCONTENT`: DWM's copy, which includes DirectComposition
/// and GPU content (GPUI, browsers) that plain `WM_PRINT` leaves black.
const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
/// How long an open waits for the pictures before it flies in with what
/// it has (a hung app's window keeps its icon card).
const DEADLINE: Duration = Duration::from_millis(400);
/// Nothing bigger than a 4K screen is pictured.
const MAX_PIXELS: i64 = 3840 * 2160;

fn handle(window: WindowId) -> HWND {
    HWND(window.0 as isize as *mut core::ffi::c_void)
}

/// The visible frame (without the invisible resize borders Windows adds
/// round a window) as a rectangle inside the window's own rectangle.
fn frame_in_window(hwnd: HWND, outer: RECT) -> RECT {
    let mut frame = RECT::default();
    // SAFETY: reads a property of a window into a RECT of the right size.
    let read = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if read.is_err() || frame.right <= frame.left || frame.bottom <= frame.top {
        frame = outer;
    }
    let width = outer.right - outer.left;
    let height = outer.bottom - outer.top;
    RECT {
        left: (frame.left - outer.left).clamp(0, width),
        top: (frame.top - outer.top).clamp(0, height),
        right: (frame.right - outer.left).clamp(0, width),
        bottom: (frame.bottom - outer.top).clamp(0, height),
    }
}

/// One window's picture as opaque BGRA, cut to its visible frame.
fn picture(window: WindowId) -> Option<Pixels> {
    let hwnd = handle(window);
    let mut outer = RECT::default();
    // SAFETY: reads properties of a window the window list named; a window
    // that has gone since only makes the calls fail.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            return None;
        }
        GetWindowRect(hwnd, &mut outer).ok()?;
    }
    let width = outer.right - outer.left;
    let height = outer.bottom - outer.top;
    if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > MAX_PIXELS {
        return None;
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: rows top to bottom.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let crop = frame_in_window(hwnd, outer);
    // SAFETY: a DIB section sized from `info`, selected into a memory DC
    // for the one PrintWindow call, read only within its rows, and deleted
    // with the DCs before returning.
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            return None;
        };
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let printed = PrintWindow(hwnd, memory, PW_RENDERFULLCONTENT).as_bool();
        let mut pixels = None;
        if printed && !bits.is_null() {
            let stride = width as usize * 4;
            let source = std::slice::from_raw_parts(bits as *const u8, stride * height as usize);
            let (crop_width, crop_height) = (
                (crop.right - crop.left).max(0) as usize,
                (crop.bottom - crop.top).max(0) as usize,
            );
            if crop_width > 0 && crop_height > 0 {
                let mut bgra = Vec::with_capacity(crop_width * crop_height * 4);
                for y in crop.top as usize..crop.top as usize + crop_height {
                    let start = y * stride + crop.left as usize * 4;
                    bgra.extend_from_slice(&source[start..start + crop_width * 4]);
                }
                for alpha in bgra.iter_mut().skip(3).step_by(4) {
                    *alpha = 0xFF;
                }
                pixels = Some((crop_width as u32, crop_height as u32, bgra));
            }
        }
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        pixels
    }
}

/// Picture every open window of the scene. Windows has no picture of a
/// whole Space, so the Spaces bar shows its window plates.
pub fn capture_scene(scene: &Scene) -> Captured {
    let (sender, pictures) = mpsc::channel();
    let mut asked = 0usize;
    for window in scene.windows.iter().filter(|window| !window.minimised) {
        let id = window.id;
        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("mission-control-picture".into())
            .spawn(move || {
                let _ = sender.send((id, picture(id)));
            });
        if spawned.is_ok() {
            asked += 1;
        }
    }
    drop(sender);
    let deadline = Instant::now() + DEADLINE;
    let mut windows = Vec::with_capacity(asked);
    for _ in 0..asked {
        let left = deadline.saturating_duration_since(Instant::now());
        match pictures.recv_timeout(left) {
            Ok((id, Some(pixels))) => windows.push((id, pixels)),
            Ok((_, None)) => {}
            Err(_) => break,
        }
    }
    Captured {
        desktop: None,
        windows,
    }
}
