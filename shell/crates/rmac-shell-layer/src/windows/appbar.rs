//! Exclusive zones as AppBars (`SHAppBarMessage`): Windows takes the
//! menu bar's and the Dock's strips out of the work area, so maximised
//! windows stop below the bar and above the Dock, as on the Mac and on Lulo
//! OS.

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE,
    ABM_SETPOS, ABM_WINDOWPOSCHANGED, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// The message Windows sends an AppBar about the others (`ABN_*`).
pub const CALLBACK_MESSAGE: u32 = WM_APP + 0x4C;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    fn code(self) -> u32 {
        match self {
            Edge::Top => ABE_TOP,
            Edge::Bottom => ABE_BOTTOM,
            Edge::Left => ABE_LEFT,
            Edge::Right => ABE_RIGHT,
        }
    }

    fn fit(self, rc: &mut RECT, thickness: i32) {
        match self {
            Edge::Top => rc.bottom = rc.top + thickness,
            Edge::Bottom => rc.top = rc.bottom - thickness,
            Edge::Left => rc.right = rc.left + thickness,
            Edge::Right => rc.left = rc.right - thickness,
        }
    }
}

fn data(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        uCallbackMessage: CALLBACK_MESSAGE,
        uEdge: 0,
        rc: RECT::default(),
        lParam: LPARAM(0),
    }
}

/// Make `hwnd` an AppBar. False when Windows refused (it is one already).
pub fn register(hwnd: HWND) -> bool {
    let mut data = data(hwnd);
    // SAFETY: `data` is a valid, sized APPBARDATA for the call.
    unsafe { SHAppBarMessage(ABM_NEW, &mut data) != 0 }
}

/// Reserve a strip `thickness` physical pixels deep along `edge` of
/// `monitor`, next to any other AppBars there, and return it. When that is
/// the strip already held (`current`), nothing is sent: setting a position
/// notifies every other AppBar, which must not echo back and forth.
pub fn reserve(hwnd: HWND, edge: Edge, thickness: i32, monitor: RECT, current: RECT) -> RECT {
    let mut data = data(hwnd);
    data.uEdge = edge.code();
    data.rc = monitor;
    edge.fit(&mut data.rc, thickness);
    // SAFETY: `data` is a valid, sized APPBARDATA. Windows moves the
    // proposed rectangle off any AppBar already on that edge (the
    // taskbar), then the depth is fixed again.
    unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut data) };
    edge.fit(&mut data.rc, thickness);
    if data.rc == current {
        return current;
    }
    // SAFETY: as above.
    unsafe { SHAppBarMessage(ABM_SETPOS, &mut data) };
    data.rc
}

/// Tell Windows the AppBar moved or changed, as the documentation asks
/// after every `WM_WINDOWPOSCHANGED`.
pub fn moved(hwnd: HWND) {
    let mut data = data(hwnd);
    // SAFETY: as above.
    unsafe { SHAppBarMessage(ABM_WINDOWPOSCHANGED, &mut data) };
}

/// Give the strip back to the work area.
pub fn remove(hwnd: HWND) {
    let mut data = data(hwnd);
    // SAFETY: as above; removing an unknown AppBar does nothing.
    unsafe { SHAppBarMessage(ABM_REMOVE, &mut data) };
}
