//! Version-aware global shortcut boundary with an explicit niri fallback.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use async_channel::Sender;
use serde::{Deserialize, Serialize};

mod configuration;
mod dispatch;
mod fallback;
pub mod lock;
pub mod lock_settings;
mod model;
mod portal;
#[cfg(test)]
mod tests;

pub use configuration::*;
pub use dispatch::*;
pub use fallback::*;
pub use model::*;
pub use portal::*;

pub const PORTAL_MINIMUM_VERSION: u32 = 1;
pub const PORTAL_CONFIGURE_VERSION: u32 = 2;
const CONTROL_PROTOCOL_VERSION: u8 = 1;
const CONTROL_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(any(target_os = "linux", test))]
const CONFIGURATION_REQUEST_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(2);
static CONTROL_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
