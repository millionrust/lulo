//! rmac's Open/Save panel, served as the xdg-desktop-portal FileChooser
//! backend (`org.freedesktop.impl.portal.FileChooser`). See
//! docs/decisions/0012-file-chooser-portal.md and design-lab/file-chooser.html.
//!
//! Everything in this library is framework-neutral and unit-tested; the GPUI
//! panel lives in the `rmac-file-chooser` binary.

pub mod browser;
#[cfg(target_os = "linux")]
pub mod dbus;
pub mod filter;
/// Go to Folder (⇧⌘G) path resolution, shared with Files.
pub use rmac_finder::goto;
pub mod metrics;
pub mod outcome;
pub mod parent;
pub mod request;
pub mod service;

#[cfg(test)]
mod tests;
