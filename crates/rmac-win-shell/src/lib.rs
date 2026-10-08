//! Lulo on Windows (ADR 0023 phase 3, "Phase 3 revised: shared shell
//! views"): Lulo OS's own menu bar, Dock and desktop views, running over
//! the Windows desktop beside Explorer.
//!
//! - The views are `shell/bins`' libraries, the same code as on Lulo OS;
//!   `shell` starts them in one process over Windows' backends.
//! - `share` lays out the Lulo apps' desktop entries and artwork as on
//!   Lulo OS.
//! - `model` holds plain data (Spotlight's hotkey choice, the Windows app
//!   catalogue's entries), unit-tested on every platform.
//! - `win` wraps the Win32 pieces only Windows has: the taskbar and
//!   Explorer's desktop icons, the session watchdog, the hotkey, Use Files
//!   for Folders, the registry records, memory trims, the Windows app
//!   catalogue and icon helpers, and the wallpaper layer.
//!
//! Nothing polls: every update arrives as a Windows event, a pipe message
//! or a change notification, and an idle Lulo layer uses no CPU (the
//! `windows` CI job's idle gate measures it).

pub mod model;

#[cfg(windows)]
pub mod share;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
pub mod win;

#[cfg(windows)]
pub use shell::run;
