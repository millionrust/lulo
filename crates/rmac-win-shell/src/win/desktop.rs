//! Explorer's desktop while Lulo mode runs (ADR 0023 "Lulo mode").
//!
//! Lulo draws its own desktop (`ui::desktop`): Lulo's wallpaper and the
//! user's Desktop folder as Lulo icons, in a window kept just above
//! Explorer's desktop and below every app window. Explorer's own desktop
//! icons would otherwise sit under it, still reachable from the keyboard
//! (Win+D then typing selects them), so their list view (`SHELLDLL_DefView`
//! ▸ `SysListView32`, in `Progman` or a `WorkerW`) is hidden while Lulo
//! runs. Nothing Explorer saves changes: the window is only hidden, and the
//! user's wallpaper and icon settings are never touched.
//!
//! Before the list view is hidden that is recorded under
//! `HKCU\Software\Lulo\Shell`, and [`restore`] shows it again on every way
//! out, as the taskbar's setting is restored (`lulo-shell`'s quit and
//! session-end paths, and `lulo-session` after a crash or at its next
//! start). Explorer restarting makes a new list view, which is hidden
//! again (`TaskbarCreated`).
//!
//! Earlier builds moved the list view down below the bar instead
//! (WIN-OS-33); a record of such a move is still undone here.

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetClientRect, GetShellWindow,
    GetWindow, GetWindowThreadProcessId, IsWindowVisible, SetWindowPos, ShowWindowAsync,
    GW_HWNDPREV, HWND_BOTTOM, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SW_HIDE, SW_SHOWNA,
};

use super::{registry, trace};

/// Recorded while Lulo has Explorer's desktop icons hidden.
const HIDDEN_RECORD: &str = "DesktopIconsHidden";
/// Recorded by earlier builds while the icons were moved below the bar.
const OFFSET_RECORD: &str = "DesktopIconsOffset";

/// Explorer's desktop view (`SHELLDLL_DefView`) and the icons' list view
/// in it. The view lives in `Progman`, or in a `WorkerW` once a wallpaper
/// slideshow or animated wallpaper has run.
fn folder_view() -> Option<(HWND, HWND)> {
    let in_parent = |parent: HWND| -> Option<(HWND, HWND)> {
        // SAFETY: looks windows up by class name.
        unsafe {
            let view = FindWindowExW(Some(parent), None, w!("SHELLDLL_DefView"), PCWSTR::null())
                .ok()
                .filter(|view| !view.is_invalid())?;
            let list = FindWindowExW(Some(view), None, w!("SysListView32"), PCWSTR::null())
                .ok()
                .filter(|list| !list.is_invalid())?;
            Some((view, list))
        }
    };
    // SAFETY: as above.
    if let Ok(progman) = unsafe { FindWindowW(w!("Progman"), PCWSTR::null()) } {
        if let Some(found) = in_parent(progman) {
            return Some(found);
        }
    }
    let mut after: Option<HWND> = None;
    loop {
        // SAFETY: as above, continuing after the last one found.
        let worker = unsafe { FindWindowExW(None, after, w!("WorkerW"), PCWSTR::null()) }
            .ok()
            .filter(|worker| !worker.is_invalid())?;
        if let Some(found) = in_parent(worker) {
            return Some(found);
        }
        after = Some(worker);
    }
}

/// Explorer's desktop icons' list view, for the CI checks.
pub fn icon_list_view() -> Option<HWND> {
    folder_view().map(|(_, list)| list)
}

/// Hide Explorer's desktop icons while Lulo's desktop shows the user's.
pub fn hide_icons() {
    let Some((_, list)) = folder_view() else {
        trace(|| "explorer desktop icons: no list view".into());
        return;
    };
    // SAFETY: reads the visibility of Explorer's window.
    if !unsafe { IsWindowVisible(list) }.as_bool() && registry::get_dword(HIDDEN_RECORD).is_none() {
        // The user (or Windows) has them hidden already: nothing to undo.
        trace(|| "explorer desktop icons: already hidden by the user".into());
        return;
    }
    if registry::get_dword(HIDDEN_RECORD).is_none() && !registry::set_dword(HIDDEN_RECORD, 1) {
        // Without a record there would be no way back after a crash.
        return;
    }
    // SAFETY: hides Explorer's window until Lulo shows it again, without
    // waiting on Explorer (which may be busy).
    let _ = unsafe { ShowWindowAsync(list, SW_HIDE) };
    trace(|| "explorer desktop icons hidden".into());
}

/// Give Explorer's desktop icons back. True when Lulo had hidden or moved
/// them.
pub fn restore() -> bool {
    let hidden = registry::get_dword(HIDDEN_RECORD).is_some();
    let moved = registry::get_dword(OFFSET_RECORD).is_some();
    if !hidden && !moved {
        return false;
    }
    if let Some((view, list)) = folder_view() {
        if moved {
            let mut client = RECT::default();
            // SAFETY: a plain out-parameter.
            if unsafe { GetClientRect(view, &mut client) }.is_ok() {
                // SAFETY: moves Explorer's window back to fill its view,
                // without waiting on Explorer.
                let _ = unsafe {
                    SetWindowPos(
                        list,
                        None,
                        client.left,
                        client.top,
                        client.right - client.left,
                        client.bottom - client.top,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                    )
                };
            }
        }
        if hidden {
            // SAFETY: shows Explorer's own window again, without
            // activating it or waiting on Explorer.
            let _ = unsafe { ShowWindowAsync(list, SW_SHOWNA) };
        }
    }
    registry::delete_value(HIDDEN_RECORD);
    registry::delete_value(OFFSET_RECORD);
    trace(|| "explorer desktop icons restored".into());
    true
}

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

/// Put Lulo's desktop window `ours` directly above Explorer's desktop
/// layer, below every app window. True when it moved.
pub fn place_above_desktop_layer(ours: HWND) -> bool {
    let after = match top_desktop_layer_window() {
        // SAFETY: reads the window just above Explorer's desktop.
        Some(layer) => match unsafe { GetWindow(layer, GW_HWNDPREV) } {
            Ok(previous) if previous == ours => return false,
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
    true
}

thread_local! {
    static PLACING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// While alive, the desktop window's own z-order changes go through (see
/// [`own_placement`]).
struct OwnPlacement;

impl OwnPlacement {
    fn begin() -> Self {
        PLACING.with(|placing| placing.set(true));
        Self
    }
}

impl Drop for OwnPlacement {
    fn drop(&mut self) {
        PLACING.with(|placing| placing.set(false));
    }
}

/// Whether the desktop window is being placed by [`place_above_desktop_layer`]
/// on this thread; any other change of its z-order (a click activating it,
/// Windows bringing it forward) is refused, so it stays below the apps.
pub fn own_placement() -> bool {
    PLACING.with(|placing| placing.get())
}

#[cfg(test)]
mod tests {
    #[test]
    fn records_have_their_own_names() {
        assert_ne!(super::HIDDEN_RECORD, super::OFFSET_RECORD);
    }
}
