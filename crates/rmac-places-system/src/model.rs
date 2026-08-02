use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use sha2::Digest as _;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolveHome,
    ReadUserDirs,
    InspectPlace,
    InspectTrash,
    OpenDownloads,
    EmptyTrash,
    WatchPlaces,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResolveHome => "resolve the home directory",
            Self::ReadUserDirs => "read XDG user directories",
            Self::InspectPlace => "inspect a user place",
            Self::InspectTrash => "inspect Trash",
            Self::OpenDownloads => "open Downloads",
            Self::EmptyTrash => "empty Trash",
            Self::WatchPlaces => "watch user places",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub path: Option<PathBuf>,
    pub error_kind: Option<io::ErrorKind>,
    detail: String,
}

impl Error {
    pub(crate) fn message(
        operation: Operation,
        path: Option<&Path>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            path: path.map(Path::to_path_buf),
            error_kind: None,
            detail: detail.into(),
        }
    }

    pub(crate) fn from_io(operation: Operation, path: &Path, error: io::Error) -> Self {
        Self {
            operation,
            path: Some(path.to_path_buf()),
            error_kind: Some(error.kind()),
            detail: error.to_string(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}", self.operation)?;
        if let Some(path) = &self.path {
            write!(formatter, " at {}", path.display())?;
        }
        write!(formatter, ": {}", self.detail)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Report {
    pub snapshot: rmac_places::Snapshot,
    pub warnings: Vec<Error>,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct TrashEntryId([u8; 32]);

impl TrashEntryId {
    /// Derive a stable, path-free identity from the platform Trash authority.
    /// Callers never receive the original identifier bytes.
    pub fn from_authority_bytes(bytes: &[u8]) -> Self {
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"rmac-trash-entry-v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Self(hasher.finalize().into())
    }
}

impl fmt::Debug for TrashEntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TrashEntryId(<private>)")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum WatchEvent {
    Changed,
    Failed { detail: String },
}

impl fmt::Debug for WatchEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changed => formatter.write_str("Changed"),
            Self::Failed { .. } => formatter
                .debug_struct("Failed")
                .field("detail", &"<redacted>")
                .finish(),
        }
    }
}

pub struct Watcher {
    pub(crate) _watcher: notify::RecommendedWatcher,
}

pub trait Backend {
    fn home(&self) -> Option<PathBuf>;
    fn config_home(&self) -> Option<PathBuf>;
    fn read_optional(&self, path: &Path) -> io::Result<Option<String>>;
    fn exists(&self, path: &Path) -> io::Result<bool>;
    fn trash_count(&self) -> Result<usize, String>;
    fn trash_entries(&self) -> Result<Vec<TrashEntryId>, String>;
    fn purge_trash(&self, reviewed: &[TrashEntryId]) -> Result<(), String>;
}
