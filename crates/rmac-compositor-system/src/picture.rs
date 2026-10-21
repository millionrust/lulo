//! A window's picture on Windows: Mission Control's thumbnails and the
//! Dock's minimised tiles (ADR 0023).
//!
//! Each window draws itself into a bitmap through `PrintWindow` with
//! `PW_RENDERFULLCONTENT`, which DWM answers from the window's own
//! redirection surface, so a window another one covers still gets its real
//! picture (a screen crop cannot give it). A minimised window has none.

use rmac_compositor::WindowId;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_NCRENDERING_ENABLED,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect, IsIconic};

/// BGRA pixels: width, height, rows top first.
pub type Pixels = (u32, u32, Vec<u8>);

/// `PW_RENDERFULLCONTENT`: DWM's copy, which includes DirectComposition
/// and GPU content (GPUI, browsers) that plain `WM_PRINT` leaves black.
const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
/// Nothing bigger than a 4K screen is pictured.
const MAX_PIXELS: i64 = 3840 * 2160;

fn handle(window: WindowId) -> HWND {
    HWND(window.0 as isize as *mut core::ffi::c_void)
}

/// Whether DWM draws the window's frame. Lulo's own windows turn it off
/// and draw their frame themselves (`gpui_windows`' `rmac_window_frame`):
/// for them DWM's frame bounds still include the invisible resize borders,
/// which print black, and their client area is the whole visible window.
fn dwm_draws_frame(hwnd: HWND) -> bool {
    let mut enabled = 1i32;
    // SAFETY: reads a BOOL property of a window into an i32.
    let read = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_ENABLED,
            &mut enabled as *mut i32 as *mut core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };
    read.is_err() || enabled != 0
}

/// The client area in screen coordinates.
fn client_on_screen(hwnd: HWND) -> Option<RECT> {
    let mut client = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: reads a window's client rectangle and maps its origin.
    unsafe {
        GetClientRect(hwnd, &mut client).ok()?;
        ClientToScreen(hwnd, &mut origin).ok().ok()?;
    }
    Some(RECT {
        left: origin.x,
        top: origin.y,
        right: origin.x + client.right,
        bottom: origin.y + client.bottom,
    })
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
    if !dwm_draws_frame(hwnd) {
        if let Some(client) = client_on_screen(hwnd) {
            frame = client;
        }
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

/// One window's picture as opaque BGRA, cut to its visible frame; `None`
/// for a minimised window or one that does not draw. Blocking: it waits for
/// the window to answer.
pub fn window_picture(window: WindowId) -> Option<Pixels> {
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
