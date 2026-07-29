use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::operation_journal;

const MAX_PLANNED_ENTRIES: u64 = 1_000_000;
const MAX_PLANNED_DEPTH: usize = 256;
const ENTRY_SPACE_ALLOWANCE: u64 = 4 * 1024;
const MAX_FREE_SPACE_RESERVE: u64 = 512 * 1024 * 1024;

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
        progress: &mut dyn FnMut(CopyActivity),
    ) -> io::Result<()> {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        self.copy(source, destination)?;
        progress(CopyActivity::Finishing);
        Ok(())
    }
    fn remove(&self, path: &Path) -> io::Result<()>;

    fn source_usage(&self, _path: &Path, cancel: &AtomicBool) -> io::Result<SourceUsage> {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        Ok(SourceUsage {
            logical_bytes: 0,
            entries: 1,
            device: 0,
        })
    }

    fn source_device(&self, path: &Path, cancel: &AtomicBool) -> io::Result<u64> {
        self.source_usage(path, cancel).map(|usage| usage.device)
    }

    fn destination_space(&self, _parent: &Path) -> io::Result<VolumeSpace> {
        Ok(VolumeSpace {
            device: 1,
            total_bytes: u64::MAX,
            available_bytes: u64::MAX,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyActivity {
    Bytes(u64),
    Finishing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceUsage {
    logical_bytes: u64,
    entries: u64,
    device: u64,
}

impl SourceUsage {
    fn required_bytes(self) -> io::Result<u64> {
        self.entries
            .checked_mul(ENTRY_SPACE_ALLOWANCE)
            .and_then(|overhead| self.logical_bytes.checked_add(overhead))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "copy size is too large"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VolumeSpace {
    device: u64,
    total_bytes: u64,
    available_bytes: u64,
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
        progress: &mut dyn FnMut(CopyActivity),
    ) -> io::Result<()> {
        crate::copy_item_cancellable(source, destination, cancel, progress)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        if std::fs::symlink_metadata(path)?.file_type().is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        }
    }

    fn source_usage(&self, path: &Path, cancel: &AtomicBool) -> io::Result<SourceUsage> {
        measure_source(path, cancel)
    }

    fn source_device(&self, path: &Path, cancel: &AtomicBool) -> io::Result<u64> {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "scan cancelled"));
        }
        Ok(std::fs::symlink_metadata(path)?.dev())
    }

    fn destination_space(&self, parent: &Path) -> io::Result<VolumeSpace> {
        let metadata = std::fs::metadata(parent)?;
        let stats = rustix::fs::statvfs(parent).map_err(io::Error::from)?;
        let fragment_size = if stats.f_frsize == 0 {
            stats.f_bsize
        } else {
            stats.f_frsize
        };
        let total_bytes = stats.f_blocks.checked_mul(fragment_size).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "volume size is too large")
        })?;
        let available_bytes = stats.f_bavail.checked_mul(fragment_size).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "free-space value is too large")
        })?;
        Ok(VolumeSpace {
            device: metadata.dev(),
            total_bytes,
            available_bytes,
        })
    }
}

fn measure_source(path: &Path, cancel: &AtomicBool) -> io::Result<SourceUsage> {
    let metadata = std::fs::symlink_metadata(path)?;
    let device = metadata.dev();
    let mut usage = SourceUsage {
        logical_bytes: 0,
        entries: 0,
        device,
    };
    measure_source_inner(path, cancel, 0, &mut usage)?;
    Ok(usage)
}

fn measure_source_inner(
    path: &Path,
    cancel: &AtomicBool,
    depth: usize,
    usage: &mut SourceUsage,
) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "scan cancelled"));
    }
    if depth > MAX_PLANNED_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "folder nesting exceeds the transfer safety limit",
        ));
    }
    usage.entries = usage
        .entries
        .checked_add(1)
        .filter(|entries| *entries <= MAX_PLANNED_ENTRIES)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "transfer contains too many files",
            )
        })?;

    let metadata = std::fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            measure_source_inner(&entry?.path(), cancel, depth + 1, usage)?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "special files cannot be copied",
        ));
    }
    usage.logical_bytes = usage
        .logical_bytes
        .checked_add(metadata.len())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "copy size is too large"))?;
    Ok(())
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
    journal: Option<&operation_journal::Journal>,
    progress: &mut dyn FnMut(CopyActivity),
) -> Result<(), Failure> {
    let Some(journal) = journal else {
        return fs
            .copy_cancellable(source, destination, cancel, progress)
            .map_err(|error| {
                Failure::from_io(Operation::Copy, source, Some(destination), error)
                    .with_recovery_detail(format!(
                        "A partial destination may remain at {}",
                        destination.display()
                    ))
            });
    };

    let mut ticket = journal.prepare_copy(source, destination).map_err(|error| {
        Failure::from_io(Operation::Copy, source, Some(destination), error)
            .with_recovery_detail("Recovery could not be prepared, so no copy was created")
    })?;
    fs.copy_cancellable(source, &ticket.staging_destination(), cancel, progress)
        .map_err(|error| {
            Failure::from_io(Operation::Copy, source, Some(destination), error)
                .with_recovery_detail("The source was retained; a partial recovery copy may remain")
        })?;
    if cancel.load(Ordering::Acquire) {
        return Err(Failure::from_io(
            Operation::Copy,
            source,
            Some(destination),
            io::Error::new(io::ErrorKind::Interrupted, "cancelled"),
        )
        .with_recovery_detail("The source was retained; a complete recovery copy may remain"));
    }
    ticket.mark_destination_complete().map_err(|error| {
        Failure::from_io(Operation::Copy, source, Some(destination), error)
            .with_recovery_detail("The source was retained; a completed recovery copy may remain")
    })?;
    if !ticket.destination_still_matches().map_err(|error| {
        Failure::from_io(Operation::Copy, source, Some(destination), error)
            .with_recovery_detail("The source was retained; the staged copy could not be rechecked")
    })? {
        return Err(Failure::message(
            Operation::Copy,
            source,
            Some(destination),
            "the staged copy changed before publication",
        )
        .with_recovery_detail("The source was retained for safe recovery"));
    }
    if !ticket.source_still_matches().map_err(|error| {
        Failure::from_io(Operation::Copy, source, Some(destination), error)
            .with_recovery_detail("The complete staged copy was retained for review")
    })? {
        return Err(Failure::message(
            Operation::Copy,
            source,
            Some(destination),
            "the source changed while it was being copied",
        )
        .with_recovery_detail(
            "The source was retained; the complete staged copy remains for review",
        ));
    }
    ticket.publish_copy().map_err(|error| {
        Failure::from_io(Operation::Copy, source, Some(destination), error).with_recovery_detail(
            "The source was retained; the complete copy remains in recovery storage",
        )
    })?;
    ticket.commit().map_err(|error| {
        Failure::from_io(Operation::Copy, source, Some(destination), error)
            .with_recovery_detail("The copy is complete, but Files retained a recovery record")
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
    move_item_cancellable(
        fs,
        source,
        destination,
        &AtomicBool::new(false),
        None,
        true,
        &mut |_| {},
    )
}

fn move_item_cancellable(
    fs: &impl FileSystem,
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    journal: Option<&operation_journal::Journal>,
    cross_volume_copy_allowed: bool,
    progress: &mut dyn FnMut(CopyActivity),
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
        Err(error)
            if error.kind() == io::ErrorKind::CrossesDevices && !cross_volume_copy_allowed =>
        {
            return Err(Failure::message(
                Operation::Move,
                source,
                Some(destination),
                "the destination volume changed after the transfer was checked; try again",
            ));
        }
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
        fs.copy_cancellable(source, &staging, cancel, progress)
            .map_err(|error| {
                Failure::from_io(Operation::Move, source, Some(destination), error)
                    .with_recovery_detail(
                        "The source was retained; a partial recovery copy may remain",
                    )
            })?;
    } else {
        copy_cancellable(fs, source, destination, cancel, None, progress)?;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferPhase {
    Scanning,
    Copying,
    Finishing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TransferProgress {
    pub(crate) phase: TransferPhase,
    pub(crate) processed: usize,
    pub(crate) total: usize,
    pub(crate) bytes_processed: u64,
    pub(crate) bytes_total: u64,
}

#[derive(Clone, Copy, Debug)]
struct PlannedTask {
    copy_required: bool,
}

#[derive(Debug)]
struct TransferPlan {
    tasks: Vec<PlannedTask>,
    bytes_total: u64,
}

#[derive(Clone, Copy, Debug)]
struct VolumeRequirement {
    space: VolumeSpace,
    required_bytes: u64,
    task_index: usize,
}

fn plan_transfers(
    fs: &impl FileSystem,
    tasks: &[TransferTask],
    cancel: &AtomicBool,
    progress: &mut impl FnMut(TransferProgress),
) -> Result<TransferPlan, Failure> {
    let mut planned = Vec::with_capacity(tasks.len());
    let mut volumes = BTreeMap::<u64, VolumeRequirement>::new();
    let mut bytes_total = 0u64;

    progress(TransferProgress {
        phase: TransferPhase::Scanning,
        processed: 0,
        total: tasks.len(),
        bytes_processed: 0,
        bytes_total: 0,
    });

    for (index, task) in tasks.iter().enumerate() {
        let operation = match task.kind {
            TransferKind::Copy => Operation::Copy,
            TransferKind::Move => Operation::Move,
        };
        let parent = task.destination.parent().ok_or_else(|| {
            Failure::message(
                operation,
                &task.source,
                Some(&task.destination),
                "the destination has no containing folder",
            )
        })?;
        let space = fs.destination_space(parent).map_err(|error| {
            Failure::from_io(operation, &task.source, Some(&task.destination), error)
        })?;
        let copy_required = match task.kind {
            TransferKind::Copy => true,
            TransferKind::Move => {
                fs.source_device(&task.source, cancel).map_err(|error| {
                    Failure::from_io(operation, &task.source, Some(&task.destination), error)
                })? != space.device
            }
        };

        if copy_required {
            let usage = fs.source_usage(&task.source, cancel).map_err(|error| {
                Failure::from_io(operation, &task.source, Some(&task.destination), error)
            })?;
            bytes_total = bytes_total
                .checked_add(usage.logical_bytes)
                .ok_or_else(|| {
                    Failure::message(
                        operation,
                        &task.source,
                        Some(&task.destination),
                        "the transfer size is too large",
                    )
                })?;
            let required_bytes = usage.required_bytes().map_err(|error| {
                Failure::from_io(operation, &task.source, Some(&task.destination), error)
            })?;
            if let Some(requirement) = volumes.get_mut(&space.device) {
                requirement.required_bytes = requirement
                    .required_bytes
                    .checked_add(required_bytes)
                    .ok_or_else(|| {
                        Failure::message(
                            operation,
                            &task.source,
                            Some(&task.destination),
                            "the transfer size is too large",
                        )
                    })?;
                requirement.space.available_bytes =
                    requirement.space.available_bytes.min(space.available_bytes);
                requirement.space.total_bytes =
                    requirement.space.total_bytes.min(space.total_bytes);
            } else {
                volumes.insert(
                    space.device,
                    VolumeRequirement {
                        space,
                        required_bytes,
                        task_index: index,
                    },
                );
            }
        }

        planned.push(PlannedTask { copy_required });
        progress(TransferProgress {
            phase: TransferPhase::Scanning,
            processed: index + 1,
            total: tasks.len(),
            bytes_processed: 0,
            bytes_total,
        });
    }

    for requirement in volumes.values() {
        let reserve = (requirement.space.total_bytes / 20).min(MAX_FREE_SPACE_RESERVE);
        let usable = requirement.space.available_bytes.saturating_sub(reserve);
        if requirement.required_bytes > usable {
            let task = &tasks[requirement.task_index];
            let operation = match task.kind {
                TransferKind::Copy => Operation::Copy,
                TransferKind::Move => Operation::Move,
            };
            return Err(Failure::message(
                operation,
                &task.source,
                Some(&task.destination),
                format!(
                    "not enough free space on the destination ({} needed, {} available while keeping a {} reserve)",
                    format_bytes(requirement.required_bytes),
                    format_bytes(usable),
                    format_bytes(reserve)
                ),
            ));
        }
    }

    Ok(TransferPlan {
        tasks: planned,
        bytes_total,
    })
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} bytes")
    }
}

pub(crate) fn execute_transfers(
    fs: &impl FileSystem,
    journal: Option<&operation_journal::Journal>,
    tasks: &[TransferTask],
    cancel: &AtomicBool,
    mut progress: impl FnMut(TransferProgress),
) -> TransferReport {
    let mut report = TransferReport::default();
    let plan = match plan_transfers(fs, tasks, cancel, &mut progress) {
        Ok(plan) => plan,
        Err(failure) => {
            report.cancelled = failure.error_kind == io::ErrorKind::Interrupted;
            if !report.cancelled {
                report.failures.push(failure);
            }
            report.unfinished_moves.extend(
                tasks
                    .iter()
                    .filter(|task| task.kind == TransferKind::Move)
                    .map(|task| task.source.clone()),
            );
            return report;
        }
    };
    let mut bytes_processed = 0u64;

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

        let planned = plan.tasks[index];
        let mut phase = if planned.copy_required {
            TransferPhase::Copying
        } else {
            TransferPhase::Finishing
        };
        progress(TransferProgress {
            phase,
            processed: report.processed,
            total: tasks.len(),
            bytes_processed,
            bytes_total: plan.bytes_total,
        });
        let result = {
            let mut copy_progress = |activity| {
                match activity {
                    CopyActivity::Bytes(bytes) => {
                        phase = TransferPhase::Copying;
                        bytes_processed = bytes_processed.saturating_add(bytes);
                    }
                    CopyActivity::Finishing => {
                        phase = TransferPhase::Finishing;
                    }
                }
                progress(TransferProgress {
                    phase,
                    processed: report.processed,
                    total: tasks.len(),
                    bytes_processed,
                    bytes_total: plan.bytes_total,
                });
            };
            match task.kind {
                TransferKind::Copy => copy_cancellable(
                    fs,
                    &task.source,
                    &task.destination,
                    cancel,
                    journal,
                    &mut copy_progress,
                ),
                TransferKind::Move => move_item_cancellable(
                    fs,
                    &task.source,
                    &task.destination,
                    cancel,
                    journal,
                    planned.copy_required,
                    &mut copy_progress,
                ),
            }
        };
        report.processed += 1;
        progress(TransferProgress {
            phase,
            processed: report.processed,
            total: tasks.len(),
            bytes_processed,
            bytes_total: plan.bytes_total,
        });

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

        let report = execute_transfers(&fs, None, &tasks, &cancel, |_| {});

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

        let report = execute_transfers(&fs, None, &tasks, &cancel, |update| {
            progress.push(update);
        });

        assert_eq!(report.processed, 2);
        assert!(report.failures.is_empty());
        assert_eq!(
            progress
                .iter()
                .filter(|update| update.phase == TransferPhase::Finishing)
                .map(|update| update.processed)
                .collect::<Vec<_>>(),
            vec![0, 1, 1, 2]
        );
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

        let report = execute_transfers(&fs, None, &tasks, &cancel, |_| {});

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

        let report =
            execute_transfers(&fs, Some(&journal), &tasks, &AtomicBool::new(false), |_| {});

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

        let report =
            execute_transfers(&fs, Some(&journal), &tasks, &AtomicBool::new(false), |_| {});

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

        let report =
            execute_transfers(&fs, Some(&journal), &tasks, &AtomicBool::new(false), |_| {});

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

        let report =
            execute_transfers(&fs, Some(&journal), &tasks, &AtomicBool::new(false), |_| {});

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

        let report = execute_transfers(&fs, Some(&journal), &tasks, &cancel, |_| {});

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

    #[test]
    fn journaled_copy_publishes_atomically_and_preserves_source() {
        let root = TestDirectory::new("journaled-copy");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        let journal = operation_journal::Journal::open(root.0.join("operation-journal")).unwrap();
        let tasks = vec![TransferTask {
            kind: TransferKind::Copy,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report = execute_transfers(
            &RealFileSystem,
            Some(&journal),
            &tasks,
            &AtomicBool::new(false),
            |_| {},
        );

        assert!(report.failures.is_empty());
        assert_eq!(std::fs::read(source).unwrap(), b"source bytes");
        assert_eq!(std::fs::read(destination).unwrap(), b"source bytes");
        assert_eq!(journal.pending_count().unwrap(), 0);
    }

    #[test]
    fn journaled_copy_never_publishes_a_changed_source() {
        let root = TestDirectory::new("journaled-copy-source-change");
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
            kind: TransferKind::Copy,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report =
            execute_transfers(&fs, Some(&journal), &tasks, &AtomicBool::new(false), |_| {});

        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0]
            .detail
            .contains("source changed while it was being copied"));
        assert_eq!(std::fs::read(source).unwrap(), b"replacement bytes");
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read(staged_transfer_in(&root.0)).unwrap(),
            b"original bytes"
        );
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn cancellation_after_journaled_ordinary_copy_keeps_hidden_stage() {
        let root = TestDirectory::new("journaled-copy-cancellation");
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
            kind: TransferKind::Copy,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report = execute_transfers(&fs, Some(&journal), &tasks, &cancel, |_| {});

        assert!(report.cancelled);
        assert_eq!(std::fs::read(source).unwrap(), b"source bytes");
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read(staged_transfer_in(&root.0)).unwrap(),
            b"source bytes"
        );
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn journaled_copy_name_race_preserves_complete_hidden_stage() {
        struct CopyConflictFixture {
            destination: PathBuf,
        }

        impl FileSystem for CopyConflictFixture {
            fn create_dir(&self, path: &Path) -> io::Result<()> {
                std::fs::create_dir(path)
            }

            fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
                unreachable!("ordinary copy does not use the fixture rename")
            }

            fn copy(&self, source: &Path, staging: &Path) -> io::Result<()> {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                let mut output = options.open(staging)?;
                io::copy(&mut std::fs::File::open(source)?, &mut output)?;
                output.sync_all()?;
                std::fs::write(&self.destination, b"racing destination")?;
                Ok(())
            }

            fn remove(&self, _path: &Path) -> io::Result<()> {
                unreachable!("ordinary copy never removes the source")
            }
        }

        let root = TestDirectory::new("journaled-copy-conflict");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        let journal = operation_journal::Journal::open(root.0.join("operation-journal")).unwrap();
        let fs = CopyConflictFixture {
            destination: destination.clone(),
        };
        let tasks = vec![TransferTask {
            kind: TransferKind::Copy,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report =
            execute_transfers(&fs, Some(&journal), &tasks, &AtomicBool::new(false), |_| {});

        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].error_kind, io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(source).unwrap(), b"source bytes");
        assert_eq!(std::fs::read(destination).unwrap(), b"racing destination");
        assert_eq!(
            std::fs::read(staged_transfer_in(&root.0)).unwrap(),
            b"source bytes"
        );
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    #[test]
    fn mid_copy_enospc_never_exposes_partial_final_name() {
        struct PartialCopyFixture;

        impl FileSystem for PartialCopyFixture {
            fn create_dir(&self, path: &Path) -> io::Result<()> {
                std::fs::create_dir(path)
            }

            fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
                unreachable!("ordinary copy does not use the fixture rename")
            }

            fn copy(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
                unreachable!("the cancellable copy fixture is authoritative")
            }

            fn copy_cancellable(
                &self,
                _source: &Path,
                staging: &Path,
                _cancel: &AtomicBool,
                progress: &mut dyn FnMut(CopyActivity),
            ) -> io::Result<()> {
                std::fs::write(staging, b"partial")?;
                progress(CopyActivity::Bytes(7));
                Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "fixture destination is full",
                ))
            }

            fn remove(&self, _path: &Path) -> io::Result<()> {
                unreachable!("ordinary copy never removes the source")
            }
        }

        let root = TestDirectory::new("journaled-copy-enospc");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        let journal = operation_journal::Journal::open(root.0.join("operation-journal")).unwrap();
        let tasks = vec![TransferTask {
            kind: TransferKind::Copy,
            source: source.clone(),
            destination: destination.clone(),
        }];

        let report = execute_transfers(
            &PartialCopyFixture,
            Some(&journal),
            &tasks,
            &AtomicBool::new(false),
            |_| {},
        );

        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].error_kind, io::ErrorKind::StorageFull);
        assert_eq!(std::fs::read(source).unwrap(), b"source bytes");
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read(staged_transfer_in(&root.0)).unwrap(),
            b"partial"
        );
        assert_eq!(journal.pending_count().unwrap(), 1);
    }

    struct PlanningFixture {
        usage: SourceUsage,
        space: VolumeSpace,
        rename_error: Option<io::ErrorKind>,
        calls: RefCell<Vec<&'static str>>,
    }

    impl FileSystem for PlanningFixture {
        fn create_dir(&self, _path: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push("create_dir");
            Ok(())
        }

        fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push("rename");
            match self.rename_error {
                Some(kind) => Err(io::Error::new(kind, "planned fixture rename")),
                None => Ok(()),
            }
        }

        fn copy(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push("copy");
            Ok(())
        }

        fn remove(&self, _path: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push("remove");
            Ok(())
        }

        fn source_usage(&self, _path: &Path, cancel: &AtomicBool) -> io::Result<SourceUsage> {
            if cancel.load(Ordering::Acquire) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            Ok(self.usage)
        }

        fn destination_space(&self, _parent: &Path) -> io::Result<VolumeSpace> {
            Ok(self.space)
        }
    }

    fn one_task(kind: TransferKind) -> Vec<TransferTask> {
        vec![TransferTask {
            kind,
            source: PathBuf::from("source"),
            destination: PathBuf::from("destination-parent/destination"),
        }]
    }

    #[test]
    fn low_space_preflight_refuses_the_batch_before_mutation() {
        let fs = PlanningFixture {
            usage: SourceUsage {
                logical_bytes: 16 * 1024,
                entries: 2,
                device: 1,
            },
            space: VolumeSpace {
                device: 2,
                total_bytes: 1024 * 1024,
                available_bytes: 32 * 1024,
            },
            rename_error: None,
            calls: RefCell::new(Vec::new()),
        };

        let report = execute_transfers(
            &fs,
            None,
            &one_task(TransferKind::Move),
            &AtomicBool::new(false),
            |_| {},
        );

        assert_eq!(report.processed, 0);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].detail.contains("not enough free space"));
        assert_eq!(
            report.unfinished_moves,
            vec![PathBuf::from("source")],
            "a rejected move must remain available to retry"
        );
        assert!(
            fs.calls.borrow().is_empty(),
            "preflight failure must precede every mutation"
        );
    }

    #[test]
    fn same_volume_move_needs_no_duplicate_space() {
        let fs = PlanningFixture {
            usage: SourceUsage {
                logical_bytes: 8 * 1024 * 1024,
                entries: 1,
                device: 7,
            },
            space: VolumeSpace {
                device: 7,
                total_bytes: 1024 * 1024,
                available_bytes: 0,
            },
            rename_error: None,
            calls: RefCell::new(Vec::new()),
        };

        let report = execute_transfers(
            &fs,
            None,
            &one_task(TransferKind::Move),
            &AtomicBool::new(false),
            |_| {},
        );

        assert!(report.failures.is_empty());
        assert_eq!(&*fs.calls.borrow(), &["rename"]);
    }

    #[test]
    fn changed_mount_boundary_never_bypasses_preflight() {
        let fs = PlanningFixture {
            usage: SourceUsage {
                logical_bytes: 8 * 1024,
                entries: 1,
                device: 7,
            },
            space: VolumeSpace {
                device: 7,
                total_bytes: 1024 * 1024,
                available_bytes: 0,
            },
            rename_error: Some(io::ErrorKind::CrossesDevices),
            calls: RefCell::new(Vec::new()),
        };

        let report = execute_transfers(
            &fs,
            None,
            &one_task(TransferKind::Move),
            &AtomicBool::new(false),
            |_| {},
        );

        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0]
            .detail
            .contains("destination volume changed"));
        assert_eq!(&*fs.calls.borrow(), &["rename"]);
        assert_eq!(report.unfinished_moves, vec![PathBuf::from("source")]);
    }

    #[test]
    fn sparse_files_use_their_logical_copy_size_and_symlinks_are_not_followed() {
        let root = TestDirectory::new("sparse-plan");
        let source = root.0.join("source");
        std::fs::create_dir(&source).unwrap();
        let sparse = source.join("sparse");
        std::fs::File::create(&sparse)
            .unwrap()
            .set_len(8 * 1024 * 1024)
            .unwrap();
        std::os::unix::fs::symlink(&source, source.join("cycle")).unwrap();

        let usage = measure_source(&source, &AtomicBool::new(false)).unwrap();

        assert_eq!(usage.logical_bytes, 8 * 1024 * 1024);
        assert_eq!(usage.entries, 3);
        assert_eq!(
            usage.required_bytes().unwrap(),
            8 * 1024 * 1024 + 3 * ENTRY_SPACE_ALLOWANCE
        );
    }

    #[test]
    fn real_copy_reports_exact_bytes_and_ordered_phases() {
        let root = TestDirectory::new("byte-progress");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        let bytes = vec![0x5a; 600 * 1024];
        std::fs::write(&source, &bytes).unwrap();
        let tasks = vec![TransferTask {
            kind: TransferKind::Copy,
            source: source.clone(),
            destination: destination.clone(),
        }];
        let mut updates = Vec::new();

        let report = execute_transfers(
            &RealFileSystem,
            None,
            &tasks,
            &AtomicBool::new(false),
            |update| updates.push(update),
        );

        assert!(report.failures.is_empty());
        assert_eq!(std::fs::read(destination).unwrap(), bytes);
        assert_eq!(
            updates
                .iter()
                .filter(|update| update.phase == TransferPhase::Copying)
                .map(|update| update.bytes_processed)
                .max(),
            Some(600 * 1024)
        );
        assert!(updates
            .windows(2)
            .all(|pair| pair[0].bytes_processed <= pair[1].bytes_processed));
        let first_copy = updates
            .iter()
            .position(|update| update.phase == TransferPhase::Copying)
            .unwrap();
        let first_finish = updates
            .iter()
            .position(|update| update.phase == TransferPhase::Finishing)
            .unwrap();
        assert!(first_copy < first_finish);
    }

    #[test]
    fn deterministic_mid_copy_enospc_retains_move_source() {
        struct FullDuringCopy {
            calls: RefCell<Vec<&'static str>>,
        }

        impl FileSystem for FullDuringCopy {
            fn create_dir(&self, _path: &Path) -> io::Result<()> {
                Ok(())
            }

            fn rename(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
                self.calls.borrow_mut().push("rename");
                Err(io::Error::new(
                    io::ErrorKind::CrossesDevices,
                    "fixture filesystem boundary",
                ))
            }

            fn copy(&self, _source: &Path, _destination: &Path) -> io::Result<()> {
                unreachable!("the cancellable copy fixture is authoritative")
            }

            fn copy_cancellable(
                &self,
                _source: &Path,
                _destination: &Path,
                _cancel: &AtomicBool,
                progress: &mut dyn FnMut(CopyActivity),
            ) -> io::Result<()> {
                self.calls.borrow_mut().push("copy");
                progress(CopyActivity::Bytes(4096));
                Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "fixture destination is full",
                ))
            }

            fn remove(&self, _path: &Path) -> io::Result<()> {
                self.calls.borrow_mut().push("remove");
                Ok(())
            }

            fn source_usage(&self, _path: &Path, _cancel: &AtomicBool) -> io::Result<SourceUsage> {
                Ok(SourceUsage {
                    logical_bytes: 8192,
                    entries: 1,
                    device: 1,
                })
            }

            fn destination_space(&self, _parent: &Path) -> io::Result<VolumeSpace> {
                Ok(VolumeSpace {
                    device: 2,
                    total_bytes: 1024 * 1024 * 1024,
                    available_bytes: 1024 * 1024 * 1024,
                })
            }
        }

        let fs = FullDuringCopy {
            calls: RefCell::new(Vec::new()),
        };
        let mut updates = Vec::new();
        let report = execute_transfers(
            &fs,
            None,
            &one_task(TransferKind::Move),
            &AtomicBool::new(false),
            |update| updates.push(update),
        );

        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].error_kind, io::ErrorKind::StorageFull);
        assert_eq!(report.unfinished_moves, vec![PathBuf::from("source")]);
        assert_eq!(&*fs.calls.borrow(), &["rename", "copy"]);
        assert_eq!(
            updates.iter().map(|update| update.bytes_processed).max(),
            Some(4096)
        );
    }
}
