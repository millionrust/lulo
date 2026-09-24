//! Cross-platform battery state and system power-profile controls.

use std::fmt;
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(any(test, feature = "test-support"))]
pub mod contract;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
#[cfg(any(not(target_os = "macos"), test))]
mod linux;
// The macOS parsers also build under test, so their fixtures run on Linux.
#[cfg(any(target_os = "macos", test))]
mod macos;
mod model;
#[cfg(test)]
mod tests;

#[cfg(any(not(target_os = "macos"), test))]
use linux::*;
#[cfg(all(test, not(target_os = "macos")))]
use macos::parse_macos_battery;
#[cfg(target_os = "macos")]
use macos::*;
use model::percent;
pub use model::*;
