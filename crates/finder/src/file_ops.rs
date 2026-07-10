use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Copy,
    CreateFolder,
    Delete,
    Move,
    Rename,
    Trash,
}

impl Operation {
    fn verb(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::CreateFolder => "create folder",
            Self::Delete => "delete",
            Self::Move => "move",
            Self::Rename => "rename",
            Self::Trash => "move to Trash",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) operation: Operation,
    pub(crate) source: PathBuf,
    pub(crate) destination: Option<PathBuf>,
    pub(crate) error_kind: io::ErrorKind,
    pub(crate) detail: String,
    pub(crate) recovery_detail: Option<String>,
}

impl Failure {
    fn from_io(
        operation: Operation,
        source: &Path,
        destination: Option<&Path>,
        error: io::Error,
    ) -> Self {
        Self {
            operation,
            source: source.to_path_buf(),
            destination: destination.map(Path::to_path_buf),
            error_kind: error.kind(),
            detail: error.to_string(),
            recovery_detail: None,
        }
    }

    pub(crate) fn message(
        operation: Operation,
        source: &Path,
        destination: Option<&Path>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            source: source.to_path_buf(),
            destination: destination.map(Path::to_path_buf),
            error_kind: io::ErrorKind::Other,
            detail: detail.into(),
            recovery_detail: None,
        }
    }

    fn with_recovery_detail(mut self, detail: impl Into<String>) -> Self {
        self.recovery_detail = Some(detail.into());
        self
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self
            .source
            .file_name()
            .unwrap_or(self.source.as_os_str())
            .to_string_lossy();
        write!(
            f,
            "Could not {} “{}”: {}",
            self.operation.verb(),
            name,
            self.detail
        )?;
        if let Some(recovery) = &self.recovery_detail {
            write!(f, ". {recovery}")?;
        }
        Ok(())
    }
}

pub(crate) trait FileSystem {
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, source: &Path, destination: &Path) -> io::Result<()>;
    fn copy(&self, source: &Path, destination: &Path) -> io::Result<()>;
    fn copy_cancellable(
        &self,
        source: &Path,
        destination: &Path,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        self.copy(source, destination)
    }
    fn remove(&self, path: &Path) -> io::Result<()>;
}

pub(crate) struct RealFileSystem;

impl FileSystem for RealFileSystem {
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir(path)
    }

    fn rename(&self, source: &Path, destination: &Path) -> io::Result<()> {
        std::fs::rename(source, destination)
    }

    fn copy(&self, source: &Path, destination: &Path) -> io::Result<()> {
        crate::copy_item(source, destination)
    }

    fn copy_cancellable(
        &self,
        source: &Path,
        destination: &Path,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        crate::copy_item_cancellable(source, destination, cancel)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        if std::fs::symlink_metadata(path)?.file_type().is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
    }
}

pub(crate) fn create_folder(fs: &impl FileSystem, path: &Path) -> Result<(), Failure> {
    fs.create_dir(path)
        .map_err(|error| Failure::from_io(Operation::CreateFolder, path, None, error))
}

fn copy_cancellable(
    fs: &impl FileSystem,
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<(), Failure> {
    fs.copy_cancellable(source, destination, cancel)
        .map_err(|error| {
            Failure::from_io(Operation::Copy, source, Some(destination), error)
                .with_recovery_detail(format!(
                    "A partial destination may remain at {}",
                    destination.display()
                ))
        })
}

pub(crate) fn delete(fs: &impl FileSystem, path: &Path) -> Result<(), Failure> {
    fs.remove(path)
        .map_err(|error| Failure::from_io(Operation::Delete, path, None, error))
}

pub(crate) fn rename(
    fs: &impl FileSystem,
    source: &Path,
    destination: &Path,
) -> Result<(), Failure> {
    fs.rename(source, destination)
        .map_err(|error| Failure::from_io(Operation::Rename, source, Some(destination), error))
}

/// Move without data loss. Copy-and-delete is allowed only for EXDEV. If
/// deleting the source fails after a successful copy, both paths are retained
/// and reported. Automatically deleting the destination would be unsafe if a
/// different process won a destination-path race.
#[cfg(test)]
pub(crate) fn move_item(
    fs: &impl FileSystem,
    source: &Path,
    destination: &Path,
) -> Result<(), Failure> {
    move_item_cancellable(fs, source, destination, &AtomicBool::new(false))
}

fn move_item_cancellable(
    fs: &impl FileSystem,
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<(), Failure> {
    if cancel.load(Ordering::Acquire) {
        return Err(Failure::from_io(
            Operation::Move,
            source,
            Some(destination),
            io::Error::new(io::ErrorKind::Interrupted, "cancelled"),
        ));
    }
    match fs.rename(source, destination) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() != io::ErrorKind::CrossesDevices => {
            return Err(Failure::from_io(
                Operation::Move,
                source,
                Some(destination),
                error,
            ));
        }
        Err(_) => {}
    }

    copy_cancellable(fs, source, destination, cancel)?;
    if cancel.load(Ordering::Acquire) {
        return Err(Failure::from_io(
            Operation::Move,
            source,
            Some(destination),
            io::Error::new(io::ErrorKind::Interrupted, "cancelled"),
        )
        .with_recovery_detail(format!(
            "The source was retained; a copy may remain at {}",
            destination.display()
        )));
    }
    if let Err(error) = fs.remove(source) {
        return Err(
            Failure::from_io(Operation::Move, source, Some(destination), error)
                .with_recovery_detail(format!(
                    "The source was retained; the completed copy is at {}",
                    destination.display()
                )),
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferKind {
    Copy,
    Move,
}

#[derive(Clone, Debug)]
pub(crate) struct TransferTask {
    pub(crate) kind: TransferKind,
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
}

#[derive(Debug, Default)]
pub(crate) struct TransferReport {
    pub(crate) processed: usize,
    pub(crate) failures: Vec<Failure>,
    pub(crate) unfinished_moves: Vec<PathBuf>,
    pub(crate) cancelled: bool,
}

pub(crate) fn execute_transfers(
    fs: &impl FileSystem,
    tasks: &[TransferTask],
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> TransferReport {
    let mut report = TransferReport::default();
    for (index, task) in tasks.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            report.cancelled = true;
            report.unfinished_moves.extend(
                tasks[index..]
                    .iter()
                    .filter(|task| task.kind == TransferKind::Move)
                    .map(|task| task.source.clone()),
            );
            break;
        }

        let result = match task.kind {
            TransferKind::Copy => copy_cancellable(fs, &task.source, &task.destination, cancel),
            TransferKind::Move => {
                move_item_cancellable(fs, &task.source, &task.destination, cancel)
            }
        };
        report.processed += 1;
        progress(report.processed, tasks.len());

        if let Err(failure) = result {
            if task.kind == TransferKind::Move {
                report.unfinished_moves.push(task.source.clone());
            }
            if failure.error_kind == io::ErrorKind::Interrupted {
                report.cancelled = true;
                // Cancellation can leave a partial or completed destination.
                // Keep the typed recovery detail visible to the user.
                report.failures.push(failure);
                report.unfinished_moves.extend(
                    tasks[index + 1..]
                        .iter()
                        .filter(|task| task.kind == TransferKind::Move)
                        .map(|task| task.source.clone()),
                );
                break;
            }
            report.failures.push(failure);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeFileSystem {
        rename: io::Result<()>,
        copy: io::Result<()>,
        removes: RefCell<Vec<io::Result<()>>>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeFileSystem {
        fn with(
            rename: io::Result<()>,
            copy: io::Result<()>,
            removes: Vec<io::Result<()>>,
        ) -> Self {
            Self {
                rename,
                copy,
                removes: RefCell::new(removes),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl FileSystem for FakeFileSystem {
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("create_dir:{}", path.display()));
            Ok(())
        }

        fn rename(&self, source: &Path, destination: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push(format!(
                "rename:{}:{}",
                source.display(),
                destination.display()
            ));
            self.rename
                .as_ref()
                .map(|_| ())
                .map_err(|error| io::Error::new(error.kind(), error.to_string()))
        }

        fn copy(&self, source: &Path, destination: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push(format!(
                "copy:{}:{}",
                source.display(),
                destination.display()
            ));
            self.copy
                .as_ref()
                .map(|_| ())
                .map_err(|error| io::Error::new(error.kind(), error.to_string()))
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("remove:{}", path.display()));
            self.removes.borrow_mut().remove(0)
        }
    }

    fn error(kind: io::ErrorKind, message: &'static str) -> io::Result<()> {
        Err(io::Error::new(kind, message))
    }

    #[test]
    fn permission_denied_move_never_falls_back_to_copy_and_delete() {
        let fs = FakeFileSystem::with(
            error(io::ErrorKind::PermissionDenied, "denied"),
            Ok(()),
            vec![],
        );

        let failure = move_item(&fs, Path::new("source"), Path::new("dest")).unwrap_err();

        assert_eq!(failure.operation, Operation::Move);
        assert_eq!(failure.error_kind, io::ErrorKind::PermissionDenied);
        assert_eq!(&*fs.calls.borrow(), &["rename:source:dest"]);
    }

    #[test]
    fn cross_device_move_copies_then_removes_the_source() {
        let fs = FakeFileSystem::with(
            error(io::ErrorKind::CrossesDevices, "cross-device"),
            Ok(()),
            vec![Ok(())],
        );

        move_item(&fs, Path::new("source"), Path::new("dest")).unwrap();

        assert_eq!(
            &*fs.calls.borrow(),
            &["rename:source:dest", "copy:source:dest", "remove:source"]
        );
    }

    #[test]
    fn failed_source_removal_retains_both_paths_for_safe_recovery() {
        let fs = FakeFileSystem::with(
            error(io::ErrorKind::CrossesDevices, "cross-device"),
            Ok(()),
            vec![error(io::ErrorKind::PermissionDenied, "source busy")],
        );

        let failure = move_item(&fs, Path::new("source"), Path::new("dest")).unwrap_err();

        assert_eq!(failure.operation, Operation::Move);
        assert_eq!(failure.error_kind, io::ErrorKind::PermissionDenied);
        assert_eq!(
            &*fs.calls.borrow(),
            &["rename:source:dest", "copy:source:dest", "remove:source"]
        );
        assert_eq!(
            failure.recovery_detail.as_deref(),
            Some("The source was retained; the completed copy is at dest")
        );
    }

    #[test]
    fn copy_failure_reports_that_a_partial_destination_may_remain() {
        let fs = FakeFileSystem::with(
            error(io::ErrorKind::CrossesDevices, "cross-device"),
            error(io::ErrorKind::WriteZero, "disk full"),
            vec![],
        );

        let failure = move_item(&fs, Path::new("source"), Path::new("dest")).unwrap_err();

        assert_eq!(failure.operation, Operation::Copy);
        assert_eq!(failure.error_kind, io::ErrorKind::WriteZero);
        assert_eq!(
            failure.recovery_detail.as_deref(),
            Some("A partial destination may remain at dest")
        );
        assert_eq!(
            &*fs.calls.borrow(),
            &["rename:source:dest", "copy:source:dest"]
        );
    }

    #[test]
    fn delete_failure_preserves_the_typed_error() {
        let fs = FakeFileSystem::with(
            Ok(()),
            Ok(()),
            vec![error(io::ErrorKind::PermissionDenied, "read only")],
        );

        let failure = delete(&fs, Path::new("protected")).unwrap_err();

        assert_eq!(failure.operation, Operation::Delete);
        assert_eq!(failure.error_kind, io::ErrorKind::PermissionDenied);
        assert_eq!(&*fs.calls.borrow(), &["remove:protected"]);
    }

    #[test]
    fn cancellation_before_a_batch_preserves_every_unfinished_move() {
        let fs = FakeFileSystem::with(Ok(()), Ok(()), vec![]);
        let cancel = AtomicBool::new(true);
        let tasks = vec![TransferTask {
            kind: TransferKind::Move,
            source: "source".into(),
            destination: "dest".into(),
        }];

        let report = execute_transfers(&fs, &tasks, &cancel, |_, _| {});

        assert!(report.cancelled);
        assert_eq!(report.processed, 0);
        assert_eq!(report.unfinished_moves, vec![PathBuf::from("source")]);
        assert!(fs.calls.borrow().is_empty());
    }

    #[test]
    fn batch_reports_item_progress_in_order() {
        let fs = FakeFileSystem::with(Ok(()), Ok(()), vec![]);
        let cancel = AtomicBool::new(false);
        let tasks = vec![
            TransferTask {
                kind: TransferKind::Copy,
                source: "one".into(),
                destination: "one-copy".into(),
            },
            TransferTask {
                kind: TransferKind::Copy,
                source: "two".into(),
                destination: "two-copy".into(),
            },
        ];
        let mut progress = Vec::new();

        let report = execute_transfers(&fs, &tasks, &cancel, |processed, total| {
            progress.push((processed, total));
        });

        assert_eq!(report.processed, 2);
        assert!(report.failures.is_empty());
        assert_eq!(progress, vec![(1, 2), (2, 2)]);
    }

    #[test]
    fn cancellation_after_cross_device_copy_never_removes_the_source() {
        struct CancelAfterCopy<'a> {
            cancel: &'a AtomicBool,
            calls: RefCell<Vec<&'static str>>,
        }

        impl FileSystem for CancelAfterCopy<'_> {
            fn create_dir(&self, _path: &Path) -> io::Result<()> {
                Ok(())
            }

            fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
                self.calls.borrow_mut().push("rename");
                Err(io::Error::new(
                    io::ErrorKind::CrossesDevices,
                    "cross-device",
                ))
            }

            fn copy(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
                self.calls.borrow_mut().push("copy");
                self.cancel.store(true, Ordering::Release);
                Ok(())
            }

            fn remove(&self, _path: &Path) -> io::Result<()> {
                self.calls.borrow_mut().push("remove");
                Ok(())
            }
        }

        let cancel = AtomicBool::new(false);
        let fs = CancelAfterCopy {
            cancel: &cancel,
            calls: RefCell::new(Vec::new()),
        };
        let tasks = vec![TransferTask {
            kind: TransferKind::Move,
            source: "source".into(),
            destination: "dest".into(),
        }];

        let report = execute_transfers(&fs, &tasks, &cancel, |_, _| {});

        assert!(report.cancelled);
        assert_eq!(report.unfinished_moves, vec![PathBuf::from("source")]);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0]
            .recovery_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("source was retained")));
        assert_eq!(&*fs.calls.borrow(), &["rename", "copy"]);
    }
}
