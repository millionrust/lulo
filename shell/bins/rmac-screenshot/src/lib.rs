//! rmac screenshots (docs/decisions/0010-screenshots.md): ⇧⌘3, ⇧⌘4 and
//! ⇧⌘5 with the crosshair, the toolbar and the floating thumbnail. Lulo OS
//! runs [`service`] as its own process (`screenshot --service`); the
//! Windows Lulo layer runs it in its own process ([`service::start`]).

pub mod capture;
#[cfg(unix)]
mod ipc;
pub mod model;
#[cfg(any(all(target_os = "linux", feature = "wayland"), windows))]
pub mod service;
