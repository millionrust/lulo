use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Error, ErrorKind, Operation, MAX_ENTRIES, MAX_URI_BYTES, VERSION};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct StoredFile {
    version: u32,
    pub(super) entries: Vec<StoredEntry>,
    #[serde(default)]
    pub(super) cleared_before_unix_ms: Option<u64>,
}

impl Default for StoredFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            entries: Vec::new(),
            cleared_before_unix_ms: None,
        }
    }
}

impl StoredFile {
    pub(super) fn cleared(cleared_before_unix_ms: Option<u64>) -> Self {
        Self {
            cleared_before_unix_ms,
            ..Self::default()
        }
    }

    pub(super) fn validate(&self) -> Result<(), Error> {
        if self.version != VERSION {
            return Err(Error::new(
                Operation::Validate,
                ErrorKind::UnsupportedVersion,
            ));
        }
        if self.entries.len() > MAX_ENTRIES {
            return Err(Error::new(Operation::Validate, ErrorKind::Limit));
        }
        let mut seen = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            if entry.uri.len() > MAX_URI_BYTES
                || !seen.insert(entry.uri.as_str())
                || uri_path(&entry.uri).is_none()
            {
                return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
            }
        }
        Ok(())
    }

    pub(super) fn live_paths(mut self) -> Vec<PathBuf> {
        self.entries
            .sort_by_key(|entry| std::cmp::Reverse(entry.used_at_unix_ms));
        self.entries
            .into_iter()
            .filter_map(|entry| uri_path(&entry.uri))
            .filter_map(|path| safe_document_path(&path).ok())
            .take(MAX_ENTRIES)
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct StoredEntry {
    pub(super) uri: String,
    pub(super) used_at_unix_ms: u64,
}

fn uri_path(uri: &str) -> Option<PathBuf> {
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    path.is_absolute().then_some(path)
}

pub(super) fn safe_document_path(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute() {
        return Err(Error::new(
            Operation::InspectDocument,
            ErrorKind::UnsafeDocument,
        ));
    }
    let source_metadata = std::fs::symlink_metadata(path)
        .map_err(|error| Error::io(Operation::InspectDocument, error))?;
    if !source_metadata.file_type().is_file() {
        return Err(Error::new(
            Operation::InspectDocument,
            ErrorKind::UnsafeDocument,
        ));
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| Error::io(Operation::InspectDocument, error))?;
    let metadata = std::fs::symlink_metadata(&canonical)
        .map_err(|error| Error::io(Operation::InspectDocument, error))?;
    if !metadata.file_type().is_file() {
        return Err(Error::new(
            Operation::InspectDocument,
            ErrorKind::UnsafeDocument,
        ));
    }
    Ok(canonical)
}
