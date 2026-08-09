//! Linux lock-provider adapter primitives.
//!
//! Secret input uses one fixed-capacity allocation, redacted diagnostics, and
//! zeroization on backspace, clear, transfer, and drop.

pub mod accessibility;
#[cfg(any(target_os = "linux", test))]
mod caps_lock;
pub mod key_repeat;
pub mod keyboard;
pub mod paint;
pub mod pam_broker;
pub mod pam_conversation;
#[cfg(any(target_os = "linux", test))]
mod pointer;
#[cfg(any(test, all(target_os = "linux", feature = "provider")))]
mod process;
mod prompt_label;
#[cfg(any(target_os = "linux", test))]
mod registry_probe;
pub mod runtime;
mod secret;
pub mod surface;

pub use secret::*;

#[cfg(target_os = "linux")]
pub mod pam;
#[cfg(target_os = "linux")]
pub mod shm;
#[cfg(target_os = "linux")]
mod text_renderer;
#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
mod xkb_keyboard;

/// Installed fail-closed process entry point for the Linux session locker.
#[cfg(all(target_os = "linux", feature = "provider"))]
pub mod development_process;

#[cfg(test)]
mod evidence_assets;
#[cfg(test)]
mod tests;
