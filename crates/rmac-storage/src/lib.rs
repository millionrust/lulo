//! Shared durable filesystem primitives for rmac applications.
//!
//! Apps retain their domain-specific operation enums and user-facing errors;
//! this crate owns the mechanics that must remain identical everywhere:
//! adjacent-temp atomic replacement, parent-directory durability, no-clobber
//! copies, and cleanup of partial files.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest as _, Sha256};

mod filesystem;
mod model;
mod read;
#[cfg(test)]
mod tests;
mod write;

pub use filesystem::*;
pub use model::*;
use read::read_open_file_bounded;
pub use read::*;
pub use write::*;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
