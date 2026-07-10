use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) use rmac_storage::{Backend as Storage, FileSystem as RealStorage};
pub(crate) type Failure = rmac_storage::Failure<Operation>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    CreateRecoveryDirectory,
    LoadDocument,
    LoadRecovery,
    MigrateRecovery,
    RemoveRecovery,
    ResolveRecoveryPath,
    SaveDocument,
    SaveRecovery,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CreateRecoveryDirectory => "create recovery storage",
            Self::LoadDocument => "open document",
            Self::LoadRecovery => "load recovery data",
            Self::MigrateRecovery => "migrate recovery data",
            Self::RemoveRecovery => "remove recovery data",
            Self::ResolveRecoveryPath => "resolve the recovery path",
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

pub(crate) fn save_recovery(
    storage: &impl Storage,
    operation: Operation,
    path: &Path,
    contents: impl AsRef<[u8]>,
) -> Result<(), Failure> {
    let parent = path.parent().ok_or_else(|| {
        Failure::message(
            Operation::ResolveRecoveryPath,
            path,
            "recovery path has no parent directory",
        )
    })?;
    storage
        .create_dir_all(parent)
        .map_err(|error| Failure::from_io(Operation::CreateRecoveryDirectory, parent, error))?;
    write(storage, operation, path, contents)
}

pub(crate) fn remove_recovery(storage: &impl Storage, path: &Path) -> Result<(), Failure> {
    match storage.remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Failure::from_io(Operation::RemoveRecovery, path, error)),
    }
}

pub(crate) struct LoadedRecovery {
    pub(crate) content: Option<String>,
    pub(crate) legacy_path: Option<PathBuf>,
    pub(crate) warning: Option<Failure>,
}

pub(crate) fn load_migrating_recovery(
    storage: &impl Storage,
    primary: &Path,
    legacy: &Path,
) -> LoadedRecovery {
    match load_recovery(storage, primary) {
        Ok(Some(primary_content)) => match load_recovery(storage, legacy) {
            Ok(Some(legacy_content)) if legacy_content == primary_content => {
                match remove_recovery(storage, legacy) {
                    Ok(()) => LoadedRecovery {
                        content: Some(primary_content),
                        legacy_path: None,
                        warning: None,
                    },
                    Err(failure) => LoadedRecovery {
                        content: Some(primary_content),
                        legacy_path: Some(legacy.to_path_buf()),
                        warning: Some(failure),
                    },
                }
            }
            Ok(Some(_)) => LoadedRecovery {
                content: Some(primary_content),
                legacy_path: Some(legacy.to_path_buf()),
                warning: Some(Failure::message(
                    Operation::LoadRecovery,
                    legacy,
                    "a different legacy draft was preserved; saving or discarding clears both",
                )),
            },
            Ok(None) => LoadedRecovery {
                content: Some(primary_content),
                legacy_path: None,
                warning: None,
            },
            Err(failure) => LoadedRecovery {
                content: Some(primary_content),
                legacy_path: Some(legacy.to_path_buf()),
                warning: Some(failure),
            },
        },
        Ok(None) => match load_recovery(storage, legacy) {
            Ok(Some(content)) => {
                match save_recovery(storage, Operation::MigrateRecovery, primary, &content) {
                    Ok(()) => match remove_recovery(storage, legacy) {
                        Ok(()) => LoadedRecovery {
                            content: Some(content),
                            legacy_path: None,
                            warning: None,
                        },
                        Err(failure) => LoadedRecovery {
                            content: Some(content),
                            legacy_path: Some(legacy.to_path_buf()),
                            warning: Some(failure),
                        },
                    },
                    Err(failure) => LoadedRecovery {
                        content: Some(content),
                        legacy_path: Some(legacy.to_path_buf()),
                        warning: Some(failure),
                    },
                }
            }
            Ok(None) => LoadedRecovery {
                content: None,
                legacy_path: None,
                warning: None,
            },
            Err(failure) => LoadedRecovery {
                content: None,
                legacy_path: Some(legacy.to_path_buf()),
                warning: Some(failure),
            },
        },
        Err(failure) => {
            let legacy_content = load_recovery(storage, legacy).ok().flatten();
            LoadedRecovery {
                content: legacy_content,
                legacy_path: Some(legacy.to_path_buf()),
                warning: Some(failure),
            }
        }
    }
}

/// Attempt every cleanup path so one inaccessible file never prevents removal
/// of another copy. The first typed failure remains visible to the caller.
pub(crate) fn remove_recoveries(
    storage: &impl Storage,
    primary: &Path,
    legacy: Option<&Path>,
) -> Result<(), Failure> {
    let primary_result = remove_recovery(storage, primary);
    let legacy_result = legacy
        .filter(|path| *path != primary)
        .map(|path| remove_recovery(storage, path))
        .transpose();
    primary_result.and(legacy_result.map(|_| ()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

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

    #[derive(Default)]
    struct MemoryStorage {
        files: RefCell<HashMap<PathBuf, Vec<u8>>>,
        fail_write: bool,
        fail_remove: Option<PathBuf>,
    }

    impl MemoryStorage {
        fn with_file(path: &Path, contents: &str) -> Self {
            let storage = Self::default();
            storage
                .files
                .borrow_mut()
                .insert(path.to_path_buf(), contents.as_bytes().to_vec());
            storage
        }
    }

    impl Storage for MemoryStorage {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.files
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
            if self.fail_write {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected migration failure",
                ));
            }
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), contents.to_vec());
            Ok(())
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            if self.fail_remove.as_deref() == Some(path) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected cleanup failure",
                ));
            }
            self.files
                .borrow_mut()
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
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

    #[test]
    fn legacy_recovery_migrates_without_losing_content() {
        let primary = Path::new("state/recovery.txt");
        let legacy = Path::new("tmp/recovery.txt");
        let storage = MemoryStorage::with_file(legacy, "unsaved draft");

        let loaded = load_migrating_recovery(&storage, primary, legacy);

        assert_eq!(loaded.content.as_deref(), Some("unsaved draft"));
        assert!(loaded.legacy_path.is_none());
        assert!(loaded.warning.is_none());
        assert_eq!(
            storage.files.borrow().get(primary).unwrap(),
            b"unsaved draft"
        );
        assert!(!storage.files.borrow().contains_key(legacy));
    }

    #[test]
    fn failed_migration_keeps_the_legacy_draft_recoverable() {
        let primary = Path::new("state/recovery.txt");
        let legacy = Path::new("tmp/recovery.txt");
        let mut storage = MemoryStorage::with_file(legacy, "unsaved draft");
        storage.fail_write = true;

        let loaded = load_migrating_recovery(&storage, primary, legacy);

        assert_eq!(loaded.content.as_deref(), Some("unsaved draft"));
        assert_eq!(loaded.legacy_path.as_deref(), Some(legacy));
        assert_eq!(
            loaded.warning.unwrap().operation,
            Operation::MigrateRecovery
        );
        assert!(!storage.files.borrow().contains_key(primary));
        assert_eq!(
            storage.files.borrow().get(legacy).unwrap(),
            b"unsaved draft"
        );
    }

    #[test]
    fn cleanup_attempts_primary_and_legacy_paths() {
        let primary = Path::new("state/recovery.txt");
        let legacy = Path::new("tmp/recovery.txt");
        let mut storage = MemoryStorage::with_file(primary, "new draft");
        storage
            .files
            .borrow_mut()
            .insert(legacy.to_path_buf(), b"old draft".to_vec());
        storage.fail_remove = Some(primary.to_path_buf());

        let failure = remove_recoveries(&storage, primary, Some(legacy)).unwrap_err();

        assert_eq!(failure.operation, Operation::RemoveRecovery);
        assert!(storage.files.borrow().contains_key(primary));
        assert!(!storage.files.borrow().contains_key(legacy));
    }
}
