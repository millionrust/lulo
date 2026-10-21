//! Typed launch and niri-focus execution for Dock activation outcomes.

mod backend;
pub mod dispatch;
mod execution;
pub mod icons;
pub mod interaction;
mod model;

pub use backend::SystemBackend;

/// App Exposé for the Dock's Show All Windows on Windows, where Mission
/// Control runs in the same `lulo-shell` process as the Dock: the shell
/// installs how to reach it (Lulo OS runs `rmac-mission-control`).
#[cfg(windows)]
pub mod app_windows {
    use std::sync::OnceLock;

    type Handler = Box<dyn Fn() -> bool + Send + Sync>;

    static HANDLER: OnceLock<Handler> = OnceLock::new();

    /// Install the request; `handler` returns whether Mission Control took it.
    pub fn set_handler(handler: impl Fn() -> bool + Send + Sync + 'static) {
        let _ = HANDLER.set(Box::new(handler));
    }

    /// Ask for App Exposé; false when nothing is installed.
    pub(crate) fn request() -> bool {
        HANDLER.get().is_some_and(|handler| handler())
    }
}
pub use execution::*;
pub use model::*;

#[cfg(test)]
mod tests;
