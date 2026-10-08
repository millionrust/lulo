//! The hairline round every Lulo app window on Windows (ADR 0023 "Lulo
//! mode", WIN-OS-42).
//!
//! `gpui_windows` gives each app window DWM's soft shadow and Windows 11's
//! rounded corners. The 1 px edge DWM draws round it is the Mac's window
//! hairline here, in Lulo's own appearance rather than Windows': a dark
//! window gets a light rim and a light one a soft grey edge, both made from
//! the theme's separator over the window colour. It is set when a window's
//! root view is made and again whenever the theme changes. Shell surfaces
//! (tool windows: the menu bar, Dock, Spotlight, menus) draw their own
//! edges and are left alone.

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};

use crate::theme::RgbaColor;

/// How much stronger than a separator line the window edge is drawn: the
/// Mac's window rim reads clearly against the desktop, where a separator
/// inside a window is barely there.
const EDGE_STRENGTH: f32 = 2.5;

/// Watch for new app windows and give each its edge.
pub(crate) fn install(cx: &mut gpui::App) {
    cx.observe_new::<crate::window::Root>(|_, window, _| {
        if let Some(window) = window {
            apply(window);
        }
    })
    .detach();
}

/// The edge colour for the current theme, as an opaque colour.
pub(crate) fn edge_colour() -> RgbaColor {
    let colors = crate::theme::current().colors;
    blend(colors.window, colors.separator, EDGE_STRENGTH)
}

/// `over` (with its alpha scaled by `strength`) composited onto `base`.
fn blend(base: RgbaColor, over: RgbaColor, strength: f32) -> RgbaColor {
    let alpha = (f32::from(over.alpha) / 255.0 * strength).clamp(0.0, 1.0);
    let mix = |below: u8, above: u8| {
        (f32::from(below) + (f32::from(above) - f32::from(below)) * alpha).round() as u8
    };
    RgbaColor {
        red: mix(base.red, over.red),
        green: mix(base.green, over.green),
        blue: mix(base.blue, over.blue),
        alpha: 0xff,
    }
}

fn hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// Give `window` the edge for the current theme.
pub(crate) fn apply(window: &Window) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    // SAFETY: reads the style of a window this process owns.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if style & WS_EX_TOOLWINDOW.0 != 0 {
        return;
    }
    let edge = edge_colour();
    // COLORREF is 0x00BBGGRR.
    let colour =
        COLORREF(u32::from(edge.red) | (u32::from(edge.green) << 8) | (u32::from(edge.blue) << 16));
    // SAFETY: a COLORREF-sized attribute of a window this process owns.
    // Windows 10 does not know the attribute and refuses it; nothing else
    // changes there.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &colour as *const COLORREF as *const core::ffi::c_void,
            std::mem::size_of::<COLORREF>() as u32,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edge_is_the_separator_drawn_stronger_over_the_window() {
        let window = RgbaColor::opaque(0x1e1e1e);
        let separator = RgbaColor::with_alpha(0xffffff, 0x26);
        let edge = blend(window, separator, EDGE_STRENGTH);
        assert_eq!(edge.alpha, 0xff);
        assert!(edge.red > window.red && edge.red < 0xff);
        assert!(edge.green == edge.red && edge.blue == edge.red);
        let light = blend(
            RgbaColor::opaque(0xffffff),
            RgbaColor::with_alpha(0x000000, 0x14),
            EDGE_STRENGTH,
        );
        assert!(light.red < 0xff && light.red > 0x80);
        // Never more than the separator colour itself.
        let full = blend(window, RgbaColor::opaque(0xffffff), EDGE_STRENGTH);
        assert_eq!(full.red, 0xff);
    }
}
