//! rmac: the Mac window frame round Lulo app windows (ADR 0023 "Window
//! chrome", WIN-OS-73; docs/decisions/0025-vendor-gpui-windows.md).
//!
//! On Lulo OS niri rounds every app window (16 pt, 27 pt with a unified
//! toolbar), casts the Mac's soft shadow and `rmac-ui` asks nothing of the
//! platform. Windows 11 rounds windows at a fixed 8 px that cannot be
//! enlarged, so a window that asks for a frame (`set_frame_style`, made by
//! `rmac-ui` from Lulo's tokens) is drawn this way instead:
//!
//! - **Corners:** DirectComposition clips the swap chain's visual to a
//!   rectangle rounded at the window's radius, anti-aliased by DWM while it
//!   composes (`DirectXRenderer::set_corner_radius`). Nothing runs per
//!   frame, and a window region (aliased, jagged) is not used.
//! - **Shadow and hairline:** a layered, click-through tool window owned by
//!   the app window (`WS_EX_LAYERED | WS_EX_TRANSPARENT`), so every click on
//!   the shadow reaches whatever is below it, in any process (an
//!   `HTTRANSPARENT` margin inside the app window only passes clicks to the
//!   same thread's windows). Being owned, it sits just above the app window
//!   and hides and shows with it; its pixels over the window are clear but
//!   for the 1 px hairline, drawn along the rounded edge. It is painted once
//!   per size, radius, scale and active state (`UpdateLayeredWindow`), from
//!   a cached nine-slice of the shadow, and only moved otherwise.
//! - **DWM's own frame is off** for these windows: no 1 px frame extension
//!   (which turned DWM's shadow on), no Windows 11 rounding and no DWM
//!   border, so nothing doubles up. Non-client rendering stays on.
//! - **Maximised, full-screen and snapped** windows (`IsWindowArranged`)
//!   have square corners and no shadow, as tiled windows on Lulo OS.
//!
//! Resizing is unchanged: the system's invisible resize borders lie just
//! outside the visible edge, with the system's own cursors, and Snap
//! layouts keep working. Without DirectComposition the window keeps DWM's
//! frame, with the hairline as DWM's border colour.

use std::cell::{Cell, RefCell};
use std::sync::{Once, OnceLock};

use gpui::{Rgba, WindowFrameStyle};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
    DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, ClientToScreen,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GW_HWNDPREV, GWL_EXSTYLE, GetClientRect, GetWindow,
    GetWindowLongPtrW, HTTRANSPARENT, HWND_TOP, IsIconic, IsWindowVisible, MA_NOACTIVATE,
    RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WM_MOUSEACTIVATE, WM_NCHITTEST,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    WS_POPUP,
};
use windows::core::{BOOL, PCSTR, w};

use crate::DirectXRenderer;

const SHADOW_CLASS_NAME: windows::core::PCWSTR = w!("LuloWindowShadow");

/// The hairline's width in device pixels: the Mac's is one device pixel.
const EDGE_WIDTH: f32 = 1.0;

/// One window's frame: its style, its shadow window and what was last drawn.
#[derive(Default)]
pub(crate) struct MacFrame {
    style: Cell<Option<WindowFrameStyle>>,
    shadow: Cell<Option<HWND>>,
    active: Cell<bool>,
    drawn: Cell<Option<Drawn>>,
    /// Nine-slice sources, at most one for the active and one for the
    /// inactive shadow.
    tiles: RefCell<Vec<Tile>>,
}

/// What the shadow window shows, so a move does not repaint it.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    width: i32,
    height: i32,
    geometry: Geometry,
}

impl MacFrame {
    /// `rmac-ui` asked for a Mac frame (again, after a theme change).
    pub(crate) fn set_style(
        &self,
        hwnd: HWND,
        style: WindowFrameStyle,
        renderer: &RefCell<DirectXRenderer>,
        scale: f32,
        maximized: bool,
    ) {
        let composed = renderer
            .try_borrow()
            .map(|renderer| renderer.uses_direct_composition())
            .unwrap_or(true);
        if !composed {
            // DWM keeps its frame; the hairline is its border colour.
            set_dwm_border(hwnd, colorref(style.edge));
            return;
        }
        let first = self.style.get().is_none();
        self.style.set(Some(style));
        if first {
            turn_off_dwm_frame(hwnd);
            self.active.set(unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow() == hwnd
            });
        }
        self.update(hwnd, renderer, scale, maximized);
    }

    pub(crate) fn set_active(&self, active: bool) {
        self.active.set(active);
    }

    /// The window moved, resized, changed state, was shown or hidden, or
    /// changed activation: round its corners and place its shadow to match.
    pub(crate) fn update(
        &self,
        hwnd: HWND,
        renderer: &RefCell<DirectXRenderer>,
        scale: f32,
        flat: bool,
    ) {
        let Some(style) = self.style.get() else {
            return;
        };
        // SAFETY: reads the state of a window this thread owns.
        let shown = unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() };
        let flat = flat || is_arranged(hwnd);
        let radius = if flat {
            0.0
        } else {
            (style.corner_radius.as_f32() * scale).round()
        };
        if let Ok(mut renderer) = renderer.try_borrow_mut() {
            renderer.set_corner_radius(radius);
        }
        if !shown || flat {
            self.hide_shadow();
            return;
        }
        let Some(body) = client_rect_on_screen(hwnd) else {
            return;
        };
        let Some(shadow) = self.shadow_window(hwnd) else {
            return;
        };
        let shadow_style = if self.active.get() {
            style.active_shadow
        } else {
            style.inactive_shadow
        };
        let geometry = Geometry {
            radius,
            blur: (shadow_style.blur.as_f32() * scale).round(),
            offset_y: (shadow_style.offset_y.as_f32() * scale).round(),
            alpha: shadow_style.color.a.clamp(0.0, 1.0),
            edge: style.edge,
            edge_width: EDGE_WIDTH,
        };
        let margins = geometry.margins();
        let origin = POINT {
            x: body.left - margins.left,
            y: body.top - margins.top,
        };
        let drawn = Drawn {
            width: body.right - body.left,
            height: body.bottom - body.top,
            geometry,
        };
        if self.drawn.get() == Some(drawn) {
            // SAFETY: moves a window this thread owns.
            let _ = unsafe {
                SetWindowPos(
                    shadow,
                    None,
                    origin.x,
                    origin.y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
        } else {
            let canvas = self.canvas(&drawn.geometry, drawn.width, drawn.height);
            if paint_layered(shadow, origin, &canvas) {
                self.drawn.set(Some(drawn));
            }
        }
        show_above(shadow, hwnd);
    }

    fn hide_shadow(&self) {
        if let Some(shadow) = self.shadow.get() {
            // SAFETY: hides a window this thread owns.
            unsafe {
                if IsWindowVisible(shadow).as_bool() {
                    let _ = ShowWindow(shadow, SW_HIDE);
                }
            }
        }
    }

    /// The shadow window, made the first time it is needed. It is owned by
    /// `hwnd`, so the system destroys it with the app window.
    fn shadow_window(&self, hwnd: HWND) -> Option<HWND> {
        if let Some(shadow) = self.shadow.get() {
            return Some(shadow);
        }
        register_shadow_class();
        // SAFETY: creates a window owned by `hwnd`, on `hwnd`'s thread.
        let shadow = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                SHADOW_CLASS_NAME,
                w!(""),
                WS_POPUP,
                0,
                0,
                0,
                0,
                Some(hwnd),
                None,
                Some(crate::get_module_handle().into()),
                None,
            )
        }
        .ok()?;
        let disabled = BOOL(1);
        // SAFETY: a BOOL-sized attribute of a window this thread owns. DWM
        // would otherwise fade the shadow in and out on its own.
        let _ = unsafe {
            DwmSetWindowAttribute(
                shadow,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &disabled as *const BOOL as *const core::ffi::c_void,
                std::mem::size_of::<BOOL>() as u32,
            )
        };
        self.shadow.set(Some(shadow));
        Some(shadow)
    }

    /// The shadow window's pixels for a `width` × `height` window: the
    /// cached nine-slice stretched to it, or (for a window smaller than the
    /// slice's centre) drawn directly.
    fn canvas(&self, geometry: &Geometry, width: i32, height: i32) -> Canvas {
        let reference = geometry.reference_size();
        if width < reference || height < reference {
            return render(geometry, width, height);
        }
        let mut tiles = self.tiles.borrow_mut();
        if let Some(tile) = tiles.iter().find(|tile| tile.geometry == *geometry) {
            return tile.stretch(width, height);
        }
        let tile = Tile {
            geometry: *geometry,
            canvas: render(geometry, reference, reference),
            body: reference,
        };
        let canvas = tile.stretch(width, height);
        if tiles.len() >= 2 {
            tiles.remove(0);
        }
        tiles.push(tile);
        canvas
    }
}

/// Show `shadow`, if hidden, directly above its window `owner` in the
/// z-order, without activating it: showing it plainly would put it on top
/// of every window, over other apps' windows when its own is behind them.
fn show_above(shadow: HWND, owner: HWND) {
    // SAFETY: reads and places windows this thread owns.
    unsafe {
        if IsWindowVisible(shadow).as_bool() {
            return;
        }
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW;
        // The window just above `owner`: the shadow goes below it. When that
        // is the shadow already, it keeps its place; when it is a topmost
        // window (the taskbar), `owner` tops the ordinary windows and the
        // shadow goes on top of them, still out of the topmost band.
        let topmost =
            |hwnd: HWND| GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
        let (after, flags) = match GetWindow(owner, GW_HWNDPREV) {
            Ok(above) if above == shadow => (None, flags | SWP_NOZORDER),
            Ok(above) if !above.is_invalid() && !topmost(above) => (Some(above), flags),
            _ => (Some(HWND_TOP), flags),
        };
        let _ = SetWindowPos(shadow, after, 0, 0, 0, 0, flags);
    }
}

/// The DWM frame `configure_mac_window_frame` gave the window, taken off.
fn turn_off_dwm_frame(hwnd: HWND) {
    let margins = MARGINS::default();
    // SAFETY: plain DWM calls on a window this thread owns; Windows 10
    // refuses the two Windows 11 attributes, which changes nothing there.
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        let corner: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const core::ffi::c_void,
            std::mem::size_of_val(&corner) as u32,
        );
    }
    set_dwm_border(hwnd, COLORREF(DWMWA_COLOR_NONE));
}

fn set_dwm_border(hwnd: HWND, colour: COLORREF) {
    // SAFETY: a COLORREF-sized attribute of a window this thread owns.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &colour as *const COLORREF as *const core::ffi::c_void,
            std::mem::size_of::<COLORREF>() as u32,
        )
    };
}

/// COLORREF is 0x00BBGGRR.
fn colorref(colour: Rgba) -> COLORREF {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    COLORREF(channel(colour.r) | (channel(colour.g) << 8) | (channel(colour.b) << 16))
}

/// Whether Windows snapped the window (Snap layouts, Win+arrows, a drag to
/// an edge). `IsWindowArranged` is looked up, not linked: older Windows 10
/// builds do not have it, and a window there is never "arranged".
fn is_arranged(hwnd: HWND) -> bool {
    type IsWindowArranged = unsafe extern "system" fn(HWND) -> BOOL;
    static FUNCTION: OnceLock<Option<IsWindowArranged>> = OnceLock::new();
    let function = FUNCTION.get_or_init(|| {
        // SAFETY: looks up an export of user32, which every GUI process has
        // loaded, and gives it its documented signature.
        unsafe {
            let user32 = GetModuleHandleA(PCSTR(c"user32.dll".as_ptr() as *const u8)).ok()?;
            let address = GetProcAddress(user32, PCSTR(c"IsWindowArranged".as_ptr() as *const u8))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                IsWindowArranged,
            >(address))
        }
    });
    // SAFETY: the documented signature, on a window this thread owns.
    function.is_some_and(|function| unsafe { function(hwnd) }.as_bool())
}

/// The window's client area (the visible window: its swap chain) in screen
/// pixels.
fn client_rect_on_screen(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: out-parameters of the right size, on a window this thread owns.
    unsafe {
        GetClientRect(hwnd, &mut rect).ok()?;
        if !ClientToScreen(hwnd, &mut origin).as_bool() {
            return None;
        }
    }
    Some(RECT {
        left: origin.x,
        top: origin.y,
        right: origin.x + rect.right - rect.left,
        bottom: origin.y + rect.bottom - rect.top,
    })
}

fn register_shadow_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(shadow_procedure),
            lpszClassName: SHADOW_CLASS_NAME,
            hInstance: crate::get_module_handle().into(),
            ..Default::default()
        };
        // SAFETY: registers a class whose procedure is below.
        unsafe { RegisterClassW(&class) };
    });
}

unsafe extern "system" fn shadow_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Never in the way of a click, never activated.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: the default procedure, for a window of this class.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Copy `canvas` into a DIB and hand it to the layered window at `origin`.
fn paint_layered(shadow: HWND, origin: POINT, canvas: &Canvas) -> bool {
    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: canvas.width as i32,
        // Top-down rows.
        biHeight: -(canvas.height as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        ..Default::default()
    };
    // SAFETY: a DIB section of exactly `canvas`'s size is filled from it and
    // selected into a memory DC only for the `UpdateLayeredWindow` call;
    // every GDI object made here is released before returning.
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(None, screen);
            return false;
        };
        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(
                canvas.pixels.as_ptr(),
                bits as *mut u32,
                canvas.pixels.len(),
            );
        }
        let previous = SelectObject(memory, bitmap.into());
        let size = SIZE {
            cx: canvas.width as i32,
            cy: canvas.height as i32,
        };
        let source = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let painted = !bits.is_null()
            && UpdateLayeredWindow(
                shadow,
                Some(screen),
                Some(&origin as *const POINT),
                Some(&size as *const SIZE),
                Some(memory),
                Some(&source as *const POINT),
                COLORREF(0),
                Some(&blend as *const BLENDFUNCTION),
                ULW_ALPHA,
            )
            .is_ok();
        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
        painted
    }
}

/// The frame's measurements in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    radius: f32,
    /// CSS blur radius: the Gaussian's 2σ.
    blur: f32,
    offset_y: f32,
    alpha: f32,
    edge: Rgba,
    edge_width: f32,
}

/// The clear margin the shadow window adds on each side of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Margins {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl Geometry {
    /// As CSS's `box-shadow`: the shadow reaches `blur` past the window,
    /// moved down by the offset.
    fn margins(&self) -> Margins {
        let blur = self.blur.max(0.0).ceil() as i32;
        let offset = self.offset_y.round() as i32;
        Margins {
            left: blur,
            right: blur,
            top: (blur - offset).max(0),
            bottom: (blur + offset).max(0),
        }
    }

    /// The window size the nine-slice is drawn for: big enough that its
    /// middle row and column are clear of the corners and of the blur.
    fn reference_size(&self) -> i32 {
        let reach = self.radius.ceil() as i32
            + blur_reach(self.blur)
            + self.offset_y.abs().ceil() as i32
            + self.edge_width.ceil() as i32
            + 2;
        2 * reach + 1
    }
}

/// Premultiplied BGRA pixels (`0xAARRGGBB`), rows top-down.
#[derive(Clone, Debug, PartialEq)]
struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

/// A frame drawn for a `body` × `body` window, stretched to any larger one
/// by repeating its middle row and column.
struct Tile {
    geometry: Geometry,
    canvas: Canvas,
    body: i32,
}

impl Tile {
    fn stretch(&self, width: i32, height: i32) -> Canvas {
        let margins = self.geometry.margins();
        let out_width = (margins.left + width + margins.right) as usize;
        let out_height = (margins.top + height + margins.bottom) as usize;
        let tile = &self.canvas;
        let centre_x = (margins.left + self.body / 2) as usize;
        let centre_y = (margins.top + self.body / 2) as usize;
        let extra_x = out_width - tile.width;
        let extra_y = out_height - tile.height;
        let mut pixels = vec![0u32; out_width * out_height];
        for y in 0..out_height {
            let source_y = if y < centre_y {
                y
            } else if y >= centre_y + extra_y {
                y - extra_y
            } else {
                centre_y
            };
            let source = &tile.pixels[source_y * tile.width..(source_y + 1) * tile.width];
            let row = &mut pixels[y * out_width..(y + 1) * out_width];
            row[..centre_x].copy_from_slice(&source[..centre_x]);
            let middle = source[centre_x];
            if middle != 0 {
                row[centre_x..centre_x + extra_x].fill(middle);
            }
            row[centre_x + extra_x..].copy_from_slice(&source[centre_x..]);
        }
        Canvas {
            width: out_width,
            height: out_height,
            pixels,
        }
    }
}

/// How far three box blurs approximating a Gaussian of 2σ = `blur` reach.
fn blur_reach(blur: f32) -> i32 {
    box_radii(blur / 2.0).iter().sum()
}

/// Radii of three box blurs whose sum approximates a Gaussian of `sigma`
/// ("Fast almost-Gaussian filtering", Kovesi).
fn box_radii(sigma: f32) -> [i32; 3] {
    if sigma < 0.5 {
        return [0; 3];
    }
    let passes = 3.0f32;
    let ideal = (12.0 * sigma * sigma / passes + 1.0).sqrt();
    let mut lower = ideal.floor() as i32;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let upper = lower + 2;
    let lower_f = lower as f32;
    let count = ((12.0 * sigma * sigma
        - passes * lower_f * lower_f
        - 4.0 * passes * lower_f
        - 3.0 * passes)
        / (-4.0 * lower_f - 4.0))
        .round() as i32;
    let mut radii = [0; 3];
    for (index, radius) in radii.iter_mut().enumerate() {
        let width = if (index as i32) < count { lower } else { upper };
        *radius = (width - 1) / 2;
    }
    radii
}

/// How much of the pixel centred at (`x`, `y`) a `width` × `height`
/// rectangle at the origin with corners rounded at `radius` covers.
fn coverage(x: f32, y: f32, width: f32, height: f32, radius: f32) -> f32 {
    if width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    let radius = radius.clamp(0.0, width.min(height) / 2.0);
    let half_width = width / 2.0;
    let half_height = height / 2.0;
    let qx = (x - half_width).abs() - (half_width - radius);
    let qy = (y - half_height).abs() - (half_height - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let distance = outside + qx.max(qy).min(0.0) - radius;
    (0.5 - distance).clamp(0.0, 1.0)
}

/// One running-sum box blur of `radius` along `count` samples `stride`
/// apart starting at `start`, using `scratch` for the source.
fn box_blur_line(
    values: &mut [f32],
    scratch: &mut Vec<f32>,
    start: usize,
    stride: usize,
    count: usize,
    radius: i32,
) {
    if radius <= 0 {
        return;
    }
    scratch.clear();
    scratch.extend((0..count).map(|index| values[start + index * stride]));
    let radius = radius as usize;
    let scale = 1.0 / (2 * radius + 1) as f32;
    let mut sum: f32 = scratch.iter().take(radius.min(count)).sum();
    for index in 0..count {
        if index + radius < count {
            sum += scratch[index + radius];
        }
        if index > radius {
            sum -= scratch[index - radius - 1];
        }
        values[start + index * stride] = sum * scale;
    }
}

/// The frame for a `width` × `height` window: the shadow (black, cut out
/// under the window) with the hairline drawn over the window's edge.
fn render(geometry: &Geometry, width: i32, height: i32) -> Canvas {
    let margins = geometry.margins();
    let canvas_width = (margins.left + width + margins.right).max(1) as usize;
    let canvas_height = (margins.top + height + margins.bottom).max(1) as usize;
    let radii = box_radii(geometry.blur / 2.0);
    let pad = radii.iter().sum::<i32>() as usize + 1;
    let grid_width = canvas_width + 2 * pad;
    let grid_height = canvas_height + 2 * pad;
    let (body_width, body_height) = (width as f32, height as f32);
    // The window's shape, moved down by the offset, then blurred.
    let shape_left = (pad as i32 + margins.left) as f32;
    let shape_top = (pad as i32 + margins.top) as f32 + geometry.offset_y;
    let mut shadow = vec![0f32; grid_width * grid_height];
    for y in 0..grid_height {
        for x in 0..grid_width {
            shadow[y * grid_width + x] = coverage(
                x as f32 + 0.5 - shape_left,
                y as f32 + 0.5 - shape_top,
                body_width,
                body_height,
                geometry.radius,
            );
        }
    }
    let mut scratch = Vec::with_capacity(grid_width.max(grid_height));
    for radius in radii {
        for y in 0..grid_height {
            box_blur_line(
                &mut shadow,
                &mut scratch,
                y * grid_width,
                1,
                grid_width,
                radius,
            );
        }
    }
    for radius in radii {
        for x in 0..grid_width {
            box_blur_line(
                &mut shadow,
                &mut scratch,
                x,
                grid_width,
                grid_height,
                radius,
            );
        }
    }
    let edge = geometry.edge;
    let edge_alpha = edge.a.clamp(0.0, 1.0);
    let inset = geometry.edge_width;
    let mut pixels = vec![0u32; canvas_width * canvas_height];
    for y in 0..canvas_height {
        for x in 0..canvas_width {
            let px = x as f32 + 0.5 - margins.left as f32;
            let py = y as f32 + 0.5 - margins.top as f32;
            let body = coverage(px, py, body_width, body_height, geometry.radius);
            let inner = coverage(
                px - inset,
                py - inset,
                body_width - 2.0 * inset,
                body_height - 2.0 * inset,
                (geometry.radius - inset).max(0.0),
            );
            let hairline = (body - inner).clamp(0.0, 1.0) * edge_alpha;
            let cast = shadow[(y + pad) * grid_width + x + pad] * geometry.alpha * (1.0 - body);
            let alpha = hairline + cast * (1.0 - hairline);
            if alpha <= 0.0 {
                continue;
            }
            let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
            pixels[y * canvas_width + x] = (channel(alpha) << 24)
                | (channel(edge.r * hairline) << 16)
                | (channel(edge.g * hairline) << 8)
                | channel(edge.b * hairline);
        }
    }
    Canvas {
        width: canvas_width,
        height: canvas_height,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> Geometry {
        Geometry {
            radius: 27.0,
            blur: 42.0,
            offset_y: 16.0,
            alpha: 0xbd as f32 / 255.0,
            edge: Rgba {
                r: 0.5,
                g: 0.5,
                b: 0.5,
                a: 1.0,
            },
            edge_width: 1.0,
        }
    }

    fn alpha(canvas: &Canvas, x: usize, y: usize) -> u32 {
        canvas.pixels[y * canvas.width + x] >> 24
    }

    #[test]
    fn margins_follow_the_blur_and_the_offset() {
        assert_eq!(
            geometry().margins(),
            Margins {
                left: 42,
                top: 26,
                right: 42,
                bottom: 58,
            }
        );
    }

    #[test]
    fn three_boxes_match_the_gaussian() {
        for sigma in [4.0f32, 13.0, 21.0] {
            let variance: f32 = box_radii(sigma)
                .iter()
                .map(|radius| {
                    let width = (2 * radius + 1) as f32;
                    (width * width - 1.0) / 12.0
                })
                .sum();
            assert!(
                (variance.sqrt() - sigma).abs() < 1.0,
                "{sigma}: {}",
                variance.sqrt()
            );
        }
    }

    #[test]
    fn the_stretched_tile_is_the_frame_drawn_directly() {
        let geometry = geometry();
        let reference = geometry.reference_size();
        let tile = Tile {
            geometry,
            canvas: render(&geometry, reference, reference),
            body: reference,
        };
        let (width, height) = (reference + 37, reference + 11);
        let stretched = tile.stretch(width, height);
        let direct = render(&geometry, width, height);
        assert_eq!(
            (stretched.width, stretched.height),
            (direct.width, direct.height)
        );
        for (index, (a, b)) in stretched.pixels.iter().zip(&direct.pixels).enumerate() {
            let (a, b) = (a >> 24, b >> 24);
            assert!(
                a.abs_diff(b) <= 2,
                "pixel {} {}: {a} vs {b}",
                index % direct.width,
                index / direct.width
            );
        }
    }

    #[test]
    fn the_window_is_clear_inside_but_for_its_hairline() {
        let geometry = geometry();
        let canvas = render(&geometry, 400, 300);
        let margins = geometry.margins();
        let (left, top) = (margins.left as usize, margins.top as usize);
        // The middle of the window shows the window itself.
        assert_eq!(alpha(&canvas, left + 200, top + 150), 0);
        // The hairline along the top edge, away from the corners.
        assert_eq!(alpha(&canvas, left + 200, top), 255);
        assert_eq!(alpha(&canvas, left + 200, top + 2), 0);
    }

    #[test]
    fn the_shadow_fades_out_and_is_deeper_below() {
        let geometry = geometry();
        let canvas = render(&geometry, 400, 300);
        let margins = geometry.margins();
        let x = margins.left as usize + 200;
        let below = margins.top as usize + 300;
        let near = alpha(&canvas, x, below + 2);
        let far = alpha(&canvas, x, below + 40);
        let above = alpha(&canvas, x, margins.top as usize - 3);
        assert!(near > 60, "near {near}");
        assert!(far < near / 3, "far {far}, near {near}");
        assert!(above < near, "above {above}, below {near}");
        // Nothing at the canvas's own edge.
        assert!(alpha(&canvas, x, canvas.height - 1) < 8);
        assert!(alpha(&canvas, 0, margins.top as usize + 150) < 8);
    }

    #[test]
    fn the_rounded_corner_is_clear_outside_the_arc_and_shaded_there() {
        let geometry = geometry();
        let canvas = render(&geometry, 400, 300);
        let margins = geometry.margins();
        let (left, top) = (margins.left as usize, margins.top as usize);
        // The window's own top-left corner pixel lies outside a 27 px arc:
        // no hairline there, only (faint) shadow, never opaque.
        let corner = canvas.pixels[top * canvas.width + left];
        assert_eq!(corner & 0x00ff_ffff, 0, "no hairline colour at the corner");
        assert!(corner >> 24 < 128);
        // The hairline follows the arc: at 45°, r(1 - 1/√2) ≈ 7.9 px in.
        let diagonal = (27.0 * (1.0 - std::f32::consts::FRAC_1_SQRT_2)).ceil() as usize;
        assert!(alpha(&canvas, left + diagonal, top + diagonal) > 100);
    }

    #[test]
    fn colourref_is_bgr() {
        assert_eq!(
            colorref(Rgba {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            }),
            COLORREF(0x0000ff)
        );
    }
}
