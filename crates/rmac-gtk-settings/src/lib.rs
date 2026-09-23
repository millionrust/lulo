//! GNOME interface text scaling for GTK applications, and the appearance
//! rmac hands to third-party toolkits (GTK 3, GTK 4, libadwaita, Qt through
//! its GTK platform theme, Firefox and Chromium through the Settings portal).
//!
//! GSettings is authoritative for text scaling; mutations require writability
//! and exact readback from the same authority.

mod api;
mod process;
mod toolkit;
mod watch;

pub use api::{set_text_scale, snapshot, watch, Error, Snapshot, WatchEvent};
pub use toolkit::{
    sync_toolkit_appearance, ToolkitAppearance, ToolkitFollower, CURSOR_SIZE, CURSOR_THEME,
    DECORATION_LAYOUT, DEFAULT_ACCENT, FONT_NAME, GTK_THEME,
};

#[cfg(test)]
mod tests;
