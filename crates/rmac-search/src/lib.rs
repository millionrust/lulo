//! Cross-platform file search and recent-document providers.

#[cfg(any(not(target_os = "macos"), test))]
use std::collections::HashSet;
use std::fmt;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

mod model;
mod platform;
mod ranked;
#[cfg(test)]
mod tests;

pub use model::*;
#[cfg(test)]
use platform::*;
use ranked::*;

const DEFAULT_LIMIT: usize = 500;
const DEFAULT_MAX_ENTRIES: usize = 100_000;
const DEFAULT_MAX_CONTENT_FILE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_TOTAL_CONTENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_QUERY_BYTES: usize = 512;
const MAX_EXCERPT_CHARACTERS: usize = 240;

pub fn filenames(root: &Path, query: &str, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
    SystemSearchProvider.filenames(root, query, options)
}

/// Search names and bounded regular UTF-8 file content without following links.
///
/// This portable authority is intentionally independent of Spotlight so Files
/// can present exact match reasons and excerpts with identical safety bounds on
/// Linux and macOS. The legacy filename provider remains available to callers
/// such as Launcher that prefer the platform index.
pub fn ranked(root: &Path, query: &str, options: Options<'_>) -> Result<SearchReport, Error> {
    filesystem_ranked_search(root, query, options)
}

pub fn recents(options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
    SystemSearchProvider.recents(options)
}

pub fn tagged(tag: &str, options: Options<'_>) -> Result<Vec<PathBuf>, Error> {
    SystemSearchProvider.tagged(tag, options)
}
