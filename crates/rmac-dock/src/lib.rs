//! Framework-neutral Dock application and activation model.

pub mod accessibility;
pub mod bounce;
mod dock;
pub mod drag;
pub mod menu;
mod model;
pub mod motion;
mod pins;
pub mod presentation;
mod surfaces;
#[cfg(test)]
mod tests;

use dock::*;
pub use model::*;
pub use pins::*;
pub use surfaces::*;

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
