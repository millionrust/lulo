//! A frosted backdrop behind the menu bar and the Dock, as the Mac's
//! materials: Windows blurs whatever is behind the window (an acrylic
//! accent, `SetWindowCompositionAttribute`), and the surface draws its
//! light or dark tint over that with the `mac` tokens, so it follows the
//! appearance like everything else Lulo draws.
//!
//! Not `DWMWA_SYSTEMBACKDROP_TYPE`: Windows 11's system backdrops turn into
//! a flat colour while their window is inactive, and the bar and the Dock
//! never take the foreground. The accent blur stays on whatever window is
//! in front, on Windows 10 and 11 alike, which is how the taskbar looks.
//! The Dock's window is cut to its rounded shelf with a window region, so
//! the blur has the shelf's shape on both.
//!
//! When the user has turned transparency effects off (Settings ▸
//! Personalization ▸ Colors), there is no blur and the surfaces draw
//! their tint nearly opaque, as Windows' own surfaces do then.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{s, w, BOOL};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

use super::{registry, trace};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    Bar,
    Dock,
}

static BAR_FROSTED: AtomicBool = AtomicBool::new(false);
static DOCK_FROSTED: AtomicBool = AtomicBool::new(false);

/// Whether Windows blurs what is behind `surface`, so its tint can be
/// light.
pub fn frosted(surface: Surface) -> bool {
    match surface {
        Surface::Bar => BAR_FROSTED.load(Ordering::Acquire),
        Surface::Dock => DOCK_FROSTED.load(Ordering::Acquire),
    }
}

/// Whether the user has Windows' transparency effects on.
fn transparency_effects() -> bool {
    registry::get_user_dword(
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "EnableTransparency",
    )
    .is_none_or(|value| value != 0)
}

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut core::ffi::c_void,
    size: usize,
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

fn set_accent(hwnd: HWND, state: u32, gradient: u32) -> bool {
    type SetWindowCompositionAttribute =
        unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;
    // SAFETY: user32 is loaded in every GUI process; the export has had
    // this signature since Windows 7, and the structures are laid out as
    // it reads them. Both live across the call.
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else {
            return false;
        };
        let Some(function) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else {
            return false;
        };
        let function = std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            SetWindowCompositionAttribute,
        >(function);
        let mut accent = AccentPolicy {
            state,
            flags: 0,
            gradient,
            animation: 0,
        };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut accent as *mut AccentPolicy as *mut core::ffi::c_void,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        function(hwnd, &mut data).as_bool()
    }
}

/// Give `hwnd` its backdrop. Returns whether it is frosted.
pub fn apply(hwnd: HWND, surface: Surface) -> bool {
    // Windows 11 would round a window's corners on its own; the bar is a
    // straight strip and the Dock is cut to its own shape. Windows 10
    // refuses the attribute and rounds nothing anyway.
    let corner = DWMWCP_DONOTROUND.0 as u32;
    // SAFETY: a DWORD-sized attribute of a window this process owns.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const u32 as *const core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    // The tint's alpha must not be 0 (an acrylic accent with none draws
    // black); the surface paints the real tint itself.
    let frosted =
        transparency_effects() && set_accent(hwnd, ACCENT_ENABLE_ACRYLICBLURBEHIND, 0x0100_0000);
    match surface {
        Surface::Bar => BAR_FROSTED.store(frosted, Ordering::Release),
        Surface::Dock => DOCK_FROSTED.store(frosted, Ordering::Release),
    }
    trace(|| {
        format!(
            "backdrop {surface:?}: {}",
            if frosted { "acrylic" } else { "tint only" }
        )
    });
    frosted
}

/// Cut the Dock's window to its rounded shelf, `width` by `height`
/// physical pixels with corners of `radius`.
pub fn round(hwnd: HWND, width: i32, height: i32, radius: i32) {
    // SAFETY: the region is handed to Windows, which owns it from then on.
    unsafe {
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, 2 * radius, 2 * radius);
        if !region.is_invalid() {
            SetWindowRgn(hwnd, Some(region), true);
        }
    }
}
