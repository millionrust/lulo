//! Platform-neutral software-update snapshots, plans, and progress rules.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

mod catalog;
mod collector;
mod install;
mod model;
mod normalize;
mod notes;
mod plan;
mod prefs;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_catalog;

pub use catalog::*;
pub use collector::*;
pub use install::*;
pub use model::*;
use normalize::*;
pub use notes::*;
pub use plan::*;
pub use prefs::*;

pub const MAX_UPDATES: usize = 512;
pub const MAX_PLAN_CHANGES: usize = 1024;
const MAX_PACKAGE_ID_BYTES: usize = 1024;
const MAX_TEXT_BYTES: usize = 512;
