#![cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;

pub use linux::current_platform;
#[cfg(feature = "wayland")]
pub use linux::{
    begin_external_file_drag, external_file_drag_active, file_drop_should_copy,
    stage_external_file_drag,
};
