//! Framework-neutral Dock application and activation model.

pub mod accessibility;
pub mod badges;
pub mod bounce;
mod dock;
pub mod drag;
pub mod keyboard;
pub mod menu;
mod model;
pub mod motion;
mod pins;
pub mod presentation;
pub mod recents;
pub mod reorder;
mod stacks;
mod surfaces;
#[cfg(test)]
mod tests;

pub use dock::ResolvedStack;
use dock::*;
pub use model::*;
pub use pins::*;
pub use stacks::*;
pub use surfaces::*;

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
