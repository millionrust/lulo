//! The blurred material under the Dock's shelf and open menus on Windows.
//!
//! On Lulo OS these are small layer surfaces that ask niri to blur what is
//! behind them (`WindowBackgroundAppearance::Blurred`), shaped by the
//! client, with the view's tint drawn over the blur. GPUI's Windows backend
//! answers `Blurred` with acrylic, whose grey luminosity layer and noise
//! differ from niri's plain blur and which ignores the window's region, so
//! a dark box showed behind the rounded ends (WIN-OS-49). Here a blurred
//! surface gets DWM's plain blur instead, cut to the view's rounded shape
//! ([`set_corner_radius`]), which is what niri draws. With Windows'
//! transparency effects off there is no blur at all and the view's tint
//! shows alone, as Windows' own surfaces do then.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::core::{s, w, BOOL, HSTRING};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

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
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_BLURBEHIND: u32 = 3;

/// Whether the user has Windows' transparency effects on.
pub fn transparency_effects() -> bool {
    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: `value` and `size` are valid out-parameters for a DWORD.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            &HSTRING::from("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    status != ERROR_SUCCESS || value != 0
}

fn set_accent(hwnd: HWND, state: u32) -> bool {
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
            gradient: 0,
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

/// Give a blurred surface its material: DWM's plain blur, or none with
/// transparency effects off.
pub fn apply(hwnd: HWND) {
    let state = if transparency_effects() {
        ACCENT_ENABLE_BLURBEHIND
    } else {
        ACCENT_DISABLED
    };
    set_accent(hwnd, state);
}

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
