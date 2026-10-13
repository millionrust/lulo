//! The background layer on Windows: Lulo's desktop window sits directly
//! above Explorer's desktop (`Progman`, or a top-level `WorkerW` of the
//! shell's process) and below every app window, as the wallpaper layer does
//! on Lulo OS (ADR 0023 "Lulo mode").

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetShellWindow, GetWindow, GetWindowThreadProcessId, SetWindowPos,
    GW_HWNDPREV, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    // SAFETY: the buffer is writable and its length is passed.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

/// Whether `hwnd` is part of Explorer's desktop layer: `Progman`, or a
/// top-level `WorkerW` of the shell's process (the wallpaper and icon
/// layers Explorer keeps at the bottom of the z-order).
pub fn is_desktop_layer(hwnd: HWND) -> bool {
    let class = class_name(hwnd);
    if class == "Progman" {
        return true;
    }
    if class != "WorkerW" {
        return false;
    }
    let mut shell_pid = 0u32;
    let mut pid = 0u32;
    // SAFETY: plain out-parameters for windows that exist.
    unsafe {
        GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell_pid));
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    pid != 0 && pid == shell_pid
}

/// The highest window of Explorer's desktop layer in the z-order.
fn top_desktop_layer_window() -> Option<HWND> {
    let mut found: Option<HWND> = None;
    unsafe extern "system" fn first(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `Option` passed below, alive for the call.
        let found = unsafe { &mut *(lparam.0 as *mut Option<HWND>) };
        if is_desktop_layer(hwnd) {
            *found = Some(hwnd);
            return BOOL(0);
        }
        BOOL(1)
    }
    // SAFETY: the callback only writes into `found`. EnumWindows lists
    // top-level windows from the top of the z-order down.
    let _ = unsafe {
        EnumWindows(
            Some(first),
            LPARAM(&mut found as *mut Option<HWND> as isize),
        )
    };
    found
}

/// Put the background window `ours` directly above Explorer's desktop
/// layer, below every app window. True when it moved.
pub fn place_above_desktop_layer(ours: HWND) -> bool {
    let after = match top_desktop_layer_window() {
        // SAFETY: reads the window just above Explorer's desktop.
        Some(layer) => match unsafe { GetWindow(layer, GW_HWNDPREV) } {
            // Already right above the desktop layer, or above the wallpaper
            // layer that is.
            Ok(previous) if previous == ours => return false,
            Ok(previous) if super::wallpaper_layer::is_layer(previous) => {
                // SAFETY: reads the window above the wallpaper layer.
                match unsafe { GetWindow(previous, GW_HWNDPREV) } {
                    Ok(above) if above == ours => return false,
                    Ok(above) if !above.is_invalid() => above,
                    _ => HWND::default(),
                }
            }
            Ok(previous) if !previous.is_invalid() => previous,
            // Nothing above the desktop layer: ours goes on top of the
            // non-topmost band, which is right above it.
            _ => HWND::default(),
        },
        None => HWND_BOTTOM,
    };
    let _guard = OwnPlacement::begin();
    // SAFETY: positions a window this process owns.
    let _ = unsafe {
        SetWindowPos(
            ours,
            Some(after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
    // Lulo's wallpaper layer goes with it, just below.
    super::wallpaper_layer::keep_below(ours);
    true
}

thread_local! {
    static PLACING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// While alive, a background window's own z-order changes go through (see
/// [`own_placement`]).
pub(crate) struct OwnPlacement;

impl OwnPlacement {
    pub(crate) fn begin() -> Self {
        PLACING.with(|placing| placing.set(true));
        Self
    }
}

impl Drop for OwnPlacement {
    fn drop(&mut self) {
        PLACING.with(|placing| placing.set(false));
    }
}

/// Whether a background window is being placed by the shell on this
/// thread; any other change of its z-order (a click activating it, Windows
/// bringing it forward) is refused, so it stays below the apps.
pub fn own_placement() -> bool {
    PLACING.with(|placing| placing.get())
}
