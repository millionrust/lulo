//! The work area when Lulo is the user's shell (ADR 0023, "Lulo as the
//! shell").
//!
//! Over Explorer the menu bar and the Dock are AppBars: `SHAppBarMessage`
//! asks Explorer's taskbar, which keeps the work area. Without Explorer
//! nobody answers those messages, so maximised windows would cover the bar
//! and the Dock. In that mode ([`set_shell`]) the strips are kept here and
//! each monitor's work area is set directly (`SPI_SETWORKAREA`): the
//! monitor less every strip on it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPIF_SENDCHANGE, SPI_SETWORKAREA,
};

use super::appbar::Edge;

static SHELL: AtomicBool = AtomicBool::new(false);

struct Strip {
    hwnd: isize,
    edge: Edge,
    monitor: RECT,
    rect: RECT,
}

static STRIPS: Mutex<Vec<Strip>> = Mutex::new(Vec::new());

/// Lulo is the shell: keep the work area here instead of asking Explorer.
pub fn set_shell(on: bool) {
    SHELL.store(on, Ordering::Release);
}

/// Whether Lulo is the shell (no Explorer taskbar keeps the work area).
pub fn is_shell() -> bool {
    SHELL.load(Ordering::Acquire)
}

/// `monitor` less the strips held on it.
pub(crate) fn work_area(monitor: RECT, strips: &[(Edge, RECT)]) -> RECT {
    let mut area = monitor;
    for (edge, rect) in strips {
        match edge {
            Edge::Top => area.top = area.top.max(rect.bottom),
            Edge::Bottom => area.bottom = area.bottom.min(rect.top),
            Edge::Left => area.left = area.left.max(rect.right),
            Edge::Right => area.right = area.right.min(rect.left),
        }
    }
    if area.bottom <= area.top || area.right <= area.left {
        return monitor;
    }
    area
}

fn apply(strips: &[Strip], monitor: RECT) {
    let held: Vec<(Edge, RECT)> = strips
        .iter()
        .filter(|strip| strip.monitor == monitor)
        .map(|strip| (strip.edge, strip.rect))
        .collect();
    let mut area = work_area(monitor, &held);
    // SAFETY: passes a RECT for SPI_SETWORKAREA to read; Windows sets the
    // work area of the monitor that rectangle is on.
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_SETWORKAREA,
            0,
            Some(&mut area as *mut RECT as *mut core::ffi::c_void),
            SPIF_SENDCHANGE,
        )
    };
}

/// Hold a strip `thickness` deep along `edge` of `monitor` for `hwnd`,
/// and return it.
pub fn reserve(hwnd: HWND, edge: Edge, thickness: i32, monitor: RECT) -> RECT {
    let mut rect = monitor;
    edge.fit(&mut rect, thickness);
    let mut strips = STRIPS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let raw = hwnd.0 as isize;
    let previous = strips
        .iter()
        .position(|strip| strip.hwnd == raw)
        .map(|index| strips.remove(index));
    strips.push(Strip {
        hwnd: raw,
        edge,
        monitor,
        rect,
    });
    if let Some(previous) = previous.filter(|previous| previous.monitor != monitor) {
        apply(&strips, previous.monitor);
    }
    apply(&strips, monitor);
    rect
}

/// Give `hwnd`'s strip back to the work area.
pub fn remove(hwnd: HWND) {
    let mut strips = STRIPS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let raw = hwnd.0 as isize;
    if let Some(index) = strips.iter().position(|strip| strip.hwnd == raw) {
        let strip = strips.remove(index);
        apply(&strips, strip.monitor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn the_work_area_is_the_monitor_less_the_bar_and_the_dock() {
        let monitor = rect(0, 0, 1366, 768);
        let bar = (Edge::Top, rect(0, 0, 1366, 29));
        let dock = (Edge::Bottom, rect(0, 679, 1366, 768));
        assert_eq!(work_area(monitor, &[bar, dock]), rect(0, 29, 1366, 679));
        assert_eq!(work_area(monitor, &[]), monitor);
        let side = (Edge::Left, rect(0, 0, 80, 768));
        assert_eq!(work_area(monitor, &[side]), rect(80, 0, 1366, 768));
    }

    #[test]
    fn strips_that_leave_nothing_are_ignored() {
        let monitor = rect(0, 0, 100, 100);
        let huge = (Edge::Top, rect(0, 0, 100, 120));
        assert_eq!(work_area(monitor, &[huge]), monitor);
    }
}
