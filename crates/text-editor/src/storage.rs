use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    LoadDocument,
    LoadRecovery,
    RemoveRecovery,
    SaveDocument,
    SaveRecovery,
}

impl Operation {
    fn label(self) -> &'static str {
        match self {
            Self::LoadDocument => "open document",
            Self::LoadRecovery => "load recovery data",
            Self::RemoveRecovery => "remove recovery data",
            Self::SaveDocument => "save document",
            Self::SaveRecovery => "save recovery data",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) operation: Operation,
    pub(crate) path: PathBuf,
    pub(crate) error_kind: io::ErrorKind,
    pub(crate) detail: String,
}

impl Failure {
    fn from_io(operation: Operation, path: &Path, error: io::Error) -> Self {
        Self {
            operation,
            path: path.to_path_buf(),
            error_kind: error.kind(),
            detail: error.to_string(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self
            .path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy();
        write!(
            f,
            "Could not {} “{}”: {}",
            self.operation.label(),
            name,
            self.detail
        )
    }
}

pub(crate) trait Storage {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
}

pub(crate) struct RealStorage;

impl Storage for RealStorage {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        atomic_write(path, contents)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
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
        .unwrap_or("document");
    let temporary = parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
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

pub(crate) fn read(
    storage: &impl Storage,
    operation: Operation,
    path: &Path,
) -> Result<Vec<u8>, Failure> {
    storage
        .read(path)
        .map_err(|error| Failure::from_io(operation, path, error))
}

pub(crate) fn load_recovery(
    storage: &impl Storage,
    path: &Path,
) -> Result<Option<String>, Failure> {
    match read(storage, Operation::LoadRecovery, path) {
        Ok(bytes) if bytes.is_empty() => Ok(None),
        Ok(bytes) => String::from_utf8(bytes).map(Some).map_err(|error| {
            Failure::from_io(
                Operation::LoadRecovery,
                path,
                io::Error::new(io::ErrorKind::InvalidData, error),
            )
        }),
        Err(failure) if failure.error_kind == io::ErrorKind::NotFound => Ok(None),
        Err(failure) => Err(failure),
    }
}

pub(crate) fn write(
    storage: &impl Storage,
    operation: Operation,
    path: &Path,
    contents: impl AsRef<[u8]>,
) -> Result<(), Failure> {
    storage
        .write_atomic(path, contents.as_ref())
        .map_err(|error| Failure::from_io(operation, path, error))
}

pub(crate) fn remove_recovery(storage: &impl Storage, path: &Path) -> Result<(), Failure> {
    match storage.remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Failure::from_io(Operation::RemoveRecovery, path, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FailingStorage {
        error: io::ErrorKind,
        calls: RefCell<Vec<String>>,
    }

    impl FailingStorage {
        fn failure(&self) -> io::Error {
            io::Error::new(self.error, "injected storage failure")
        }
    }

    impl Storage for FailingStorage {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.calls
                .borrow_mut()
                .push(format!("read:{}", path.display()));
            Err(self.failure())
        }

        fn write_atomic(&self, path: &Path, _contents: &[u8]) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("write:{}", path.display()));
            Err(self.failure())
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("remove:{}", path.display()));
            Err(self.failure())
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-text-editor-{label}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn atomic_write_replaces_the_document_without_leaving_a_temp_file() {
        let root = temp_root("atomic");
        std::fs::create_dir(&root).unwrap();
        let target = root.join("document.txt");
        std::fs::write(&target, "before").unwrap();

        atomic_write(&target, b"after").unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "after");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_recovery_data_is_not_an_error() {
        let storage = FailingStorage {
            error: io::ErrorKind::NotFound,
            calls: RefCell::new(Vec::new()),
        };

        assert_eq!(
            load_recovery(&storage, Path::new("recovery.txt")).unwrap(),
            None
        );
        remove_recovery(&storage, Path::new("recovery.txt")).unwrap();
        assert_eq!(
            &*storage.calls.borrow(),
            &["read:recovery.txt", "remove:recovery.txt"]
        );
    }

    #[test]
    fn recovery_write_and_remove_failures_remain_distinct() {
        let storage = FailingStorage {
            error: io::ErrorKind::PermissionDenied,
            calls: RefCell::new(Vec::new()),
        };

        let write_failure = write(
            &storage,
            Operation::SaveRecovery,
            Path::new("recovery.txt"),
            "draft",
        )
        .unwrap_err();
        let remove_failure = remove_recovery(&storage, Path::new("recovery.txt")).unwrap_err();

        assert_eq!(write_failure.operation, Operation::SaveRecovery);
        assert_eq!(remove_failure.operation, Operation::RemoveRecovery);
        assert_eq!(write_failure.error_kind, io::ErrorKind::PermissionDenied);
        assert_eq!(remove_failure.error_kind, io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn document_read_and_write_failures_preserve_their_operations() {
        let storage = FailingStorage {
            error: io::ErrorKind::PermissionDenied,
            calls: RefCell::new(Vec::new()),
        };

        let read_failure =
            read(&storage, Operation::LoadDocument, Path::new("document.txt")).unwrap_err();
        let write_failure = write(
            &storage,
            Operation::SaveDocument,
            Path::new("document.txt"),
            "draft",
        )
        .unwrap_err();

        assert_eq!(read_failure.operation, Operation::LoadDocument);
        assert_eq!(write_failure.operation, Operation::SaveDocument);
        assert_eq!(read_failure.error_kind, io::ErrorKind::PermissionDenied);
        assert_eq!(write_failure.error_kind, io::ErrorKind::PermissionDenied);
    }
}
