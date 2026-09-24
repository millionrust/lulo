//! GNOME's screen-reader toggle for the Lulo session: the VoiceOver
//! equivalent, `org.gnome.desktop.a11y.applications screen-reader-enabled`,
//! and Orca's process lifecycle.
//!
//! GSettings is authoritative for the on/off state. `set_enabled` is the
//! only place in rmac that starts or stops Orca, so the System Settings
//! switch and the Screen Reader shortcut (both of which call it) are the
//! only things in the Lulo session that can make Orca start talking. GNOME's
//! own Super+Alt+S accelerator is disabled for the session
//! (`disable_gnome_shortcut`) rather than watched and honored, so it cannot
//! surprise the owner the way it can on stock GNOME.

mod api;
mod orca;
mod process;
mod watch;

pub use api::{disable_gnome_shortcut, set_enabled, snapshot, watch, Error, Snapshot, WatchEvent};
pub use orca::is_orca_running;
