//! Wifi settings composition boundary.

use super::*;

mod connection;
mod credentials;
mod render;
mod scan;
mod state;

#[cfg(test)]
pub(in crate::controller) use scan::{
    wifi_pane_is_visible, wifi_pane_scan_should_start_on_navigation,
};
