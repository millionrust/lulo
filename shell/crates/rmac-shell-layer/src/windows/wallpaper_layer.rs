//! Lulo mode's wallpaper as a layered window right below the desktop
//! window (ADR 0023 "Lulo mode", WIN-OS-53; "Phase 3 revised: shared shell
//! views": Lulo OS's desktop view hands its decoded picture over with
//! [`show_picture`] and draws only its icons).
//!
//! Drawn by GPUI, the wallpaper cost lulo-shell about 18 MB of private
//! memory on the owner's PC (Radeon, 1366 × 768): the picture's pixels kept
//! for the image, a screen-sized atlas texture and the driver's copies of
//! its upload. Here the picture goes to Windows once with
//! `UpdateLayeredWindow`, which keeps its own copy for the compositor, and
//! the shell frees its pixels straight after. The desktop window above it
//! is transparent where it draws nothing, so the picture shows under the
//! icons. The layer takes no input (`WS_EX_TRANSPARENT`, never activated),
//! is out of Alt+Tab and the taskbar, and is put back directly below the
//! desktop window whenever that is placed (`desktop_layer`).

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, SetWindowPos, UpdateLayeredWindow,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS: windows::core::PCWSTR = w!("LuloWallpaperLayer");

/// The layer's window, for [`keep_below`].
static LAYER: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the default handling of every message.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn register() -> bool {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    *REGISTERED.get_or_init(|| {
        // SAFETY: a class with a static name and the default procedure.
        unsafe {
            let Ok(instance) = GetModuleHandleW(None) else {
                return false;
            };
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassW(&class) != 0
        }
    })
}

/// Put the layer directly below `desktop` in the z-order (after the
/// desktop window itself was placed).
pub fn keep_below(desktop: HWND) {
    let layer = LAYER.load(Ordering::Acquire);
    if layer == 0 {
        return;
    }
    // SAFETY: positions a window this process owns.
    let _ = unsafe {
        SetWindowPos(
            HWND(layer as *mut core::ffi::c_void),
            Some(desktop),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

/// Whether `hwnd` is the wallpaper layer.
pub fn is_layer(hwnd: HWND) -> bool {
    let layer = LAYER.load(Ordering::Acquire);
    layer != 0 && hwnd.0 as isize == layer
}

/// The wallpaper layer under the desktop window.
pub struct Layer {
    hwnd: HWND,
}

impl Layer {
    /// Make the (still empty, hidden) layer.
    pub fn new() -> Option<Self> {
        if !register() {
            return None;
        }
        // SAFETY: a top-level window of this process.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                CLASS,
                w!("Lulo wallpaper"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                None,
            )
        }
        .ok()?;
        LAYER.store(hwnd.0 as isize, Ordering::Release);
        Some(Self { hwnd })
    }

    /// Show `width` × `height` straight-alpha BGRA pixels (`bgra`, opaque)
    /// at `left`, `top` of `screen` (physical pixels), with `backdrop`
    /// (BGR) elsewhere, directly below `desktop`. Windows keeps its own
    /// copy; the caller can free `bgra` at once.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &self,
        desktop: HWND,
        screen: RECT,
        left: u32,
        top: u32,
        width: u32,
        height: u32,
        bgra: &[u8],
        backdrop: [u8; 3],
    ) -> bool {
        let screen_width = (screen.right - screen.left).max(0) as u32;
        let screen_height = (screen.bottom - screen.top).max(0) as u32;
        if screen_width == 0
            || screen_height == 0
            || bgra.len() < width as usize * height as usize * 4
        {
            return false;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: screen_width as i32,
                // Negative: rows top to bottom.
                biHeight: -(screen_height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: a DIB section sized from `info`, written only within its
        // rows, selected into a memory DC for the one call and deleted
        // after it; the window is this process's own.
        unsafe {
            let screen_dc = GetDC(None);
            let memory_dc = CreateCompatibleDC(Some(screen_dc));
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bitmap) =
                CreateDIBSection(Some(memory_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(memory_dc);
                ReleaseDC(None, screen_dc);
                return false;
            };
            let stride = screen_width as usize * 4;
            let pixels =
                std::slice::from_raw_parts_mut(bits as *mut u8, stride * screen_height as usize);
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[backdrop[0], backdrop[1], backdrop[2], 255]);
            }
            let copy_width = width.min(screen_width.saturating_sub(left)) as usize * 4;
            for row in 0..height.min(screen_height.saturating_sub(top)) as usize {
                let source = row * width as usize * 4;
                let target = (top as usize + row) * stride + left as usize * 4;
                pixels[target..target + copy_width]
                    .copy_from_slice(&bgra[source..source + copy_width]);
            }
            let previous = SelectObject(memory_dc, HGDIOBJ(bitmap.0));
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let destination = POINT {
                x: screen.left,
                y: screen.top,
            };
            let source = POINT { x: 0, y: 0 };
            let size = SIZE {
                cx: screen_width as i32,
                cy: screen_height as i32,
            };
            let shown = UpdateLayeredWindow(
                self.hwnd,
                Some(screen_dc),
                Some(&destination),
                Some(&size),
                Some(memory_dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
            .is_ok();
            SelectObject(memory_dc, previous);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(memory_dc);
            ReleaseDC(None, screen_dc);
            let _ = SetWindowPos(
                self.hwnd,
                Some(desktop),
                screen.left,
                screen.top,
                screen_width as i32,
                screen_height as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            shown
        }
    }
}

thread_local! {
    /// The layer, made with the first picture.
    static INSTANCE: std::cell::RefCell<Option<Layer>> = const { std::cell::RefCell::new(None) };
    /// A picture that came before its desktop window was placed.
    static PENDING: std::cell::RefCell<Option<(uuid::Uuid, std::sync::Arc<gpui::RenderImage>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Show the desktop's decoded wallpaper for display `output` (a BGRA image
/// of the screen's physical size) in the layer under its desktop window,
/// now or once that window is placed. Called on the UI thread. False when
/// Windows cannot make the layer: the desktop view then draws the picture
/// itself.
pub fn show_picture(output: uuid::Uuid, image: std::sync::Arc<gpui::RenderImage>) -> bool {
    let made = INSTANCE.with(|instance| {
        let mut instance = instance.borrow_mut();
        if instance.is_none() {
            *instance = Layer::new();
        }
        instance.is_some()
    });
    if !made {
        return false;
    }
    PENDING.with(|pending| *pending.borrow_mut() = Some((output, image)));
    flush();
    true
}

/// Show a pending picture if its desktop window is placed (`super::place`
/// calls this after placing a background surface).
pub(crate) fn flush() {
    let Some((output, image)) = PENDING.with(|pending| pending.borrow().clone()) else {
        return;
    };
    let Some((desktop, screen)) = super::background_window(output) else {
        return;
    };
    let size = image.size(0);
    let (width, height) = (size.width.0.max(0) as u32, size.height.0.max(0) as u32);
    let Some(bytes) = image.as_bytes(0) else {
        return;
    };
    let shown = INSTANCE.with(|instance| {
        instance.borrow().as_ref().is_some_and(|layer| {
            layer.show(desktop, screen, 0, 0, width, height, bytes, [0, 0, 0])
        })
    });
    if shown {
        // Windows keeps its own copy: the pixels go now.
        PENDING.with(|pending| *pending.borrow_mut() = None);
    }
}
