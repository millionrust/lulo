//! Lulo's ⌘Tab application switcher (docs/decisions/0009-app-switcher.md),
//! one view for Lulo OS and Windows (ADR 0023, "Notification Centre and
//! the app switcher on Windows").
//!
//! On Lulo OS `app-switcher --service` is the resident session component:
//! niri's ⌘Tab / ⌘⇧Tab binds run `app-switcher next|previous`, which
//! forwards one word to the service and exits. The service opens an
//! exclusive-keyboard overlay, tracks ⌘ through the surface's modifier
//! events, and activates the selected application when ⌘ is released.
//!
//! On Windows `lulo-shell` hosts the same service ([`start`]) and Alt+Tab
//! reaches it through a low-level keyboard hook (`alt_tab`).

#[cfg(windows)]
mod alt_tab;
#[cfg_attr(not(all(target_os = "linux", feature = "wayland")), allow(dead_code))]
mod force_quit;
#[cfg(any(all(target_os = "linux", feature = "wayland"), windows))]
mod force_quit_window;
#[cfg(unix)]
#[cfg_attr(not(all(target_os = "linux", feature = "wayland")), allow(dead_code))]
mod ipc;
#[cfg_attr(not(all(target_os = "linux", feature = "wayland")), allow(dead_code))]
mod model;
#[cfg(any(all(target_os = "linux", feature = "wayland"), windows))]
mod service;

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub use service::run;
#[cfg(windows)]
pub use service::start;
