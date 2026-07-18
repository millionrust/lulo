use std::collections::BTreeSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const WRITER_LOCK_NAME: &str = "library.writer.lock";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriterLeaseOperation {
    ValidateRoot,
    CreateRoot,
    ResolveRoot,
    OpenLockFile,
    Lock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriterLeaseErrorKind {
    InvalidRoot,
    Contended,
    Unsupported,
    Io(io::ErrorKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriterLeaseError {
    pub operation: WriterLeaseOperation,
    pub kind: WriterLeaseErrorKind,
}

impl WriterLeaseError {
    fn new(operation: WriterLeaseOperation, kind: WriterLeaseErrorKind) -> Self {
        Self { operation, kind }
    }

    fn io(operation: WriterLeaseOperation, error: io::Error) -> Self {
        Self::new(operation, WriterLeaseErrorKind::Io(error.kind()))
    }
}

impl fmt::Display for WriterLeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            WriterLeaseErrorKind::InvalidRoot => {
                "The Notes library location must be an absolute app-owned directory"
            }
            WriterLeaseErrorKind::Contended => "Another Notes process already owns this library",
            WriterLeaseErrorKind::Unsupported => {
                "This platform cannot provide the required Notes writer lock"
            }
            WriterLeaseErrorKind::Io(_) => {
                "Notes could not establish exclusive ownership of the library"
            }
        })
    }
}

impl std::error::Error for WriterLeaseError {}

/// Exclusive kernel-backed ownership of one canonical Notes library root.
///
/// The lock file is deliberately retained after release. Its existence and
/// contents have no authority; only the live advisory lock on its inode does.
/// Closing the descriptor, including process termination, releases ownership.
pub struct WriterLease {
    root: PathBuf,
    key: PathBuf,
    #[cfg(unix)]
    file: Option<File>,
}

impl fmt::Debug for WriterLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriterLease")
            .finish_non_exhaustive()
    }
}

impl WriterLease {
    pub fn acquire(root: &Path) -> Result<Self, WriterLeaseError> {
        if !root.is_absolute() {
            return Err(WriterLeaseError::new(
                WriterLeaseOperation::ValidateRoot,
                WriterLeaseErrorKind::InvalidRoot,
            ));
        }
        std::fs::create_dir_all(root)
            .map_err(|error| WriterLeaseError::io(WriterLeaseOperation::CreateRoot, error))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| WriterLeaseError::io(WriterLeaseOperation::CreateRoot, error))?;
        }
        let root = std::fs::canonicalize(root)
            .map_err(|error| WriterLeaseError::io(WriterLeaseOperation::ResolveRoot, error))?;
        let key = root.join(WRITER_LOCK_NAME);
        if !reserve_process_key(&key) {
            return Err(WriterLeaseError::new(
                WriterLeaseOperation::Lock,
                WriterLeaseErrorKind::Contended,
            ));
        }

        match open_and_lock(&key) {
            Ok(file) => Ok(Self {
                root,
                key,
                #[cfg(unix)]
                file: Some(file),
            }),
            Err(error) => {
                release_process_key(&key);
                Err(error)
            }
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(test)]
    pub(crate) fn for_fake_backend(root: PathBuf) -> Self {
        debug_assert!(root.is_absolute());
        Self {
            key: root.join(WRITER_LOCK_NAME),
            root,
            #[cfg(unix)]
            file: None,
        }
    }
}

impl Drop for WriterLease {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(file) = self.file.take() {
            unlock(&file);
            drop(file);
        }
        release_process_key(&self.key);
    }
}

fn held_process_keys() -> &'static Mutex<BTreeSet<PathBuf>> {
    static KEYS: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();
    KEYS.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn reserve_process_key(key: &Path) -> bool {
    held_process_keys()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key.to_path_buf())
}

fn release_process_key(key: &Path) {
    held_process_keys()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(key);
}

#[cfg(unix)]
fn open_and_lock(path: &Path) -> Result<File, WriterLeaseError> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW);
    let file = options
        .open(path)
        .map_err(|error| WriterLeaseError::io(WriterLeaseOperation::OpenLockFile, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| WriterLeaseError::io(WriterLeaseOperation::OpenLockFile, error))?;
    // SAFETY: `geteuid` has no preconditions and does not dereference memory.
    let effective_user = unsafe { libc::geteuid() };
    if !metadata.file_type().is_file() || metadata.nlink() != 1 || metadata.uid() != effective_user
    {
        return Err(WriterLeaseError::new(
            WriterLeaseOperation::OpenLockFile,
            WriterLeaseErrorKind::Io(io::ErrorKind::InvalidData),
        ));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| WriterLeaseError::io(WriterLeaseOperation::OpenLockFile, error))?;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(file);
    }
    let error = io::Error::last_os_error();
    if matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
    ) {
        Err(WriterLeaseError::new(
            WriterLeaseOperation::Lock,
            WriterLeaseErrorKind::Contended,
        ))
    } else {
        Err(WriterLeaseError::io(WriterLeaseOperation::Lock, error))
    }
}

#[cfg(not(unix))]
fn open_and_lock(_path: &Path) -> Result<File, WriterLeaseError> {
    Err(WriterLeaseError::new(
        WriterLeaseOperation::Lock,
        WriterLeaseErrorKind::Unsupported,
    ))
}

#[cfg(unix)]
fn unlock(file: &File) {
    use std::os::fd::AsRawFd as _;

    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-notes-writer-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn root_must_be_absolute() {
        let error = WriterLease::acquire(Path::new("relative/library")).unwrap_err();
        assert_eq!(error.kind, WriterLeaseErrorKind::InvalidRoot);
    }

    #[cfg(unix)]
    #[test]
    fn lease_is_private_exclusive_and_reacquirable_after_drop() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = temp_root("exclusive");
        let lease = WriterLease::acquire(&root).unwrap();
        assert_eq!(lease.root(), std::fs::canonicalize(&root).unwrap());
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(root.join(WRITER_LOCK_NAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let contention = WriterLease::acquire(&root).unwrap_err();
        assert_eq!(contention.kind, WriterLeaseErrorKind::Contended);
        drop(lease);

        let reacquired = WriterLease::acquire(&root).unwrap();
        assert!(root.join(WRITER_LOCK_NAME).exists());
        drop(reacquired);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn kernel_lock_rejects_an_independent_descriptor() {
        let root = temp_root("kernel");
        let lease = WriterLease::acquire(&root).unwrap();
        let error = open_and_lock(&root.join(WRITER_LOCK_NAME)).unwrap_err();

        assert_eq!(error.kind, WriterLeaseErrorKind::Contended);
        drop(lease);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn lock_path_symlink_is_rejected_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let root = temp_root("symlink");
        std::fs::create_dir(&root).unwrap();
        let target = root.join("unrelated");
        std::fs::write(&target, b"unchanged").unwrap();
        symlink(&target, root.join(WRITER_LOCK_NAME)).unwrap();

        let error = WriterLease::acquire(&root).unwrap_err();

        assert_eq!(error.operation, WriterLeaseOperation::OpenLockFile);
        assert_eq!(std::fs::read(&target).unwrap(), b"unchanged");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn lock_path_hard_link_is_rejected_before_permissions_change() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = temp_root("hard-link");
        std::fs::create_dir(&root).unwrap();
        let target = root.join("unrelated");
        std::fs::write(&target, b"unchanged").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::hard_link(&target, root.join(WRITER_LOCK_NAME)).unwrap();

        let error = WriterLease::acquire(&root).unwrap_err();

        assert_eq!(error.operation, WriterLeaseOperation::OpenLockFile);
        assert_eq!(std::fs::read(&target).unwrap(), b"unchanged");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
