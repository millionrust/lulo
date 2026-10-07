//! Turning GPUI windows into shell surfaces: always on top, out of
//! Alt+Tab and the taskbar, placed in physical pixels, and (for the bar and
//! the Dock) never taking the keyboard from the app in front.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, GetWindowLongPtrW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, GWLP_WNDPROC, GWL_EXSTYLE, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, WM_MOUSEACTIVATE, WM_NCDESTROY, WNDPROC,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};

/// The Win32 window behind a GPUI window.
pub fn hwnd(window: &gpui::Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// Keep `hwnd` above app windows and out of Alt+Tab and the taskbar. A
/// surface that is not `activatable` never takes the foreground, so a
/// click on the bar or the Dock leaves the app in front with the keyboard.
pub fn make_shell_surface(hwnd: HWND, activatable: bool) {
    // SAFETY: style bits on a window this process owns.
    unsafe {
        let mut style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        style |= (WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize;
        style &= !(WS_EX_APPWINDOW.0 as isize);
        if !activatable {
            style |= WS_EX_NOACTIVATE.0 as isize;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
    }
    if !activatable {
        subclass(
            hwnd,
            Box::new(|_, message, _, _| {
                (message == WM_MOUSEACTIVATE).then_some(LRESULT(MA_NOACTIVATE as isize))
            }),
        );
    }
}

/// Show `hwnd` at `rect` (physical pixels), on top, without activating it.
pub fn show_at(hwnd: HWND, rect: RECT) {
    // SAFETY: positions a window this process owns.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    };
}

/// Show `hwnd` at `rect` and give it the keyboard (Spotlight, an open
/// menu). The user's click or hotkey just reached this process, so Windows
/// lets it take the foreground.
pub fn show_focused_at(hwnd: HWND, rect: RECT) {
    show_at(hwnd, rect);
    set_cloaked(hwnd, false);
    // SAFETY: as above.
    let _ = unsafe { SetForegroundWindow(hwnd) };
}

fn set_cloaked(hwnd: HWND, cloaked: bool) {
    let value = BOOL::from(cloaked);
    // SAFETY: a BOOL-sized attribute of a window this process owns.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &value as *const BOOL as *const core::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        )
    };
}

/// Put away a panel (Spotlight, a menu) until it is shown again. It is
/// cloaked and moved off screen rather than hidden: a hidden window gets no
/// `WM_PAINT`, so a frame it asked for would keep GPUI's vsync loop
/// running for ever, while a cloaked one still paints, parks and idles.
pub fn hide(hwnd: HWND) {
    set_cloaked(hwnd, true);
    // SAFETY: as above.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            -32000,
            -32000,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
        )
    };
}

/// Physical pixels per GPUI pixel for `hwnd`.
pub fn scale_factor(hwnd: HWND) -> f32 {
    // SAFETY: reads a property of a live window.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

/// The primary monitor's whole area and its work area, in physical pixels.
pub fn primary_monitor() -> (RECT, RECT) {
    // SAFETY: plain out-parameter.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        (info.rcMonitor, info.rcWork)
    }
}

/// A message hook: `Some` answers the message, `None` passes it on to
/// GPUI's own window procedure.
pub type Hook = Box<dyn Fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>>;

struct Subclass {
    previous: isize,
    hooks: Vec<Rc<Hook>>,
}

thread_local! {
    static SUBCLASSES: RefCell<HashMap<isize, Subclass>> = RefCell::new(HashMap::new());
}

/// Run `hook` before GPUI's window procedure for every message `hwnd`
/// gets: AppBar notifications, Explorer restarts. Hooks run on the UI
/// thread inside the message, so they must not re-enter GPUI; they hand
/// work on through a channel.
pub fn subclass(hwnd: HWND, hook: Hook) {
    let key = hwnd.0 as isize;
    let added = SUBCLASSES.with(|subclasses| {
        let mut subclasses = subclasses.borrow_mut();
        match subclasses.get_mut(&key) {
            Some(subclass) => {
                subclass.hooks.push(Rc::new(hook));
                None
            }
            None => Some(hook),
        }
    });
    let Some(hook) = added else {
        return;
    };
    // SAFETY: replaces this process's own window procedure with one that
    // forwards to the previous one.
    let previous =
        unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, subclass_proc as *const () as isize) };
    SUBCLASSES.with(|subclasses| {
        subclasses.borrow_mut().insert(
            key,
            Subclass {
                previous,
                hooks: vec![Rc::new(hook)],
            },
        )
    });
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let key = hwnd.0 as isize;
    let entry = SUBCLASSES.with(|subclasses| {
        subclasses
            .borrow()
            .get(&key)
            .map(|subclass| (subclass.previous, subclass.hooks.clone()))
    });
    let Some((previous, hooks)) = entry else {
        // SAFETY: the default procedure for a window we no longer track.
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    };
    for hook in &hooks {
        if let Some(result) = hook(hwnd, message, wparam, lparam) {
            return result;
        }
    }
    // SAFETY: `previous` is the window procedure `SetWindowLongPtrW`
    // returned for this window, a valid `WNDPROC`.
    let previous: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
    // SAFETY: forwards the message unchanged.
    let result = unsafe { CallWindowProcW(previous, hwnd, message, wparam, lparam) };
    if message == WM_NCDESTROY {
        SUBCLASSES.with(|subclasses| subclasses.borrow_mut().remove(&key));
    }
    result
}
