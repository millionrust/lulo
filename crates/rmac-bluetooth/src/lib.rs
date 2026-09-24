//! Cross-platform Bluetooth adapter and known-device service.

#[cfg(any(test, feature = "test-support"))]
pub mod contract;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
#[cfg(not(target_os = "macos"))]
mod linux;
mod macos;
mod pairing_agent;
mod service;
mod watch;

pub use pairing_agent::{
    PairingEvent, PairingInputError, PairingPasskey, PairingPinCode, PairingPrompt,
    PairingPromptId, PairingPromptKind, PairingSession,
};
pub use service::*;

#[cfg(test)]
mod tests;
