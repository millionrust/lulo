//! The Mac window frame round every Lulo app window on Windows (ADR 0023
//! "Window chrome", WIN-OS-42 and WIN-OS-73).
//!
//! On Lulo OS niri rounds each app window at Lulo's window radius, casts
//! the Mac's soft shadow and the window's own hairline shows. Windows 11
//! only rounds at a small fixed radius, so each app window asks
//! `gpui_windows` to draw the same frame itself (`Window::set_frame_style`):
//! the radius niri gives this app ([`crate::window::window_corner_radius`]),
//! the key and non-key window shadows (`rmac_design::Elevation`, the values
//! niri is configured with), and the hairline in Lulo's own appearance
//! rather than Windows': a dark window gets a light rim and a light one a
//! soft grey edge, both the theme's separator over the window colour. It is
//! set when a window's root view is made and again whenever the theme
//! changes. Shell surfaces (tool windows: the menu bar, Dock, Spotlight,
//! menus) draw their own shapes and are left alone.

use std::sync::OnceLock;

use gpui::{px, Window, WindowFrameShadow, WindowFrameStyle};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};

use crate::theme::RgbaColor;

/// How much stronger than a separator line the window edge is drawn: the
/// Mac's window rim reads clearly against the desktop, where a separator
/// inside a window is barely there.
const EDGE_STRENGTH: f32 = 2.5;

/// This process's app id, which picks its corner radius.
static APP_ID: OnceLock<&'static str> = OnceLock::new();

/// Remember the app id (the app menu installs it before any window opens).
pub(crate) fn set_app_id(app_id: &'static str) {
    let _ = APP_ID.set(app_id);
}

/// Watch for new app windows and give each its frame.
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

fn gpui_rgba(colour: RgbaColor) -> gpui::Rgba {
    gpui::Rgba {
        r: f32::from(colour.red) / 255.0,
        g: f32::from(colour.green) / 255.0,
        b: f32::from(colour.blue) / 255.0,
        a: f32::from(colour.alpha) / 255.0,
    }
}

fn shadow(shadow: rmac_design::WindowShadow) -> WindowFrameShadow {
    WindowFrameShadow {
        blur: px(shadow.softness),
        offset_y: px(shadow.offset_y),
        color: gpui::Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: (shadow.color.0 & 0xff) as f32 / 255.0,
        },
    }
}

/// The frame for this app's windows in the current theme.
fn style() -> WindowFrameStyle {
    let radii = crate::theme::current().radii;
    let elevation = rmac_design::Elevation::resolve(rmac_appearance::Contrast::Normal);
    WindowFrameStyle {
        corner_radius: px(crate::window::window_corner_radius(
            APP_ID.get().copied().unwrap_or_default(),
            radii.window,
            radii.window_toolbar,
        )),
        edge: gpui_rgba(edge_colour()),
        active_shadow: shadow(elevation.window_active),
        inactive_shadow: shadow(elevation.window_inactive),
    }
}

fn hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// Give `window` the frame for the current theme.
pub(crate) fn apply(window: &Window) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    // SAFETY: reads the style of a window this process owns.
    let style_bits = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if style_bits & WS_EX_TOOLWINDOW.0 != 0 {
        return;
    }
    window.set_frame_style(style());
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

    #[test]
    fn the_shadows_are_niris() {
        let elevation = rmac_design::Elevation::resolve(rmac_appearance::Contrast::Normal);
        let active = shadow(elevation.window_active);
        assert_eq!(active.blur, px(42.0));
        assert_eq!(active.offset_y, px(16.0));
        assert!((active.color.a - 0xbd as f32 / 255.0).abs() < 1e-6);
        let inactive = shadow(elevation.window_inactive);
        assert_eq!(inactive.blur, px(26.0));
        assert!((inactive.color.a - 0x75 as f32 / 255.0).abs() < 1e-6);
    }
}
