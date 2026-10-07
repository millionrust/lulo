//! The Windows taskbar while the Lulo layer runs (ADR 0023 decision 1).
//!
//! The Dock takes the taskbar's place, so while Lulo runs the taskbar is
//! set to auto-hide (which gives its strip back to the work area) and
//! hidden (so it does not slide up over the Dock). Before changing
//! anything, the user's own setting is recorded under
//! `HKCU\Software\Lulo\Shell`; [`restore`] puts it back and shows the
//! taskbar again. `lulo-shell` restores on every exit it controls, and
//! `lulo-session` restores after a crash or a kill, so the user's setting
//! never outlives Lulo. Start, the Win key and the tray keep working.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, ABS_ALWAYSONTOP, ABS_AUTOHIDE, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, RegisterWindowMessageW, ShowWindow, SW_HIDE, SW_SHOW,
};

use super::registry;

const RECORDED_STATE: &str = "TaskbarState";

/// The taskbar on the primary monitor and those on the others.
pub fn taskbar_windows() -> Vec<HWND> {
    let mut found = Vec::new();
    // SAFETY: looks up windows by class name.
    if let Ok(primary) = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) } {
        found.push(primary);
    }
    let mut after: Option<HWND> = None;
    loop {
        // SAFETY: as above, continuing after the last one found.
        match unsafe { FindWindowExW(None, after, w!("Shell_SecondaryTrayWnd"), PCWSTR::null()) } {
            Ok(next) if !next.is_invalid() => {
                found.push(next);
                after = Some(next);
            }
            _ => break,
        }
    }
    found
}

fn appbar_data() -> Option<APPBARDATA> {
    let tray = taskbar_windows().into_iter().next()?;
    Some(APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: tray,
        uCallbackMessage: 0,
        uEdge: 0,
        rc: RECT::default(),
        lParam: LPARAM(0),
    })
}

/// The taskbar's `ABS_*` state: auto-hide and always-on-top bits.
pub fn state() -> Option<u32> {
    let mut data = appbar_data()?;
    // SAFETY: a valid APPBARDATA naming the taskbar.
    Some(unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) } as u32)
}

fn set_state(state: u32) {
    let Some(mut data) = appbar_data() else {
        return;
    };
    data.lParam = LPARAM(state as isize);
    // SAFETY: as above.
    unsafe { SHAppBarMessage(ABM_SETSTATE, &mut data) };
}

/// Auto-hide and hide the taskbar, recording the user's setting first. An
/// existing record is kept: it holds the setting from before Lulo, which
/// an earlier run that could not restore it left behind.
pub fn take_over() {
    let Some(current) = state() else {
        return;
    };
    if registry::get_dword(RECORDED_STATE).is_none()
        && !registry::set_dword(RECORDED_STATE, current)
    {
        // Without a record there would be no way back: leave it alone.
        return;
    }
    set_state(ABS_AUTOHIDE | (current & ABS_ALWAYSONTOP));
    hide_windows();
}

/// Hide the taskbar windows again: Explorer shows an auto-hidden taskbar
/// once more after it applies the state change.
pub fn hide_windows() {
    for taskbar in taskbar_windows() {
        // SAFETY: hides Explorer's window until Lulo shows it again or
        // Explorer restarts.
        let _ = unsafe { ShowWindow(taskbar, SW_HIDE) };
    }
}

/// Put the user's taskbar setting back and show the taskbar. True when
/// there was a record to restore.
pub fn restore() -> bool {
    let recorded = registry::get_dword(RECORDED_STATE);
    if let Some(state) = recorded {
        set_state(state);
    }
    for taskbar in taskbar_windows() {
        // SAFETY: shows Explorer's own window again.
        let _ = unsafe { ShowWindow(taskbar, SW_SHOW) };
    }
    if recorded.is_some() {
        registry::delete_value(RECORDED_STATE);
    }
    recorded.is_some()
}

/// The message Explorer broadcasts when it (re)creates the taskbar; the
/// Lulo layer then hides it again and re-registers its AppBars.
pub fn taskbar_created_message() -> u32 {
    // SAFETY: registers (or looks up) a system-wide message name.
    unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
}
