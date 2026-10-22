//! rmac Mission Control, App Exposé, Show Desktop, Spaces and hot corners
//! (docs/decisions/0014-mission-control.md).
//!
//! The service and its overlay are one library shared by Lulo OS's
//! `mission-control` binary and the Windows shell (`rmac-win-shell`), as
//! the app switcher's are (ADR 0023, "Mission Control on Windows"): the
//! same model, layout, overlay and hot corners, with the window list, the
//! window pictures and the surfaces underneath from each platform.

pub mod capture;
#[cfg(unix)]
pub mod ipc;
pub mod model;
#[cfg(all(target_os = "linux", feature = "wayland"))]
mod placement;
#[cfg(all(target_os = "linux", feature = "wayland"))]
mod screencopy;
#[cfg(any(all(target_os = "linux", feature = "wayland"), windows))]
mod service;
pub mod stage;

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub use service::run;
#[cfg(windows)]
pub use service::{request, start};
