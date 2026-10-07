//! The Lulo layer on Windows (ADR 0023 phase 3): Lulo's menu bar, Dock and
//! Spotlight running on top of the Windows desktop, beside Explorer.
//!
//! - `model` holds what the surfaces show, as plain data built from plain
//!   data, so it is unit-tested on every platform.
//! - `win` wraps the Win32 pieces: AppBars, the taskbar, the window list
//!   and its WinEvent hooks, the global hotkey, launching, the app and file
//!   catalogue, status readings, power commands and the menu pipe.
//! - `ui` draws the surfaces with GPUI and rmac-ui.
//!
//! Nothing polls: every update arrives as a Windows event, a pipe message
//! or a change notification, and an idle Lulo layer uses no CPU (the
//! `windows` CI job's idle gate measures it).

pub mod model;

#[cfg(windows)]
pub mod win;

#[cfg(windows)]
mod ui;

#[cfg(windows)]
pub use ui::run;
