use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) use rmac_storage::{Backend as Storage, FileSystem as RealStorage};
pub(crate) type Failure = rmac_storage::Failure<Operation>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    CreateRecoveryDirectory,
    ExportPdf,
    LoadDocument,
    LoadRecovery,
    RemoveRecovery,
    ReadbackDocument,
    ResolveRecoveryPath,
    SaveDocument,
    SaveRecovery,
    ValidateDocumentRevision,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CreateRecoveryDirectory => "create recovery storage",
            Self::ExportPdf => "export the document as PDF",
            Self::LoadDocument => "open document",
            Self::LoadRecovery => "load recovery data",
            Self::RemoveRecovery => "remove recovery data",
            Self::ReadbackDocument => "read back the saved document",
            Self::ResolveRecoveryPath => "resolve the recovery path",
            Self::SaveDocument => "save document",
            Self::SaveRecovery => "save recovery data",
            Self::ValidateDocumentRevision => "validate the document revision",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SaveDocumentError {
    Conflict,
    Io(Failure),
    ReadbackMismatch,
}

impl fmt::Display for SaveDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Conflict => "the document changed outside Text Editor; reload it or save a copy",
            Self::Io(_) => "the document could not be written or verified",
            Self::ReadbackMismatch => "the saved document did not match during readback",
        })
    }
}

impl std::error::Error for SaveDocumentError {}

#[cfg(test)]
pub(crate) fn read(
    storage: &impl Storage,
    operation: Operation,
    path: &Path,
) -> Result<Vec<u8>, Failure> {
    storage
        .read(path)
        .map_err(|error| Failure::from_io(operation, path, error))
}

pub(crate) fn read_bounded(
    storage: &impl Storage,
    operation: Operation,
    path: &Path,
    maximum: usize,
) -> Result<Vec<u8>, Failure> {
    storage
        .read_bounded(path, maximum)
        .map_err(|error| Failure::from_io(operation, path, error))
}

pub(crate) fn load_recovery(
    storage: &impl Storage,
    path: &Path,
) -> Result<Option<String>, Failure> {
    match read_bounded(
        storage,
        Operation::LoadRecovery,
        path,
        crate::document::MAX_DOCUMENT_BYTES,
    ) {
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

/// Replace an opened document only when its complete bytes still match the
/// revision retained at open/last-save, then require exact authoritative
/// readback. `None` is reserved for a chooser-approved new destination.
pub(crate) fn write_document_if_unchanged(
    storage: &impl Storage,
    path: &Path,
    expected: Option<&[u8]>,
    contents: &[u8],
) -> Result<(), SaveDocumentError> {
    if let Some(expected) = expected {
        let current = read_bounded(
            storage,
            Operation::ValidateDocumentRevision,
            path,
            crate::document::MAX_DOCUMENT_BYTES,
        )
        .map_err(SaveDocumentError::Io)?;
        if current != expected {
            return Err(SaveDocumentError::Conflict);
        }
    }
    write(storage, Operation::SaveDocument, path, contents).map_err(SaveDocumentError::Io)?;
    let readback = read_bounded(
        storage,
        Operation::ReadbackDocument,
        path,
        crate::document::MAX_DOCUMENT_BYTES,
    )
    .map_err(SaveDocumentError::Io)?;
    if readback != contents {
        return Err(SaveDocumentError::ReadbackMismatch);
    }
    Ok(())
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
    storage
        .write_atomic_private(path, contents.as_ref())
        .map_err(|error| Failure::from_io(operation, path, error))
}

pub(crate) fn remove_recovery(storage: &impl Storage, path: &Path) -> Result<(), Failure> {
    match storage.remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Failure::from_io(Operation::RemoveRecovery, path, error)),
    }
}

pub(crate) struct LegacyDraft {
    pub(crate) content: String,
    pub(crate) paths: Vec<PathBuf>,
}

pub(crate) struct LoadedLegacyDrafts {
    pub(crate) drafts: Vec<LegacyDraft>,
    pub(crate) warning: Option<Failure>,
}

/// Load every distinct legacy draft without mutating either source. A caller
/// can then create and verify a versioned record before removing the exact raw
/// paths represented by that record.
pub(crate) fn load_legacy_recovery_drafts(
    storage: &impl Storage,
    primary: &Path,
    legacy: &Path,
) -> LoadedLegacyDrafts {
    let mut loaded = LoadedLegacyDrafts {
        drafts: Vec::new(),
        warning: None,
    };
    for path in [primary, legacy] {
        if loaded
            .drafts
            .iter()
            .flat_map(|draft| &draft.paths)
            .any(|known| known == path)
        {
            continue;
        }
        match load_recovery(storage, path) {
            Ok(Some(content)) => {
                if let Some(existing) = loaded
                    .drafts
                    .iter_mut()
                    .find(|draft| draft.content == content)
                {
                    existing.paths.push(path.to_path_buf());
                } else {
                    loaded.drafts.push(LegacyDraft {
                        content,
                        paths: vec![path.to_path_buf()],
                    });
                }
            }
            Ok(None) => {}
            Err(failure) => {
                loaded.warning.get_or_insert(failure);
            }
        }
    }
    loaded
}

/// Attempt every cleanup path so one inaccessible file never prevents removal
/// of another copy. The first typed failure remains visible to the caller.
#[cfg(test)]
pub(crate) fn remove_recoveries(
    storage: &impl Storage,
    primary: &Path,
    legacy: Option<&Path>,
) -> Result<(), Failure> {
    let mut paths = vec![primary.to_path_buf()];
    if let Some(legacy) = legacy.filter(|path| *path != primary) {
        paths.push(legacy.to_path_buf());
    }
    remove_recovery_paths(storage, &paths)
}

/// Attempt every path and retain the first failure, so a stale/inaccessible
/// legacy record cannot prevent cleanup of the current private draft.
pub(crate) fn remove_recovery_paths(
    storage: &impl Storage,
    paths: &[PathBuf],
) -> Result<(), Failure> {
    let mut first_failure = None;
    for (index, path) in paths.iter().enumerate() {
        if paths[..index].contains(path) {
            continue;
        }
        if let Err(failure) = remove_recovery(storage, path) {
            first_failure.get_or_insert(failure);
        }
    }
    match first_failure {
        Some(failure) => Err(failure),
        None => Ok(()),
    }
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

        fn write_atomic_private(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
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
        ignore_write: bool,
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
            if self.ignore_write {
                return Ok(());
            }
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), contents.to_vec());
            Ok(())
        }

        fn write_atomic_private(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
            self.write_atomic(path, contents)
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
    fn legacy_recovery_collection_is_read_only_and_deduplicates_content() {
        let primary = Path::new("state/recovery.txt");
        let legacy = Path::new("tmp/recovery.txt");
        let storage = MemoryStorage::with_file(primary, "unsaved draft");
        storage
            .files
            .borrow_mut()
            .insert(legacy.to_path_buf(), b"unsaved draft".to_vec());

        let loaded = load_legacy_recovery_drafts(&storage, primary, legacy);

        assert_eq!(loaded.drafts.len(), 1);
        assert_eq!(loaded.drafts[0].content, "unsaved draft");
        assert_eq!(loaded.drafts[0].paths, [primary, legacy]);
        assert!(loaded.warning.is_none());
        assert!(storage.files.borrow().contains_key(primary));
        assert!(storage.files.borrow().contains_key(legacy));
    }

    #[test]
    fn distinct_legacy_drafts_are_both_preserved() {
        let primary = Path::new("state/recovery.txt");
        let legacy = Path::new("tmp/recovery.txt");
        let storage = MemoryStorage::with_file(primary, "newer draft");
        storage
            .files
            .borrow_mut()
            .insert(legacy.to_path_buf(), b"older distinct draft".to_vec());

        let loaded = load_legacy_recovery_drafts(&storage, primary, legacy);

        assert_eq!(loaded.drafts.len(), 2);
        assert_eq!(loaded.drafts[0].content, "newer draft");
        assert_eq!(loaded.drafts[1].content, "older distinct draft");
        assert!(loaded.warning.is_none());
    }

    #[test]
    fn identical_legacy_and_primary_paths_never_delete_the_only_copy() {
        let path = Path::new("state/recovery.txt");
        let storage = MemoryStorage::with_file(path, "unsaved draft");

        let loaded = load_legacy_recovery_drafts(&storage, path, path);

        assert_eq!(loaded.drafts.len(), 1);
        assert_eq!(loaded.drafts[0].content, "unsaved draft");
        assert_eq!(loaded.drafts[0].paths, [path]);
        assert_eq!(storage.files.borrow().get(path).unwrap(), b"unsaved draft");
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

    #[test]
    fn exact_revision_save_writes_and_requires_exact_readback() {
        let path = Path::new("document.txt");
        let storage = MemoryStorage::with_file(path, "before");
        write_document_if_unchanged(&storage, path, Some(b"before"), b"after").unwrap();
        assert_eq!(storage.files.borrow().get(path).unwrap(), b"after");
    }

    #[test]
    fn external_change_is_refused_without_overwriting_authority() {
        let path = Path::new("document.txt");
        let storage = MemoryStorage::with_file(path, "external edit");
        let error =
            write_document_if_unchanged(&storage, path, Some(b"opened revision"), b"local edit")
                .unwrap_err();
        assert_eq!(error, SaveDocumentError::Conflict);
        assert_eq!(storage.files.borrow().get(path).unwrap(), b"external edit");
    }

    #[test]
    fn reviewed_overwrite_stops_when_the_external_revision_changes_again() {
        let path = Path::new("document.txt");
        let storage = MemoryStorage::with_file(path, "external revision reviewed by user");
        let reviewed = storage.read(path).unwrap();
        storage.files.borrow_mut().insert(
            path.to_path_buf(),
            b"newer edit after confirmation".to_vec(),
        );

        let error = write_document_if_unchanged(
            &storage,
            path,
            Some(&reviewed),
            b"local overwrite request",
        )
        .unwrap_err();

        assert_eq!(error, SaveDocumentError::Conflict);
        assert_eq!(
            storage.files.borrow().get(path).unwrap(),
            b"newer edit after confirmation"
        );
    }

    #[test]
    fn ineffective_atomic_write_fails_authoritative_readback() {
        let path = Path::new("document.txt");
        let mut storage = MemoryStorage::with_file(path, "before");
        storage.ignore_write = true;
        let error =
            write_document_if_unchanged(&storage, path, Some(b"before"), b"after").unwrap_err();
        assert_eq!(error, SaveDocumentError::ReadbackMismatch);
    }
}
