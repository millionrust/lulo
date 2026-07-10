use std::fmt;
use std::io;
use std::path::Path;

pub(crate) use rmac_storage::{Backend as Storage, FileSystem as RealStorage};
pub(crate) type Failure = rmac_storage::Failure<Operation>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    LoadDocument,
    LoadRecovery,
    RemoveRecovery,
    SaveDocument,
    SaveRecovery,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LoadDocument => "open document",
            Self::LoadRecovery => "load recovery data",
            Self::RemoveRecovery => "remove recovery data",
            Self::SaveDocument => "save document",
            Self::SaveRecovery => "save recovery data",
        })
    }
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

    struct FailingStorage {
        error: io::ErrorKind,
    }

    impl FailingStorage {
        fn failure(&self) -> io::Error {
            io::Error::new(self.error, "injected storage failure")
        }
    }

    impl Storage for FailingStorage {
        fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
            Err(self.failure())
        }

        fn write_atomic(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
            Err(self.failure())
        }

        fn remove_file(&self, _path: &Path) -> io::Result<()> {
            Err(self.failure())
        }
    }

    #[test]
    fn missing_recovery_data_is_not_an_error() {
        let storage = FailingStorage {
            error: io::ErrorKind::NotFound,
        };

        assert_eq!(
            load_recovery(&storage, Path::new("recovery.txt")).unwrap(),
            None
        );
        remove_recovery(&storage, Path::new("recovery.txt")).unwrap();
    }

    #[test]
    fn injected_failures_preserve_document_and_recovery_operations() {
        let storage = FailingStorage {
            error: io::ErrorKind::PermissionDenied,
        };

        let read_failure =
            read(&storage, Operation::LoadDocument, Path::new("document.txt")).unwrap_err();
        let write_failure = write(
            &storage,
            Operation::SaveRecovery,
            Path::new("recovery.txt"),
            "draft",
        )
        .unwrap_err();
        let remove_failure = remove_recovery(&storage, Path::new("recovery.txt")).unwrap_err();

        assert_eq!(read_failure.operation, Operation::LoadDocument);
        assert_eq!(write_failure.operation, Operation::SaveRecovery);
        assert_eq!(remove_failure.operation, Operation::RemoveRecovery);
    }
}
