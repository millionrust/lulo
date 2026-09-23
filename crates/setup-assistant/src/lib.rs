//! rmac Setup Assistant: the first-login flow (design-lab/setup-assistant.html).
//!
//! The library holds everything that is not drawing: the page flow state
//! machine, the greeting's timing, locale names, the run-once marker and the
//! backends each page writes to. The binary draws it with rmac-ui.

pub mod flow;
pub mod greeting;
pub mod marker;
pub mod names;
pub mod services;

/// Wayland app ID and desktop identity.
pub const APP_ID: &str = "org.rmac.SetupAssistant";

#[cfg(test)]
mod tests;
