//! Crash-safe private Notification Center storage and recovery.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Resolve,
    CreateDirectory,
    Read,
    Parse,
    Validate,
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
    pub(super) fn new(operation: Operation, kind: ErrorKind) -> Self {
        Self { operation, kind }
    }

    pub(super) fn io(operation: Operation, error: io::Error) -> Self {
        Self::new(operation, ErrorKind::Io(error.kind()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "notification history operation failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Store(<redacted path>)")
    }
}

impl Store {
    pub fn from_environment() -> Result<Self, Error> {
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/state"))
            })
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        Ok(Self::at(state_home.join("rmac/notifications/history.json")))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<LoadSnapshot, Error> {
        match self.read_file(&self.path) {
            Ok(center) => Ok(LoadSnapshot {
                center,
                recovery: Recovery::None,
            }),
            Err(error) if error.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                match self.read_file(&self.backup_path()) {
                    Ok(center) => Ok(LoadSnapshot {
                        center,
                        recovery: Recovery::LastGood,
                    }),
                    Err(backup) if backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                        Ok(LoadSnapshot {
                            center: Center::default(),
                            recovery: Recovery::None,
                        })
                    }
                    Err(backup) => Err(backup),
                }
            }
            Err(error) if recoverable(error) => match self.read_file(&self.backup_path()) {
                Ok(center) => Ok(LoadSnapshot {
                    center,
                    recovery: Recovery::LastGood,
                }),
                Err(backup)
                    if recoverable(backup)
                        || backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) =>
                {
                    Ok(LoadSnapshot {
                        center: Center::default(),
                        recovery: Recovery::Empty,
                    })
                }
                Err(backup) => Err(backup),
            },
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, center: &Center) -> Result<(), Error> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        set_private_directory(parent)?;
        let file = StoredFile::from_center(center)?;
        let bytes = serde_json::to_vec(&file)
            .map_err(|_| Error::new(Operation::Serialize, ErrorKind::Invalid))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Serialize, ErrorKind::Limit));
        }
        rmac_storage::atomic_write_private(&self.backup_path(), &bytes)
            .map_err(|error| Error::io(Operation::Save, error))?;
        rmac_storage::atomic_write_private(&self.path, &bytes)
            .map_err(|error| Error::io(Operation::Save, error))
    }

    fn read_file(&self, path: &Path) -> Result<Center, Error> {
        let metadata =
            std::fs::metadata(path).map_err(|error| Error::io(Operation::Read, error))?;
        if metadata.len() > MAX_FILE_BYTES || !metadata.is_file() {
            return Err(Error::new(Operation::Read, ErrorKind::Limit));
        }
        let bytes = std::fs::read(path).map_err(|error| Error::io(Operation::Read, error))?;
        let file: StoredFile = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        file.into_center()
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

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| Error::io(Operation::CreateDirectory, error))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<(), Error> {
    Ok(())
}
