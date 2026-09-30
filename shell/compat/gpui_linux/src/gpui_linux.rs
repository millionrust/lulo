#![cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;

pub use linux::current_platform;
#[cfg(feature = "wayland")]
pub use linux::{external_file_drag_active, stage_external_file_drag};
