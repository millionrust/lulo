//! XDG autostart adapter for the supported Linux session.

use rmac_login_items::{
    AddPreview, BackgroundService, Error, ErrorKind, Issue, Item, RemovePreview, Service, Snapshot,
};
#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Read as _;
use std::path::{Path, PathBuf};

mod api;
mod backend;
mod service;
#[cfg(test)]
mod tests;
mod watch;

pub use api::*;
use backend::*;
pub use service::*;
pub use watch::*;

#[cfg(target_os = "linux")]
const RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
