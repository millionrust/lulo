//! A click on a hot corner's one-pixel surface goes on to the shell
//! surface under it.
//!
//! On the Mac the pointer in the top-left corner both fires the hot corner
//! and, when clicked, opens the Apple menu under it. On Windows the corner
//! is a 1 × 1 window that takes the click, so the Lulo menu at (0, 0) did
//! not open. The corner hands its click to the first visible window of
//! this process below it that holds the pointer (the bar, the Dock), as
//! the button press and release Windows would have sent it. Other
//! processes' windows are never sent anything.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    PostMessageW, GW_HWNDNEXT, WM_LBUTTONDOWN, WM_LBUTTONUP,
};

/// The client-coordinate `lParam` of a mouse message.
pub(crate) fn point_lparam(point: POINT) -> LPARAM {
    let x = point.x as i16 as u16 as u32;
    let y = point.y as i16 as u16 as u32;
    LPARAM(((y << 16) | x) as isize)
}

/// Pass the left click `window` (a hot corner) just took to the shell
/// surface under it.
pub fn click_through(window: &gpui::Window) {
    let Some(own) = super::surface::hwnd(window) else {
        return;
    };
    let mut cursor = POINT::default();
    // SAFETY: reads the pointer's position into a POINT.
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return;
    }
    // SAFETY: no arguments.
    let process = unsafe { GetCurrentProcessId() };
    let mut next = own;
    loop {
        // SAFETY: walks the z-order below a window this process owns.
        next = match unsafe { GetWindow(next, GW_HWNDNEXT) } {
            Ok(hwnd) if !hwnd.is_invalid() => hwnd,
            _ => return,
        };
        let mut owner = 0u32;
        let mut rect = RECT::default();
        // SAFETY: reads properties of a window the walk named.
        let hit = unsafe {
            GetWindowThreadProcessId(next, Some(&mut owner as *mut u32));
            IsWindowVisible(next).as_bool()
                && GetWindowRect(next, &mut rect).is_ok()
                && cursor.x >= rect.left
                && cursor.x < rect.right
                && cursor.y >= rect.top
                && cursor.y < rect.bottom
        };
        if !hit {
            continue;
        }
        if owner != process {
            // Another app's window: it gets nothing from the shell.
            return;
        }
        post_click(next, cursor);
        return;
    }
}

fn post_click(target: HWND, screen: POINT) {
    let mut client = screen;
    // SAFETY: maps a point into a window this process owns, then posts it
    // the press and release of a left click there.
    unsafe {
        if !ScreenToClient(target, &mut client).as_bool() {
            return;
        }
        let at = point_lparam(client);
        let _ = PostMessageW(
            Some(target),
            WM_LBUTTONDOWN,
            // MK_LBUTTON: the left button is down.
            WPARAM(0x0001),
            at,
        );
        let _ = PostMessageW(Some(target), WM_LBUTTONUP, WPARAM(0), at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_coordinates_pack_as_windows_expects() {
        assert_eq!(point_lparam(POINT { x: 3, y: 2 }).0, 0x0002_0003);
        // Negative coordinates keep their 16-bit two's complement.
        assert_eq!(point_lparam(POINT { x: -1, y: 0 }).0, 0x0000_FFFF);
    }
}
