use std::fmt;
use std::path::Path;

pub(crate) use rmac_storage::{Backend as Storage, FileSystem as RealStorage};
pub(crate) type Failure = rmac_storage::Failure<Operation>;

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

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
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
        })
    }
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
    use std::io;

    struct FailingStorage;

    impl FailingStorage {
        fn failure() -> io::Error {
            io::Error::new(io::ErrorKind::PermissionDenied, "injected storage failure")
        }
    }

    impl Storage for FailingStorage {
        fn write_atomic(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
            Err(Self::failure())
        }

        fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
            Err(Self::failure())
        }

        fn remove_file(&self, _path: &Path) -> io::Result<()> {
            Err(Self::failure())
        }

        fn remove_dir_all(&self, _path: &Path) -> io::Result<()> {
            Err(Self::failure())
        }
    }

    #[test]
    fn injected_atomic_write_failure_preserves_operation_and_retry_signal() {
        let failure = write(
            &FailingStorage,
            Operation::SaveNote,
            Path::new("note.md"),
            "contents",
        )
        .unwrap_err();

        assert_eq!(failure.operation, Operation::SaveNote);
        assert_eq!(failure.error_kind, io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn injected_rename_and_delete_failures_remain_distinct() {
        let rename_failure = rename(
            &FailingStorage,
            Operation::RenameFolder,
            Path::new("old"),
            Path::new("new"),
        )
        .unwrap_err();
        let note_failure =
            remove_file(&FailingStorage, Operation::DeleteNote, Path::new("note.md")).unwrap_err();
        let folder_failure = remove_dir_all(
            &FailingStorage,
            Operation::DeleteFolder,
            Path::new("folder"),
        )
        .unwrap_err();

        assert_eq!(rename_failure.operation, Operation::RenameFolder);
        assert_eq!(note_failure.operation, Operation::DeleteNote);
        assert_eq!(folder_failure.operation, Operation::DeleteFolder);
    }
}
