//! Framework-neutral notification validation, policy, and state transitions.
//!
//! D-Bus and XDG portal adapters normalize untrusted requests into [`Request`].
//! The reducer intentionally owns no bus connection, timer, persistence file,
//! sound player, or UI so every presentation surface observes one authority.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

pub mod banner;
mod model;
pub mod protocol;
mod reducer;
#[cfg(test)]
mod tests;

pub use model::*;
use reducer::*;

const MAX_APP_ID_BYTES: usize = 256;
const MAX_EXTERNAL_ID_BYTES: usize = 256;
const MAX_TITLE_BYTES: usize = 512;
const MAX_BODY_BYTES: usize = 16 * 1024;
const MAX_ACTIONS: usize = 8;
const MAX_ACTION_ID_BYTES: usize = 256;
const MAX_ACTION_LABEL_BYTES: usize = 256;
const MAX_TARGET_BYTES: usize = 16 * 1024;
const MAX_CATEGORY_BYTES: usize = 128;
const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1_000;
