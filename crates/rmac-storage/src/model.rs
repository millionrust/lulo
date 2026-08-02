//! Stable storage model, failure, and injectable backend contract.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileFingerprint {
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

/// Exact state reviewed before replacing a user-selected export destination.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DestinationBaseline {
    Missing,
    Exact(FileFingerprint),
}

impl fmt::Debug for DestinationBaseline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("Missing"),
            Self::Exact(fingerprint) => formatter
                .debug_struct("Exact")
                .field("byte_len", &fingerprint.byte_len)
                .field("sha256", &"<redacted>")
                .finish(),
        }
    }
}

/// Which side of a verified streaming copy failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifiedCopyError {
    Source(io::ErrorKind),
    Destination(io::ErrorKind),
}

impl VerifiedCopyError {
    pub fn kind(self) -> io::ErrorKind {
        match self {
            Self::Source(kind) | Self::Destination(kind) => kind,
        }
    }
}

/// A domain operation plus the path and original I/O classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure<O> {
    pub operation: O,
    pub path: PathBuf,
    pub error_kind: io::ErrorKind,
    pub detail: String,
}

impl<O> Failure<O> {
    pub fn from_io(operation: O, path: &Path, error: io::Error) -> Self {
        Self {
            operation,
            path: path.to_path_buf(),
            error_kind: error.kind(),
            detail: error.to_string(),
        }
    }

    pub fn message(operation: O, path: &Path, detail: impl Into<String>) -> Self {
        Self::message_with_kind(operation, path, io::ErrorKind::InvalidData, detail)
    }

    pub fn message_with_kind(
        operation: O,
        path: &Path,
        error_kind: io::ErrorKind,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            path: path.to_path_buf(),
            error_kind,
            detail: detail.into(),
        }
    }
}

impl<O: fmt::Display> fmt::Display for Failure<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Could not {} at “{}”: {}",
            self.operation,
            self.path.display(),
            self.detail
        )
    }
}

impl<O: fmt::Debug + fmt::Display> std::error::Error for Failure<O> {}

/// Injectable filesystem boundary. Methods default to `Unsupported` so small
/// fault-injection fakes only need to implement the operations under test.
pub trait Backend {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(unsupported("read"))
    }

    fn read_to_string(&self, _path: &Path) -> io::Result<String> {
        Err(unsupported("read text"))
    }

    /// Read at most `maximum` bytes. The default keeps small injectable
    /// backends simple; the host filesystem override rejects by metadata and
    /// streams only `maximum + 1` bytes so an untrusted file cannot force an
    /// unbounded allocation.
    fn read_bounded(&self, path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
        let bytes = self.read(path)?;
        if bytes.len() > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file exceeds the configured size limit",
            ));
        }
        Ok(bytes)
    }

    /// Read a bounded private state file without following a substituted link.
    /// The host implementation also rejects multiply linked files.
    fn read_bounded_no_follow(&self, path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
        self.read_bounded(path, maximum)
    }

    /// Stream a bounded fingerprint from a private regular file without
    /// following a substituted final link. Host files are never allocated in
    /// full; small injectable backends may use their bounded read fallback.
    fn fingerprint_bounded_no_follow(
        &self,
        path: &Path,
        maximum: u64,
    ) -> io::Result<FileFingerprint> {
        let maximum = usize::try_from(maximum).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "fingerprint bound is too large",
            )
        })?;
        let bytes = self.read_bounded_no_follow(path, maximum)?;
        Ok(FileFingerprint {
            byte_len: bytes.len() as u64,
            sha256: Sha256::digest(&bytes).into(),
        })
    }

    fn write_atomic(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Err(unsupported("write atomically"))
    }

    fn write_atomic_private(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Err(unsupported("write private data atomically"))
    }

    /// Create a private file without ever replacing an existing path.
    fn write_new_private(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Err(unsupported("write new private data"))
    }

    /// Stream into a fresh private file without replacing an existing entry.
    fn write_new_private_stream(
        &self,
        _path: &Path,
        _source: &mut dyn io::Read,
        _maximum: u64,
    ) -> io::Result<FileFingerprint> {
        Err(unsupported("stream new private data"))
    }

    fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
        Err(unsupported("create directory"))
    }

    /// Create or repair the final private state directory as owner-only.
    fn create_dir_all_private(&self, path: &Path) -> io::Result<()> {
        self.create_dir_all(path)
    }

    fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
        Err(unsupported("rename"))
    }

    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        Err(unsupported("remove file"))
    }

    /// Remove a file and durably record the directory-entry change.
    ///
    /// Injectable backends may model this as a normal removal; the host
    /// implementation syncs the parent directory before reporting success.
    fn remove_file_durable(&self, path: &Path) -> io::Result<()> {
        self.remove_file(path)
    }

    fn remove_dir_all(&self, _path: &Path) -> io::Result<()> {
        Err(unsupported("remove directory"))
    }

    fn copy(&self, _source: &Path, _destination: &Path) -> io::Result<u64> {
        Err(unsupported("copy"))
    }
}

fn unsupported(operation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("storage backend does not implement {operation}"),
    )
}
