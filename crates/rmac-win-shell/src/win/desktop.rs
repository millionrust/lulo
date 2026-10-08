//! The desktop's icons while the menu bar holds the top of the screen.
//!
//! Explorer lays desktop icons out in its `SysListView32` ("FolderView"),
//! which fills the desktop window. Taking the top strip out of the work
//! area does not move icons the user placed by hand, so the top row would
//! sit under the bar. While Lulo runs, the list view itself is moved down
//! by the bar's depth (and made as much shorter), which moves every icon
//! with it and changes nothing Explorer saves: icon positions are kept
//! relative to the list view. With "Auto arrange icons" on, Explorer flows
//! icons into the work area itself; Lulo only asks it to arrange once.
//!
//! Before the list view is moved the offset is recorded under
//! `HKCU\Software\Lulo\Shell`, and [`restore`] gives the list view its
//! whole desktop window back on every way out, as the taskbar's setting
//! is restored (`lulo-shell`'s quit and session-end paths, and
//! `lulo-session` after a crash). Explorer restarting makes a new list
//! view, which starts in its usual place.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::MapWindowPoints;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetClientRect, GetWindowLongPtrW, SendMessageTimeoutW,
    SetWindowPos, GWL_STYLE, SMTO_ABORTIFHUNG, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER,
};

use super::{registry, trace};

const RECORD: &str = "DesktopIconsOffset";
const LVS_AUTOARRANGE: u32 = 0x0100;
const LVM_ARRANGE: u32 = 0x1000 + 22;
const LVA_DEFAULT: usize = 0;

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

/// Where the list view goes: the desktop view's client area with its top
/// `offset` pixels left free.
fn placed(client: RECT, offset: i32) -> RECT {
    RECT {
        top: client.top + offset,
        ..client
    }
}

fn move_list(list: HWND, rect: RECT) {
    // SAFETY: moves Explorer's window without waiting on Explorer, which
    // may be busy (the move happens on its own thread).
    let _ = unsafe {
        SetWindowPos(
            list,
            None,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    };
}

/// Keep the desktop's icons clear of the top `bar_bottom` physical pixels
/// of the screen.
pub fn make_room(bar_bottom: i32) {
    let Some((view, list)) = folder_view() else {
        trace(|| "desktop icons: no desktop list view".into());
        return;
    };
    // SAFETY: reads the list view's style bits.
    let style = unsafe { GetWindowLongPtrW(list, GWL_STYLE) } as u32;
    if style & LVS_AUTOARRANGE != 0 {
        // Explorer keeps auto-arranged icons in the work area; ask it to
        // flow them now that the work area starts below the bar.
        // SAFETY: a message without pointers, with a time limit.
        let _ = unsafe {
            SendMessageTimeoutW(
                list,
                LVM_ARRANGE,
                WPARAM(LVA_DEFAULT),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                500,
                None,
            )
        };
        trace(|| "desktop icons: auto-arranged by Explorer".into());
        return;
    }
    let mut client = RECT::default();
    let mut top = [POINT {
        x: 0,
        y: bar_bottom,
    }];
    // SAFETY: plain out-parameters for windows that exist.
    unsafe {
        if GetClientRect(view, &mut client).is_err() {
            return;
        }
        MapWindowPoints(None, Some(view), &mut top);
    }
    let offset = top[0].y - client.top;
    if offset <= 0 {
        return;
    }
    if registry::get_dword(RECORD).is_none() && !registry::set_dword(RECORD, offset as u32) {
        // Without a record there would be no way back after a crash.
        return;
    }
    let rect = placed(client, offset);
    move_list(list, rect);
    trace(|| {
        format!(
            "desktop icons below the bar: list view at {},{},{},{}",
            rect.left, rect.top, rect.right, rect.bottom
        )
    });
}

/// Give the list view its whole desktop window back. True when Lulo had
/// moved it.
pub fn restore() -> bool {
    if registry::get_dword(RECORD).is_none() {
        return false;
    }
    if let Some((view, list)) = folder_view() {
        let mut client = RECT::default();
        // SAFETY: a plain out-parameter.
        if unsafe { GetClientRect(view, &mut client) }.is_ok() {
            move_list(list, placed(client, 0));
        }
    }
    registry::delete_value(RECORD);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_view_keeps_its_bottom_and_loses_the_bar_strip() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 1366,
            bottom: 768,
        };
        let moved = placed(client, 24);
        assert_eq!((moved.top, moved.bottom), (24, 768));
        assert_eq!(placed(client, 0), client);
    }
}
