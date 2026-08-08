//! Versioned, private per-application window geometry persistence.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_storage::{Backend as _, FileSystem};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const MAX_FILE_BYTES: usize = 8 * 1024;
const MAX_APP_ID_BYTES: usize = 128;

mod geometry;

pub use geometry::{DisplayBounds, WindowMode, WindowState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ResolvePath,
    Read,
    Parse,
    Validate,
    CreateDirectory,
    Serialize,
    Save,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Io(io::ErrorKind),
    Invalid,
    UnsupportedVersion,
    Limit,
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
        write!(
            formatter,
            "Window-state operation failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn from_environment(app_id: &str) -> Result<Self, Error> {
        validate_app_id(app_id)?;
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/state"))
            })
            .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))?;
        Ok(Self::at(
            state_home
                .join("rmac/windows")
                .join(format!("{app_id}.json")),
        ))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<WindowState>, Error> {
        match self.read(&self.path) {
            Ok(Some(state)) => Ok(Some(state)),
            Ok(None) => self.read(&self.last_good_path()),
            Err(primary) if recoverable(primary) => match self.read(&self.last_good_path()) {
                Ok(Some(state)) => Ok(Some(state)),
                Ok(None) => Err(primary),
                Err(_) => Err(primary),
            },
            Err(error) => Err(error),
        }
    }

    fn read(&self, path: &Path) -> Result<Option<WindowState>, Error> {
        let bytes = match FileSystem.read_bounded_no_follow(path, MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(Operation::Read, error)),
        };
        let stored: StoredState = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        if stored.version != VERSION {
            return Err(Error::new(Operation::Parse, ErrorKind::UnsupportedVersion));
        }
        stored
            .state
            .is_valid()
            .then_some(Some(stored.state))
            .ok_or_else(|| Error::new(Operation::Validate, ErrorKind::Invalid))
    }

    pub fn save(&self, state: WindowState) -> Result<(), Error> {
        if !state.is_valid() {
            return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))?;
        rmac_storage::create_dir_all_private(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        let bytes = serde_json::to_vec(&StoredState {
            version: VERSION,
            state,
        })
        .map_err(|_| Error::new(Operation::Serialize, ErrorKind::Invalid))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Serialize, ErrorKind::Limit));
        }
        rmac_storage::atomic_write_private(&self.last_good_path(), &bytes)
            .map_err(|error| Error::io(Operation::Save, error))?;
        rmac_storage::atomic_write_private(&self.path, &bytes)
            .map_err(|error| Error::io(Operation::Save, error))
    }

    fn last_good_path(&self) -> PathBuf {
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("window.json");
        self.path.with_file_name(format!("{name}.last-good"))
    }
}

fn recoverable(error: Error) -> bool {
    matches!(
        error.kind,
        ErrorKind::Invalid
            | ErrorKind::UnsupportedVersion
            | ErrorKind::Limit
            | ErrorKind::Io(io::ErrorKind::InvalidData)
    )
}

fn validate_app_id(app_id: &str) -> Result<(), Error> {
    let valid = !app_id.is_empty()
        && app_id.len() <= MAX_APP_ID_BYTES
        && app_id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && app_id
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && app_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        && !app_id.contains("..");
    valid
        .then_some(())
        .ok_or_else(|| Error::new(Operation::ResolvePath, ErrorKind::Invalid))
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredState {
    version: u32,
    state: WindowState,
}

#[cfg(test)]
mod tests;
