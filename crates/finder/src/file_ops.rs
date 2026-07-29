use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::operation_journal;

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
    source_retained: bool,
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
            source_retained: true,
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
            source_retained: true,
        }
    }

    fn with_recovery_detail(mut self, detail: impl Into<String>) -> Self {
        self.recovery_detail = Some(detail.into());
        self
    }

    fn with_source_removed(mut self) -> Self {
        self.source_retained = false;
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
        rename_noreplace(source, destination)
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

/// Atomically rename without replacing a destination created by another
/// process after the operation was planned. Ordinary `std::fs::rename`
/// overwrites on Unix, which turns a harmless conflict race into data loss.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rename_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)
}

/// Files is currently packaged only for Linux and developed on macOS. Refuse a
/// move on other targets instead of silently falling back to a clobbering
/// rename with a time-of-check/time-of-use race.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_noreplace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable on this platform",
    ))
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
    move_item_cancellable(fs, source, destination, &AtomicBool::new(false), None)
}

fn move_item_cancellable(
    fs: &impl FileSystem,
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    journal: Option<&operation_journal::Journal>,
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

    let mut ticket = match journal {
        Some(journal) => Some(journal.prepare_move(source, destination).map_err(|error| {
            Failure::from_io(Operation::Move, source, Some(destination), error)
                .with_recovery_detail(
                    "Recovery could not be prepared, so the source was not changed",
                )
        })?),
        None => None,
    };

    if let Some(ticket) = ticket.as_ref() {
        let staging = ticket.staging_destination();
        fs.copy_cancellable(source, &staging, cancel)
            .map_err(|error| {
                Failure::from_io(Operation::Move, source, Some(destination), error)
                    .with_recovery_detail(
                        "The source was retained; a partial recovery copy may remain",
                    )
            })?;
    } else {
        copy_cancellable(fs, source, destination, cancel)?;
    }
    if cancel.load(Ordering::Acquire) {
        let failure = Failure::from_io(
            Operation::Move,
            source,
            Some(destination),
            io::Error::new(io::ErrorKind::Interrupted, "cancelled"),
        );
        return Err(if ticket.is_some() {
            failure.with_recovery_detail(
                "The source was retained; a partial or complete recovery copy may remain",
            )
        } else {
            failure.with_recovery_detail(format!(
                "The source was retained; a copy may remain at {}",
                destination.display()
            ))
        });
    }
    if let Some(ticket) = ticket.as_mut() {
        ticket.mark_destination_complete().map_err(|error| {
            Failure::from_io(Operation::Move, source, Some(destination), error)
                .with_recovery_detail("The source was retained; a completed destination may remain")
        })?;
        let destination_matches = ticket.destination_still_matches().map_err(|error| {
            Failure::from_io(Operation::Move, source, Some(destination), error)
                .with_recovery_detail(
                    "The source was retained because the staged copy could not be rechecked",
                )
        })?;
        if !destination_matches {
            return Err(Failure::message(
                Operation::Move,
                source,
                Some(destination),
                "the staged copy changed before the source could be removed",
            )
            .with_recovery_detail("The source was retained for safe recovery"));
        }
        let source_matches = ticket.source_still_matches().map_err(|error| {
            Failure::from_io(Operation::Move, source, Some(destination), error)
                .with_recovery_detail(
                    "The source was retained because its identity could not be rechecked",
                )
        })?;
        if !source_matches {
            return Err(Failure::message(
                Operation::Move,
                source,
                Some(destination),
                "the source changed while it was being copied",
            )
            .with_recovery_detail(
                "Nothing at the source path was removed; a completed destination remains",
            ));
        }
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
    if let Some(mut ticket) = ticket {
        ticket.mark_source_removed().map_err(|error| {
            Failure::from_io(Operation::Move, source, Some(destination), error)
                .with_source_removed()
                .with_recovery_detail(
                    "The destination is complete, but Files retained a recovery record",
                )
        })?;
        ticket.publish().map_err(|error| {
            Failure::from_io(Operation::Move, source, Some(destination), error)
                .with_source_removed()
                .with_recovery_detail(
                    "The complete copy remains in recovery storage and was not overwritten",
                )
        })?;
        ticket.commit().map_err(|error| {
            Failure::from_io(Operation::Move, source, Some(destination), error)
                .with_source_removed()
                .with_recovery_detail(
                    "The destination is complete, but Files retained a recovery record",
                )
        })?;
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
    journal: Option<&operation_journal::Journal>,
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
                move_item_cancellable(fs, &task.source, &task.destination, cancel, journal)
            }
        };
        report.processed += 1;
        progress(report.processed, tasks.len());

        if let Err(failure) = result {
            if task.kind == TransferKind::Move && failure.source_retained {
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
    use std::cell::{Cell, RefCell};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should follow the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rmac-files-{label}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn staged_transfer_in(parent: &Path) -> PathBuf {
        let mut staged = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".rmac-transfer-"))
            })
            .collect::<Vec<_>>();
        assert_eq!(staged.len(), 1, "exactly one staged transfer should remain");
        staged.pop().unwrap()
    }

    struct FakeFileSystem {
        rename: io::Result<()>,
        copy: io::Result<()>,
        removes: RefCell<Vec<io::Result<()>>>,
        calls: RefCell<Vec<String>>,
    }

    struct CrossDeviceFixture<'a> {
        replace_source_during_copy: bool,
        fail_source_removal: bool,
        racing_destination: Option<PathBuf>,
        cancel_after_copy: Option<&'a AtomicBool>,
        remove_calls: Cell<usize>,
    }

    impl FileSystem for CrossDeviceFixture<'_> {
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            std::fs::create_dir(path)
        }

        fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::CrossesDevices,
                "fixture filesystem boundary",
            ))
        }

        fn copy(&self, source: &Path, destination: &Path) -> io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            let mut output = options.open(destination)?;
            io::copy(&mut std::fs::File::open(source)?, &mut output)?;
            output.sync_all()?;
            if self.replace_source_during_copy {
                std::fs::remove_file(source)?;
                std::fs::write(source, b"replacement bytes")?;
            }
            if let Some(cancel) = self.cancel_after_copy {
                cancel.store(true, Ordering::Release);
            }
            Ok(())
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            self.remove_calls.set(self.remove_calls.get() + 1);
            if self.fail_source_removal {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "fixture source removal failure",
                ))
            } else {
                std::fs::remove_file(path)?;
                if let Some(destination) = &self.racing_destination {
                    std::fs::write(destination, b"racing destination bytes")?;
                }
                Ok(())
            }
        }
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

        let report = execute_transfers(&fs, None, &tasks, &cancel, |_, _| {});

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

        let report = execute_transfers(&fs, None, &tasks, &cancel, |processed, total| {
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

        let report = execute_transfers(&fs, None, &tasks, &cancel, |_, _| {});

        assert!(report.cancelled);
        assert_eq!(report.unfinished_moves, vec![PathBuf::from("source")]);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0]
            .recovery_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("source was retained")));
        assert_eq!(&*fs.calls.borrow(), &["rename", "copy"]);
    }

    #[test]
    fn real_rename_never_replaces_a_racing_destination() {
        let root = TestDirectory::new("rename-conflict");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        std::fs::write(&destination, b"destination bytes").unwrap();

        let failure = rename(&RealFileSystem, &source, &destination).unwrap_err();

        assert_eq!(failure.operation, Operation::Rename);
        assert_eq!(failure.error_kind, io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
        assert_eq!(std::fs::read(&destination).unwrap(), b"destination bytes");
    }

    #[test]
    fn real_rename_moves_to_an_unoccupied_destination() {
        let root = TestDirectory::new("rename-success");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();

        rename(&RealFileSystem, &source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(std::fs::read(&destination).unwrap(), b"source bytes");
    }

    #[test]
    fn journaled_cross_device_move_commits_only_after_source_removal() {
        let root = TestDirectory::new("journaled-cross-device");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        let journal = operation_journal::Journal::open(root.0.join("operation-journal")).unwrap();
        let fs = CrossDeviceFixture {
            replace_source_during_copy: false,
            fail_source_removal: false,
            racing_destination: None,
            cancel_after_copy: None,
            remove_calls: Cell::new(0),
        };
        let tasks = vec![TransferTask {
            kind: TransferKind::Move,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report = execute_transfers(
            &fs,
            Some(&journal),
            &tasks,
            &AtomicBool::new(false),
            |_, _| {},
        );

        assert!(report.failures.is_empty());
        assert!(!source.exists());
        assert_eq!(std::fs::read(destination).unwrap(), b"source bytes");
        assert_eq!(fs.remove_calls.get(), 1);
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn journaled_move_never_removes_a_replaced_source() {
        let root = TestDirectory::new("journaled-replacement");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"original bytes").unwrap();
        let journal = operation_journal::Journal::open(root.0.join("operation-journal")).unwrap();
        let fs = CrossDeviceFixture {
            replace_source_during_copy: true,
            fail_source_removal: false,
            racing_destination: None,
            cancel_after_copy: None,
            remove_calls: Cell::new(0),
        };
        let tasks = vec![TransferTask {
            kind: TransferKind::Move,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report = execute_transfers(
            &fs,
            Some(&journal),
            &tasks,
            &AtomicBool::new(false),
            |_, _| {},
        );

        assert_eq!(report.failures.len(), 1);
        assert_eq!(fs.remove_calls.get(), 0);
        assert_eq!(std::fs::read(&source).unwrap(), b"replacement bytes");
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read(staged_transfer_in(&root.0)).unwrap(),
            b"original bytes"
        );
        assert_eq!(report.unfinished_moves, vec![source]);
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn journal_retains_both_paths_when_source_removal_fails() {
        let root = TestDirectory::new("journaled-removal-failure");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        let journal = operation_journal::Journal::open(root.0.join("operation-journal")).unwrap();
        let fs = CrossDeviceFixture {
            replace_source_during_copy: false,
            fail_source_removal: true,
            racing_destination: None,
            cancel_after_copy: None,
            remove_calls: Cell::new(0),
        };
        let tasks = vec![TransferTask {
            kind: TransferKind::Move,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report = execute_transfers(
            &fs,
            Some(&journal),
            &tasks,
            &AtomicBool::new(false),
            |_, _| {},
        );

        assert_eq!(report.failures.len(), 1);
        assert_eq!(fs.remove_calls.get(), 1);
        assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read(staged_transfer_in(&root.0)).unwrap(),
            b"source bytes"
        );
        assert_eq!(report.unfinished_moves, vec![source]);
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn final_name_conflict_preserves_the_complete_staged_copy() {
        let root = TestDirectory::new("journaled-publish-conflict");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        let journal = operation_journal::Journal::open(root.0.join("operation-journal")).unwrap();
        let fs = CrossDeviceFixture {
            replace_source_during_copy: false,
            fail_source_removal: false,
            racing_destination: Some(destination.clone()),
            cancel_after_copy: None,
            remove_calls: Cell::new(0),
        };
        let tasks = vec![TransferTask {
            kind: TransferKind::Move,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report = execute_transfers(
            &fs,
            Some(&journal),
            &tasks,
            &AtomicBool::new(false),
            |_, _| {},
        );

        assert_eq!(report.failures.len(), 1);
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"racing destination bytes"
        );
        assert_eq!(
            std::fs::read(staged_transfer_in(&root.0)).unwrap(),
            b"source bytes"
        );
        assert!(report.unfinished_moves.is_empty());
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn cancellation_after_journaled_copy_keeps_source_and_staged_copy() {
        let root = TestDirectory::new("journaled-cancellation");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        let journal = operation_journal::Journal::open(root.0.join("operation-journal")).unwrap();
        let cancel = AtomicBool::new(false);
        let fs = CrossDeviceFixture {
            replace_source_during_copy: false,
            fail_source_removal: false,
            racing_destination: None,
            cancel_after_copy: Some(&cancel),
            remove_calls: Cell::new(0),
        };
        let tasks = vec![TransferTask {
            kind: TransferKind::Move,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report = execute_transfers(&fs, Some(&journal), &tasks, &cancel, |_, _| {});

        assert!(report.cancelled);
        assert_eq!(fs.remove_calls.get(), 0);
        assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read(staged_transfer_in(&root.0)).unwrap(),
            b"source bytes"
        );
        assert_eq!(report.unfinished_moves, vec![source]);
        assert_eq!(journal.pending_count().unwrap(), 1);
    }
}
