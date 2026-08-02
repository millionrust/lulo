//! Platform-neutral appearance state and event contracts.
//!
//! This crate deliberately has no GPUI, D-Bus, portal, or platform-FFI
//! dependency. Platform adapters implement [`AppearanceSource`], while apps and
//! shell surfaces consume complete snapshots and reduce events through
//! [`AppearanceState`].

mod model;
mod source;
mod state;

pub use model::{
    AccentColor, Capabilities, ColorScheme, Contrast, MotionPreference, ResolvedAppearance,
    ResolvedColorScheme, Snapshot, TextScale,
};
pub use source::{
    AppearanceSource, Error, FakeAppearanceController, FakeAppearanceSource, SourceFuture,
};
pub use state::{AppearanceState, Event};

#[cfg(test)]
mod tests;
