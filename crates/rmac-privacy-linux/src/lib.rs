//! Linux portal privacy decisions backed by the XDG PermissionStore.

use rmac_privacy::{
    AutomaticUpdates, PackageSources, PortalDecision, PortalResource, ProStatus, ReleaseSupport,
    SecurityCoverageSnapshot, Snapshot,
};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use zbus::blocking::Proxy;
use zbus::zvariant::OwnedValue;

mod portal;
mod security;
#[cfg(test)]
mod tests;

pub use portal::*;
pub use security::*;

const DESTINATION: &str = "org.freedesktop.impl.portal.PermissionStore";
const PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";
const DEVICE_TABLE: &str = "devices";
const NOT_FOUND: &str = "org.freedesktop.portal.Error.NotFound";
const RESOURCES: [PortalResource; 2] = [PortalResource::Camera, PortalResource::Microphone];
const MAX_HELPER_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_DECISIONS: usize = 512;
const MAX_PERMISSIONS_PER_DECISION: usize = 32;
const MAX_PERMISSION_TOKEN_BYTES: usize = 256;
const MAX_PERMISSION_SUMMARY_BYTES: usize = 512;
const MAX_ERROR_BYTES: usize = 512;
const MAX_SECURITY_LIST_ITEMS: usize = 64;
const HELPER_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(target_os = "linux")]
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
