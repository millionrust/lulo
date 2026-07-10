use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Attach,
    CreateFolder,
    CreateNote,
    DeleteFolder,
    DeleteNote,
    LoadPins,
    LoadNote,
    LoadSort,
    RenameFolder,
    SaveNote,
    SavePins,
    SaveSort,
}

impl Operation {
    fn label(self) -> &'static str {
        match self {
            Self::Attach => "attach image",
            Self::CreateFolder => "create folder",
            Self::CreateNote => "create note",
            Self::DeleteFolder => "delete folder",
            Self::DeleteNote => "delete note",
            Self::LoadPins => "load pinned notes",
            Self::LoadNote => "load note",
            Self::LoadSort => "load sort order",
            Self::RenameFolder => "rename folder",
            Self::SaveNote => "save note",
            Self::SavePins => "save pinned notes",
            Self::SaveSort => "save sort order",
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

    pub(crate) fn message(operation: Operation, path: &Path, detail: impl Into<String>) -> Self {
        Self {
            operation,
            path: path.to_path_buf(),
            error_kind: io::ErrorKind::Other,
            detail: detail.into(),
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
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, source: &Path, destination: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;
    fn copy(&self, source: &Path, destination: &Path) -> io::Result<u64>;
}

pub(crate) struct RealStorage;

impl Storage for RealStorage {
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
        let mut destination_created = false;
        let result = (|| {
            let mut source_file = File::open(source)?;
            let mut destination_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)?;
            destination_created = true;
            let copied = io::copy(&mut source_file, &mut destination_file)?;
            destination_file.sync_all()?;
            destination_file.set_permissions(source_file.metadata()?.permissions())?;
            Ok(copied)
        })();

        if result.is_err() && destination_created {
            // `create_new` proves this invocation created the destination, so
            // cleanup cannot remove a pre-existing attachment.
            let _ = std::fs::remove_file(destination);
        }
        result
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
        .unwrap_or("note");
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
) -> Result<String, Failure> {
    storage
        .read_to_string(path)
        .map_err(|error| Failure::from_io(operation, path, error))
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

pub(crate) fn create_dir(
    storage: &impl Storage,
    operation: Operation,
    path: &Path,
) -> Result<(), Failure> {
    storage
        .create_dir_all(path)
        .map_err(|error| Failure::from_io(operation, path, error))
}

pub(crate) fn rename(
    storage: &impl Storage,
    operation: Operation,
    source: &Path,
    destination: &Path,
) -> Result<(), Failure> {
    storage
        .rename(source, destination)
        .map_err(|error| Failure::from_io(operation, source, error))
}

pub(crate) fn remove_file(
    storage: &impl Storage,
    operation: Operation,
    path: &Path,
) -> Result<(), Failure> {
    storage
        .remove_file(path)
        .map_err(|error| Failure::from_io(operation, path, error))
}

pub(crate) fn remove_dir_all(
    storage: &impl Storage,
    operation: Operation,
    path: &Path,
) -> Result<(), Failure> {
    storage
        .remove_dir_all(path)
        .map_err(|error| Failure::from_io(operation, path, error))
}

pub(crate) fn copy(
    storage: &impl Storage,
    operation: Operation,
    source: &Path,
    destination: &Path,
) -> Result<(), Failure> {
    storage
        .copy(source, destination)
        .map(|_| ())
        .map_err(|error| Failure::from_io(operation, source, error))
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
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.calls
                .borrow_mut()
                .push(format!("read:{}", path.display()));
            Ok(String::new())
        }

        fn write_atomic(&self, path: &Path, _contents: &[u8]) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("write:{}", path.display()));
            Err(self.failure())
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("create:{}", path.display()));
            Err(self.failure())
        }

        fn rename(&self, source: &Path, destination: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push(format!(
                "rename:{}:{}",
                source.display(),
                destination.display()
            ));
            Err(self.failure())
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("remove_file:{}", path.display()));
            Err(self.failure())
        }

        fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("remove_dir:{}", path.display()));
            Err(self.failure())
        }

        fn copy(&self, source: &Path, destination: &Path) -> io::Result<u64> {
            self.calls.borrow_mut().push(format!(
                "copy:{}:{}",
                source.display(),
                destination.display()
            ));
            Err(self.failure())
        }
    }

    #[test]
    fn atomic_write_replaces_the_target_without_leaving_a_temp_file() {
        let root = std::env::temp_dir().join(format!(
            "rmac-notes-atomic-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("note.md");
        std::fs::write(&target, "before").unwrap();

        atomic_write(&target, b"after").unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "after");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn injected_atomic_write_failure_preserves_operation_and_retry_signal() {
        let storage = FailingStorage {
            error: io::ErrorKind::PermissionDenied,
            calls: RefCell::new(Vec::new()),
        };

        let failure = write(
            &storage,
            Operation::SaveNote,
            Path::new("note.md"),
            "contents",
        )
        .unwrap_err();

        assert_eq!(failure.operation, Operation::SaveNote);
        assert_eq!(failure.error_kind, io::ErrorKind::PermissionDenied);
        assert_eq!(&*storage.calls.borrow(), &["write:note.md"]);
    }

    #[test]
    fn injected_rename_and_delete_failures_remain_distinct() {
        let storage = FailingStorage {
            error: io::ErrorKind::PermissionDenied,
            calls: RefCell::new(Vec::new()),
        };

        let rename_failure = rename(
            &storage,
            Operation::RenameFolder,
            Path::new("old"),
            Path::new("new"),
        )
        .unwrap_err();
        let note_failure =
            remove_file(&storage, Operation::DeleteNote, Path::new("note.md")).unwrap_err();
        let folder_failure =
            remove_dir_all(&storage, Operation::DeleteFolder, Path::new("folder")).unwrap_err();

        assert_eq!(rename_failure.operation, Operation::RenameFolder);
        assert_eq!(note_failure.operation, Operation::DeleteNote);
        assert_eq!(folder_failure.operation, Operation::DeleteFolder);
        assert_eq!(
            &*storage.calls.borrow(),
            &["rename:old:new", "remove_file:note.md", "remove_dir:folder"]
        );
    }

    #[test]
    fn attachment_copy_never_overwrites_an_existing_destination() {
        let root = std::env::temp_dir().join(format!(
            "rmac-notes-copy-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.png");
        let destination = root.join("destination.png");
        std::fs::write(&source, "new image").unwrap();
        std::fs::write(&destination, "existing image").unwrap();

        let failure = copy(&RealStorage, Operation::Attach, &source, &destination).unwrap_err();

        assert_eq!(failure.error_kind, io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "existing image"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
