//! Local application, setting, file, and calculator launcher providers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rmac_launcher::{
    Action, Cancellation, Category, Privacy, ProviderDescriptor, ProviderError, Request, ResultId,
    SearchResult,
};

mod applications;
mod calculator;
mod files;
mod helpers;
mod model;
mod settings;
#[cfg(test)]
mod tests;

pub use applications::*;
pub use calculator::*;
pub use files::*;
use helpers::*;
pub use model::*;
pub use settings::*;

pub const APPLICATIONS_PROVIDER: &str = "applications";
pub const SETTINGS_PROVIDER: &str = "settings";
pub const FILES_PROVIDER: &str = "files";
pub const CALCULATOR_PROVIDER: &str = "calculator";

const PROVIDER_LIMIT: usize = 100;
