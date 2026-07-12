//! Shared durable filesystem primitives for rmac applications.
//!
//! Apps retain their domain-specific operation enums and user-facing errors;
//! this crate owns the mechanics that must remain identical everywhere:
//! adjacent-temp atomic replacement, parent-directory durability, no-clobber
//! copies, and cleanup of partial files.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

    fn write_atomic(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Err(unsupported("write atomically"))
    }

    fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
        Err(unsupported("create directory"))
    }

    fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
        Err(unsupported("rename"))
    }

    fn remove_file(&self, _path: &Path) -> io::Result<()> {
        Err(unsupported("remove file"))
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

/// Real host filesystem implementation.
pub struct FileSystem;

impl Backend for FileSystem {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        atomic_write(path, contents)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn rename(&self, source: &Path, destination: &Path) -> io::Result<()> {
        std::fs::rename(source, destination)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(path)
    }

    fn copy(&self, source: &Path, destination: &Path) -> io::Result<u64> {
        copy_no_clobber(source, destination)
    }
}

/// Atomically replace `path` using a same-directory temporary file.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no parent directory",
        )
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    let temporary = parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Atomically replace a private state file.
///
/// On Unix, the temporary file is created as `0600` before any content is
/// written and remains `0600` after replacement, regardless of umask or prior
/// target permissions. Use this for notification content, recent-item data,
/// and other user-private state—not user-authored documents whose permissions
/// should be preserved.
pub fn atomic_write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "target has no parent directory",
        )
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("private-data");
    let temporary = parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));

    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Copy to a newly created destination and never overwrite an existing path.
/// A copy/sync/permission failure removes only the destination created by this
/// invocation, leaving no partial attachment behind.
pub fn copy_no_clobber(source: &Path, destination: &Path) -> io::Result<u64> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "destination has no parent directory",
        )
    })?;
    let mut destination_created = false;
    let result = (|| {
        let mut source_file = File::open(source)?;
        let mut destination_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        destination_created = true;
        let copied = io::copy(&mut source_file, &mut destination_file)?;
        destination_file.set_permissions(source_file.metadata()?.permissions())?;
        destination_file.sync_all()?;
        drop(destination_file);
        File::open(parent)?.sync_all()?;
        Ok(copied)
    })();

    if result.is_err() && destination_created {
        let _ = std::fs::remove_file(destination);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-storage-{label}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn atomic_write_replaces_target_without_leaving_a_temp_file() {
        let root = temp_root("atomic");
        std::fs::create_dir(&root).unwrap();
        let target = root.join("settings.txt");
        std::fs::write(&target, "before").unwrap();

        atomic_write(&target, b"after").unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "after");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_atomic_write_never_inherits_a_public_target_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = temp_root("private-atomic");
        std::fs::create_dir(&root).unwrap();
        let target = root.join("notifications.json");
        std::fs::write(&target, "before").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

        atomic_write_private(&target, b"private notification").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"private notification");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copy_never_overwrites_an_existing_destination() {
        let root = temp_root("no-clobber");
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.png");
        let destination = root.join("destination.png");
        std::fs::write(&source, "new image").unwrap();
        std::fs::write(&destination, "existing image").unwrap();

        let error = copy_no_clobber(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "existing image"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_copy_removes_its_partial_destination() {
        let root = temp_root("partial-copy");
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source-directory");
        let destination = root.join("destination");
        std::fs::create_dir(&source).unwrap();

        copy_no_clobber(&source, &destination).unwrap_err();

        assert!(!destination.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_failure_preserves_operation_path_and_kind() {
        let failure = Failure::from_io(
            "save settings",
            Path::new("settings.json"),
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        );

        assert_eq!(failure.operation, "save settings");
        assert_eq!(failure.path, Path::new("settings.json"));
        assert_eq!(failure.error_kind, io::ErrorKind::PermissionDenied);
        assert!(failure.to_string().contains("save settings"));
    }
}
