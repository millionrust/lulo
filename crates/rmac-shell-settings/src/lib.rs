//! Durable, compositor-independent settings authority for rmac shell processes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_storage::{Backend, Failure, FileSystem};
use serde::{Deserialize, Serialize};

mod migration;
mod model;
mod store;
#[cfg(test)]
mod tests;
mod validation;

use migration::*;
pub use model::*;
pub use store::*;
use validation::*;

const CURRENT_VERSION: u32 = 4;
const MAX_PINNED_APPS: usize = 128;
const MAX_DOCK_STACKS: usize = 32;
const MAX_SPOTLIGHT_EXCLUSIONS: usize = 128;
const MAX_RECOVERY_COPY_BYTES: usize = 1024 * 1024;
const LEGACY_FILES_APP_ID: &str = "org.rmac.Finder";
const FILES_APP_ID: &str = "org.rmac.Files";
