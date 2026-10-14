//! Cross-platform battery state and system power-profile controls.

use std::fmt;
#[cfg(not(any(target_os = "linux", windows)))]
use std::process::Command;

#[cfg(any(test, feature = "test-support"))]
pub mod contract;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
#[cfg(any(target_os = "linux", test))]
mod linux;
// The macOS parsers also build under test, so their fixtures run on Linux.
pub mod low_power;
#[cfg(any(not(any(target_os = "linux", windows)), test))]
mod macos;
mod model;
pub mod screen_usage;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod win;

#[cfg(any(target_os = "linux", test))]
use linux::*;
#[cfg(all(test, any(target_os = "linux", windows)))]
use macos::parse_macos_battery;
#[cfg(not(any(target_os = "linux", windows)))]
use macos::*;
use model::percent;
pub use model::*;
#[cfg(windows)]
use win::*;
