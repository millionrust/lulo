//! Turning GPUI windows into shell surfaces: always on top, out of
//! Alt+Tab and the taskbar, placed in physical pixels, and (for the bar and
//! the Dock) never taking the keyboard from the app in front.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_CLOAK, DWMWA_COLOR_NONE,
    DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallWindowProcW, DefWindowProcW, GetForegroundWindow, GetWindowLongPtrW,
    GetWindowThreadProcessId, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, GWLP_WNDPROC,
    GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST, MA_NOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, WINDOWPOS, WM_MOUSEACTIVATE,
    WM_NCDESTROY, WM_WINDOWPOSCHANGING, WNDPROC, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

/// The Win32 window behind a GPUI window.
pub fn hwnd(window: &gpui::Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

fn set_dword_attribute(hwnd: HWND, attribute: DWMWINDOWATTRIBUTE, value: u32) {
    // SAFETY: a DWORD-sized attribute of a window this process owns;
    // Windows 10 refuses the Windows 11 ones, which changes nothing.
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attribute,
            &value as *const u32 as *const core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
}

/// No DWM frame at all: shell surfaces draw their own shapes, edges and
/// shadows, so Windows 11's rounded corners, 1 px border and window shadow
/// would outline their clear margins (Spotlight once showed a faint box
/// round its whole window).
pub fn plain(hwnd: HWND) {
    set_dword_attribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_DONOTROUND.0 as u32,
    );
    set_dword_attribute(hwnd, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE);
}

/// [`plain`], and no DWM window shadow either (the panels and the desktop
/// window; the bar and the Dock keep their blur set-up untouched).
pub fn plain_without_shadow(hwnd: HWND) {
    plain(hwnd);
    set_dword_attribute(hwnd, DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED.0 as u32);
}

/// Make `hwnd` a plain popup with no frame at all, so its client area is
/// the whole window: GPUI's pop-up windows are overlapped windows, which
/// keep a resize border the surface never paints (it showed as a black
/// rim round the bar and the Dock, and Explorer's desktop at the edges of
/// Lulo's).
pub fn make_borderless(hwnd: HWND) {
    // SAFETY: style bits on a window this process owns; the frame change is
    // applied at once.
    unsafe {
        let kept = GetWindowLongPtrW(hwnd, GWL_STYLE)
            & (WS_VISIBLE.0 | WS_CLIPCHILDREN.0 | WS_CLIPSIBLINGS.0) as isize;
        SetWindowLongPtrW(hwnd, GWL_STYLE, kept | WS_POPUP.0 as isize);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Lulo mode's desktop window: out of Alt+Tab and the taskbar like the
/// other surfaces, but not topmost, and kept below every app window: any
/// change of its place in the z-order it did not ask for itself (a click
/// activating it, Windows bringing it forward) is dropped.
pub fn make_desktop_surface(hwnd: HWND) {
    // SAFETY: style bits on a window this process owns.
    unsafe {
        let mut style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        style |= WS_EX_TOOLWINDOW.0 as isize;
        style &= !((WS_EX_APPWINDOW.0 | WS_EX_TOPMOST.0) as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
    }
    make_borderless(hwnd);
    plain_without_shadow(hwnd);
    subclass(
        hwnd,
        Box::new(|_, message, _, lparam| {
            if message == WM_WINDOWPOSCHANGING && lparam.0 != 0 && !super::desktop_layer::own_placement()
            {
                // SAFETY: for WM_WINDOWPOSCHANGING, `lparam` points at the
                // WINDOWPOS Windows is about to apply; changing its flags
                // is how a window declines part of the change.
                let position = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
                position.flags |= SWP_NOZORDER;
            }
            None
        }),
    );
}

/// Move and size `hwnd` (physical pixels) without changing its z-order or
/// activating it (Spotlight growing with its results).
pub fn set_bounds(hwnd: HWND, rect: RECT) {
    // SAFETY: positions a window this process owns.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    };
}

/// Show `hwnd` at `rect` without activating it or changing its place in
/// the z-order (the desktop window).
pub fn show_in_place(hwnd: HWND, rect: RECT) {
    // SAFETY: positions a window this process owns.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
        )
    };
}

/// Keep `hwnd` above app windows and out of Alt+Tab and the taskbar. A
/// surface that is not `activatable` never takes the foreground, so a
/// click on the bar or the Dock leaves the app in front with the keyboard.
pub fn make_shell_surface(hwnd: HWND, activatable: bool) {
    make_borderless(hwnd);
    if activatable {
        plain_without_shadow(hwnd);
    } else {
        plain(hwnd);
    }
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
    take_foreground(hwnd);
}

/// Give `hwnd` the keyboard: the user's click or key just reached this
/// process, so Windows lets it take the foreground.
pub fn take_foreground(hwnd: HWND) {
    // SAFETY: as above.
    let _ = unsafe { SetForegroundWindow(hwnd) };
    // SAFETY: reads the foreground window.
    if unsafe { GetForegroundWindow() } == hwnd {
        return;
    }
    // A key the low-level hook took (Win+Space) does not count as input
    // this process received, so Windows may refuse the foreground. Joining
    // the front app's input queue for the moment of the switch lets it
    // through, as the user asked for this panel.
    // SAFETY: attaches this thread's input to the foreground window's
    // thread only for the calls below, and detaches it again.
    unsafe {
        let front = GetForegroundWindow();
        let front_thread = GetWindowThreadProcessId(front, None);
        let own_thread = GetCurrentThreadId();
        let attached = front_thread != 0
            && front_thread != own_thread
            && AttachThreadInput(own_thread, front_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        if attached {
            let _ = AttachThreadInput(own_thread, front_thread, false);
        }
    }
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
