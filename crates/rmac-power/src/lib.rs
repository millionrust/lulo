//! Cross-platform battery state and system power-profile controls.

use std::fmt;
#[cfg(not(target_os = "linux"))]
use std::process::Command;

#[cfg(any(test, feature = "test-support"))]
pub mod contract;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
#[cfg(any(target_os = "linux", test))]
mod linux;
// The macOS parsers also build under test, so their fixtures run on Linux.
#[cfg(any(not(target_os = "linux"), test))]
mod macos;
mod model;
#[cfg(test)]
mod tests;

#[cfg(any(target_os = "linux", test))]
use linux::*;
#[cfg(all(test, target_os = "linux"))]
use macos::parse_macos_battery;
#[cfg(not(target_os = "linux"))]
use macos::*;
use model::percent;
pub use model::*;
