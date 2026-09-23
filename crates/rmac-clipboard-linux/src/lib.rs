//! Session clipboard-history service for Spotlight's Clipboard view.
//!
//! The service watches the Wayland clipboard through wl-clipboard
//! (`wl-paste --watch`, which speaks ext-data-control or wlr-data-control,
//! whichever the compositor offers; docs/decisions/0011-clipboard-history.md
//! explains the choice), records items only after the user allowed it,
//! skips password-manager secrets, keeps payloads in a 0700 directory under
//! `$XDG_RUNTIME_DIR` (so logging out clears them) and exports the history
//! as `org.rmac.Clipboard1` on the session bus.

use std::fmt;

pub mod client;
pub mod service;
pub mod store;
pub mod wayland;

pub const BUS_NAME: &str = "org.rmac.Clipboard1";
pub const OBJECT_PATH: &str = "/org/rmac/Clipboard1";
pub const INTERFACE_NAME: &str = "org.rmac.Clipboard1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// No usable private directory (`$XDG_RUNTIME_DIR` unset or unsafe).
    Store,
    Bus,
    /// wl-clipboard is missing or the clipboard could not be read.
    Clipboard,
    Protocol,
    /// The history is off until the user allows it.
    Disabled,
    NotFound,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = match self {
            Self::Store => "clipboard history storage is unavailable",
            Self::Bus => "clipboard history service is unavailable",
            Self::Clipboard => "the clipboard could not be read or written",
            Self::Protocol => "clipboard history reply is invalid",
            Self::Disabled => "clipboard history is off",
            Self::NotFound => "clipboard item is no longer available",
        };
        formatter.write_str(detail)
    }
}

impl std::error::Error for Error {}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
