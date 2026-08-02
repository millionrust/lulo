//! Cross-platform battery state and system power-profile controls.

use std::fmt;
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(any(not(target_os = "macos"), test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod model;
#[cfg(test)]
mod tests;

#[cfg(any(not(target_os = "macos"), test))]
use linux::*;
#[cfg(target_os = "macos")]
use macos::*;
use model::percent;
pub use model::*;
