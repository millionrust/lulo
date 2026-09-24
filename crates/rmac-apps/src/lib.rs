//! Cross-platform installed-application catalog and launcher.

#![cfg_attr(target_os = "macos", allow(dead_code))]

mod catalog;
mod icons;
pub mod identity;
mod model;
mod platform;
#[cfg(test)]
mod tests;

pub use catalog::*;
use icons::*;
pub use model::*;
use platform::*;
// Only the desktop-entry tokenizer is re-exported, for the desktop_entry
// fuzz target; the rest of `platform` stays crate-private.
pub use platform::desktop_group_named;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
#[cfg(target_os = "linux")]
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
#[cfg(target_os = "linux")]
use std::process::{ExitStatus, Stdio};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
