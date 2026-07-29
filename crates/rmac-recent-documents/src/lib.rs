//! Private, bounded, cross-process recent-document authority for rmac apps.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

mod lock;
mod stored;

use lock::FileLock;
use stored::{safe_document_path, StoredEntry, StoredFile};

const VERSION: u32 = 1;
const MAX_ENTRIES: usize = 256;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_URI_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Resolve,
    InspectDocument,
    CreateDirectory,
    Lock,
    Read,
    Parse,
    Validate,
    Serialize,
    Save,
    Clear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Io(io::ErrorKind),
    Invalid,
    UnsupportedVersion,
    Limit,
    UnsafeDocument,
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub kind: ErrorKind,
}

impl Error {
    fn new(operation: Operation, kind: ErrorKind) -> Self {
        Self { operation, kind }
    }

    fn io(operation: Operation, error: io::Error) -> Self {
        Self::new(operation, ErrorKind::Io(error.kind()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match (self.operation, self.kind) {
            (Operation::Resolve, _) => "recent-document storage is unavailable",
            (Operation::InspectDocument, ErrorKind::UnsafeDocument) => {
                "only a local regular document can be added to Recents"
            }
            (Operation::Lock, ErrorKind::Busy) => "recent-document storage is temporarily busy",
            (Operation::Read | Operation::Parse | Operation::Validate, _) => {
                "recent documents could not be read"
            }
            (Operation::Clear, _) => "recent documents could not be cleared",
            _ => "the recent document could not be recorded",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Recovery {
    #[default]
    None,
    LastGood,
    Empty,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub paths: Vec<PathBuf>,
    pub recovery: Recovery,
    /// Desktop-history entries at or before this wall-clock boundary are
    /// intentionally hidden from rmac's merged Recents view.
    pub cleared_before_unix_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordOutcome {
    Added,
    Refreshed,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Store")
            .field("path", &"<redacted>")
            .finish()
    }
}

impl Store {
    pub fn from_environment() -> Result<Self, Error> {
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .map(|home| home.join(".local/state"))
            })
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        Ok(Self {
            path: state_home.join("rmac/recent-documents.json"),
        })
    }

    pub fn load(&self) -> Result<Snapshot, Error> {
        let (stored, recovery) = self.load_stored()?;
        let cleared_before_unix_ms = stored.cleared_before_unix_ms;
        Ok(Snapshot {
            paths: stored.live_paths(),
            recovery,
            cleared_before_unix_ms,
        })
    }

    pub fn record(&self, path: &Path) -> Result<RecordOutcome, Error> {
        let path = safe_document_path(path)?;
        let uri = url::Url::from_file_path(&path)
            .map_err(|()| Error::new(Operation::InspectDocument, ErrorKind::UnsafeDocument))?
            .to_string();
        if uri.len() > MAX_URI_BYTES {
            return Err(Error::new(Operation::Validate, ErrorKind::Limit));
        }

        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        rmac_storage::create_dir_all_private(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        let _lock = FileLock::acquire(&parent.join("recent-documents.lock"))?;
        let (mut stored, _) = self.load_stored()?;
        let outcome = if stored.entries.iter().any(|entry| entry.uri == uri) {
            RecordOutcome::Refreshed
        } else {
            RecordOutcome::Added
        };
        stored.entries.retain(|entry| entry.uri != uri);
        let newest = stored
            .entries
            .iter()
            .map(|entry| entry.used_at_unix_ms)
            .max()
            .unwrap_or(0);
        let now = current_unix_ms();
        stored.entries.insert(
            0,
            StoredEntry {
                uri,
                used_at_unix_ms: now
                    .max(newest.saturating_add(1))
                    .max(stored.cleared_before_unix_ms.unwrap_or(0).saturating_add(1)),
            },
        );
        stored.entries.truncate(MAX_ENTRIES);
        self.save(&stored, Operation::Save)?;
        Ok(outcome)
    }

    /// Clear rmac records and advance the boundary used to suppress older
    /// desktop XBEL records. Returns the number of rmac-owned records removed.
    pub fn clear(&self) -> Result<usize, Error> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        rmac_storage::create_dir_all_private(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        let _lock = FileLock::acquire(&parent.join("recent-documents.lock"))?;
        let (stored, _) = self.load_stored()?;
        let removed = stored.entries.len();
        let cleared_before_unix_ms = Some(
            current_unix_ms().max(stored.cleared_before_unix_ms.unwrap_or(0).saturating_add(1)),
        );
        self.save(
            &StoredFile::cleared(cleared_before_unix_ms),
            Operation::Clear,
        )?;
        Ok(removed)
    }

    fn load_stored(&self) -> Result<(StoredFile, Recovery), Error> {
        match self.read(&self.path) {
            Ok(stored) => Ok((stored, Recovery::None)),
            Err(error) if error.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                match self.read(&self.backup_path()) {
                    Ok(stored) => Ok((stored, Recovery::LastGood)),
                    Err(backup) if backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                        Ok((StoredFile::default(), Recovery::None))
                    }
                    Err(backup) if recoverable(backup) => {
                        Ok((StoredFile::default(), Recovery::Empty))
                    }
                    Err(backup) => Err(backup),
                }
            }
            Err(error) if recoverable(error) => match self.read(&self.backup_path()) {
                Ok(stored) => Ok((stored, Recovery::LastGood)),
                Err(backup)
                    if recoverable(backup)
                        || backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) =>
                {
                    Ok((StoredFile::default(), Recovery::Empty))
                }
                Err(backup) => Err(backup),
            },
            Err(error) => Err(error),
        }
    }

    fn read(&self, path: &Path) -> Result<StoredFile, Error> {
        let bytes = rmac_storage::read_bounded_no_follow(path, MAX_FILE_BYTES)
            .map_err(|error| Error::io(Operation::Read, error))?;
        let stored: StoredFile = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        stored.validate()?;
        Ok(stored)
    }

    fn save(&self, stored: &StoredFile, operation: Operation) -> Result<(), Error> {
        stored.validate()?;
        let bytes = serde_json::to_vec(stored)
            .map_err(|_| Error::new(Operation::Serialize, ErrorKind::Invalid))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Serialize, ErrorKind::Limit));
        }
        rmac_storage::atomic_write_private(&self.backup_path(), &bytes)
            .map_err(|error| Error::io(operation, error))?;
        rmac_storage::atomic_write_private(&self.path, &bytes)
            .map_err(|error| Error::io(operation, error))
    }

    fn backup_path(&self) -> PathBuf {
        self.path.with_extension("last-good.json")
    }
}

fn recoverable(error: Error) -> bool {
    matches!(
        error.kind,
        ErrorKind::Invalid | ErrorKind::UnsupportedVersion | ErrorKind::Limit
    )
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests;
