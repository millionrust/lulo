use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    CreateConfigDirectory,
    LoadColumns,
    ResolveConfigPath,
    SaveColumns,
}

impl Operation {
    fn label(self) -> &'static str {
        match self {
            Self::CreateConfigDirectory => "create the preferences directory",
            Self::LoadColumns => "load column preferences",
            Self::ResolveConfigPath => "resolve the preferences path",
            Self::SaveColumns => "save column preferences",
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
            error_kind: io::ErrorKind::InvalidData,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Could not {} at “{}”: {}",
            self.operation.label(),
            self.path.display(),
            self.detail
        )
    }
}

pub(crate) trait Storage {
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()>;
}

pub(crate) struct RealStorage;

impl Storage for RealStorage {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        atomic_write(path, contents)
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences path has no parent directory",
        )
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("columns");
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

pub(crate) fn load_optional(
    storage: &impl Storage,
    path: &Path,
) -> Result<Option<String>, Failure> {
    match storage.read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Failure::from_io(Operation::LoadColumns, path, error)),
    }
}

pub(crate) fn save(
    storage: &impl Storage,
    path: &Path,
    contents: impl AsRef<[u8]>,
) -> Result<(), Failure> {
    let parent = path.parent().ok_or_else(|| {
        Failure::message(
            Operation::ResolveConfigPath,
            path,
            "preferences path has no parent directory",
        )
    })?;
    storage
        .create_dir_all(parent)
        .map_err(|error| Failure::from_io(Operation::CreateConfigDirectory, parent, error))?;
    storage
        .write_atomic(path, contents.as_ref())
        .map_err(|error| Failure::from_io(Operation::SaveColumns, path, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FailingStorage {
        error: io::ErrorKind,
        create_succeeds: bool,
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
            Err(self.failure())
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("create:{}", path.display()));
            if self.create_succeeds {
                Ok(())
            } else {
                Err(self.failure())
            }
        }

        fn write_atomic(&self, path: &Path, _contents: &[u8]) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("write:{}", path.display()));
            Err(self.failure())
        }
    }

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-activity-monitor-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn atomic_save_replaces_columns_without_leaving_a_temp_file() {
        let root = temp_root();
        std::fs::create_dir(&root).unwrap();
        let target = root.join("columns.txt");
        std::fs::write(&target, "pid,name").unwrap();

        save(&RealStorage, &target, "name,cpu").unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "name,cpu");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_preferences_are_a_normal_first_launch() {
        let storage = FailingStorage {
            error: io::ErrorKind::NotFound,
            create_succeeds: false,
            calls: RefCell::new(Vec::new()),
        };

        assert_eq!(
            load_optional(&storage, Path::new("columns.txt")).unwrap(),
            None
        );
    }

    #[test]
    fn injected_load_directory_and_write_failures_remain_distinct() {
        let load_storage = FailingStorage {
            error: io::ErrorKind::PermissionDenied,
            create_succeeds: false,
            calls: RefCell::new(Vec::new()),
        };
        let directory_storage = FailingStorage {
            error: io::ErrorKind::PermissionDenied,
            create_succeeds: false,
            calls: RefCell::new(Vec::new()),
        };
        let write_storage = FailingStorage {
            error: io::ErrorKind::PermissionDenied,
            create_succeeds: true,
            calls: RefCell::new(Vec::new()),
        };

        let load_failure = load_optional(&load_storage, Path::new("columns.txt")).unwrap_err();
        let directory_failure =
            save(&directory_storage, Path::new("config/columns.txt"), "name").unwrap_err();
        let write_failure =
            save(&write_storage, Path::new("config/columns.txt"), "name").unwrap_err();

        assert_eq!(load_failure.operation, Operation::LoadColumns);
        assert_eq!(
            directory_failure.operation,
            Operation::CreateConfigDirectory
        );
        assert_eq!(write_failure.operation, Operation::SaveColumns);
        assert_eq!(load_failure.error_kind, io::ErrorKind::PermissionDenied);
        assert_eq!(
            directory_failure.error_kind,
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(write_failure.error_kind, io::ErrorKind::PermissionDenied);
    }
}
