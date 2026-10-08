//! Session-owned, Spotlight-style launcher surface. Linux runs it as its
//! own process (`rmac-launcher`); the Windows shell hosts the same view in
//! `lulo-shell` (ADR 0023, "Phase 3 revised: shared shell views").

mod assets;
mod intelligence;
mod service;
mod view;

pub use assets::{asset, asset_names};
pub use service::start;
#[cfg(not(target_os = "linux"))]
pub use service::toggle;

/// Run the launcher as its own process, answering its shortcut endpoint.
pub fn run() {
    service::run();
}
