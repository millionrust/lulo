//! Concurrent provider orchestration and overlay-facing launcher lifecycle.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, StreamExt as _};
use rmac_launcher::{
    ActivationMode, MoveSelection, ProviderDescriptor, ProviderError, Request, ResultId,
};
use rmac_launcher_providers::{Batch, Provider};

pub mod accessibility;
mod coordinator;
mod model;
mod registry;
#[cfg(test)]
mod tests;
mod watchers;

pub use model::*;
pub use registry::*;
pub use watchers::*;
