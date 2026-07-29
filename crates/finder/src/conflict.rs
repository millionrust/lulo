//! Snapshot-bound Keep Both, Replace, and Skip decisions for Files transfers.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::{file_ops, operation_journal, sanitize_dialog_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConflictTransferKind {
    Copy,
    Move,
}

#[derive(Clone)]
pub(crate) struct TransferConflict {
    pub(crate) kind: ConflictTransferKind,
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
    source_snapshot: operation_journal::TreeSnapshot,
    pub(crate) destination_snapshot: Option<operation_journal::TreeSnapshot>,
}

pub(crate) struct ConflictBatch {
    pub(crate) label: &'static str,
    pub(crate) ready: Vec<file_ops::TransferTask>,
    pub(crate) conflicts: VecDeque<TransferConflict>,
    pub(crate) conflict_total: usize,
    pub(crate) reserved_destinations: BTreeSet<PathBuf>,
    pub(crate) skipped_moves: Vec<PathBuf>,
    pub(crate) keep_unfinished_in_clipboard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConflictDecision {
    KeepBoth,
    Replace,
    Skip,
}

pub(crate) fn conflict_prompt(conflict: &TransferConflict) -> String {
    let name = conflict
        .destination
        .file_name()
        .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
        .unwrap_or_else(|| "this item".to_string());
    let folder = conflict
        .destination
        .parent()
        .and_then(Path::file_name)
        .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "the destination folder".to_string());
    let choice = if conflict.destination_snapshot.is_none() {
        "Another item in this batch needs the same name. Keep Both chooses an available numbered name. Replace is unavailable because no existing destination was reviewed."
    } else if conflict.kind == ConflictTransferKind::Move {
        "Keep Both moves this item under an available numbered name. Replace durably stages the move, atomically publishes it, and retains the reviewed previous item for Command-Z Undo. Skip leaves both items unchanged."
    } else if conflict.source == conflict.destination {
        "Keep Both creates a copy under an available numbered name. An item cannot replace itself, so Replace is disabled. Skip leaves it unchanged."
    } else {
        "Keep Both uses an available numbered name. Replace atomically publishes the new copy and retains the reviewed previous item for Command-Z Undo. Skip leaves both items unchanged."
    };
    format!("An item named “{name}” already exists in “{folder}”. {choice}")
}

pub(crate) fn prepare_conflict_batch(
    label: &'static str,
    tasks: Vec<file_ops::TransferTask>,
    keep_unfinished_in_clipboard: bool,
) -> std::io::Result<ConflictBatch> {
    let mut ready = Vec::with_capacity(tasks.len());
    let mut conflicts = VecDeque::new();
    let mut reserved_destinations = BTreeSet::new();

    for task in tasks {
        let requested_destination = task.destination.clone();
        let kind = match &task.kind {
            file_ops::TransferKind::Copy => ConflictTransferKind::Copy,
            file_ops::TransferKind::Move => ConflictTransferKind::Move,
            file_ops::TransferKind::Replace(_) | file_ops::TransferKind::MoveReplace(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "replacement task cannot enter conflict preflight",
                ));
            }
        };
        let destination_exists = match std::fs::symlink_metadata(&task.destination) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        if destination_exists || reserved_destinations.contains(&task.destination) {
            let source_snapshot = operation_journal::TreeSnapshot::capture(&task.source)?;
            let destination_snapshot = destination_exists
                .then(|| operation_journal::TreeSnapshot::capture(&task.destination))
                .transpose()?;
            conflicts.push_back(TransferConflict {
                kind,
                source: task.source,
                destination: requested_destination.clone(),
                source_snapshot,
                destination_snapshot,
            });
        } else {
            ready.push(task);
        }
        reserved_destinations.insert(requested_destination);
    }
    let conflict_total = conflicts.len();
    Ok(ConflictBatch {
        label,
        ready,
        conflicts,
        conflict_total,
        reserved_destinations,
        skipped_moves: Vec::new(),
        keep_unfinished_in_clipboard,
    })
}

pub(crate) fn resolve_conflict_task(
    conflict: &TransferConflict,
    decision: ConflictDecision,
    reserved: &BTreeSet<PathBuf>,
) -> std::io::Result<Option<file_ops::TransferTask>> {
    if !conflict.source_snapshot.still_matches(&conflict.source)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "conflict source changed",
        ));
    }
    let destination_matches = match &conflict.destination_snapshot {
        Some(snapshot) => snapshot.still_matches(&conflict.destination)?,
        None => match std::fs::symlink_metadata(&conflict.destination) {
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(error),
        },
    };
    if !destination_matches {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "conflict destination changed",
        ));
    }

    match decision {
        ConflictDecision::KeepBoth => {
            let destination = unique_path_avoiding(conflict.destination.clone(), reserved);
            Ok(Some(file_ops::TransferTask {
                kind: match conflict.kind {
                    ConflictTransferKind::Copy => file_ops::TransferKind::Copy,
                    ConflictTransferKind::Move => file_ops::TransferKind::Move,
                },
                source: conflict.source.clone(),
                destination,
            }))
        }
        ConflictDecision::Replace => {
            let destination_snapshot = conflict.destination_snapshot.clone().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "batch-only conflict cannot replace a destination",
                )
            })?;
            if conflict.source == conflict.destination {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "safe replacement is unavailable for this transfer",
                ));
            }
            Ok(Some(file_ops::TransferTask {
                kind: match conflict.kind {
                    ConflictTransferKind::Copy => {
                        file_ops::TransferKind::Replace(Box::new(file_ops::ReplacementBinding {
                            expected_source: conflict.source_snapshot.clone(),
                            expected_destination: destination_snapshot,
                        }))
                    }
                    ConflictTransferKind::Move => file_ops::TransferKind::MoveReplace(Box::new(
                        file_ops::ReplacementBinding {
                            expected_source: conflict.source_snapshot.clone(),
                            expected_destination: destination_snapshot,
                        },
                    )),
                },
                source: conflict.source.clone(),
                destination: conflict.destination.clone(),
            }))
        }
        ConflictDecision::Skip => Ok(None),
    }
}

pub(crate) fn unique_path_avoiding(path: PathBuf, reserved: &BTreeSet<PathBuf>) -> PathBuf {
    if !path.exists() && !reserved.contains(&path) {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());
    for suffix in 2..10_000 {
        let name = match &extension {
            Some(extension) => format!("{stem} {suffix}.{extension}"),
            None => format!("{stem} {suffix}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() && !reserved.contains(&candidate) {
            return candidate;
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should follow the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rmac-files-conflict-{label}-{}-{unique}",
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

    #[test]
    fn transfer_destinations_do_not_collide_with_reserved_batch_paths() {
        let original = PathBuf::from(format!(
            "/tmp/rmac-reserved-destination-{}",
            std::process::id()
        ));
        let reserved = BTreeSet::from([original.clone()]);

        let destination = unique_path_avoiding(original, &reserved);

        assert!(!reserved.contains(&destination));
        assert!(destination.ends_with(format!(
            "rmac-reserved-destination-{} 2",
            std::process::id()
        )));
    }

    #[test]
    fn existing_transfer_destination_opens_a_bound_keep_replace_skip_conflict() {
        let root = TestDirectory::new("preflight");
        let source = root.0.join("incoming").join("report.txt");
        let destination = root.0.join("destination").join("report.txt");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&source, b"incoming bytes").unwrap();
        std::fs::write(&destination, b"previous bytes").unwrap();

        let batch = prepare_conflict_batch(
            "Copying",
            vec![file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: source.clone(),
                destination: destination.clone(),
            }],
            false,
        )
        .unwrap();
        let conflict = batch.conflicts.front().unwrap();

        assert_eq!(batch.conflict_total, 1);
        assert!(batch.ready.is_empty());
        assert!(conflict.destination_snapshot.is_some());
        assert!(conflict_prompt(conflict).contains("Keep Both"));
        assert!(conflict_prompt(conflict).contains("Replace"));

        let keep_both = resolve_conflict_task(
            conflict,
            ConflictDecision::KeepBoth,
            &batch.reserved_destinations,
        )
        .unwrap()
        .unwrap();
        assert_eq!(keep_both.kind, file_ops::TransferKind::Copy);
        assert!(keep_both.destination.ends_with("report 2.txt"));

        let replace = resolve_conflict_task(
            conflict,
            ConflictDecision::Replace,
            &batch.reserved_destinations,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(replace.kind, file_ops::TransferKind::Replace(_)));
        assert_eq!(std::fs::read(source).unwrap(), b"incoming bytes");
        assert_eq!(std::fs::read(destination).unwrap(), b"previous bytes");
    }

    #[test]
    fn conflict_decision_rejects_a_nested_destination_change() {
        let root = TestDirectory::new("destination-change");
        let source = root.0.join("incoming");
        let destination = root.0.join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(source.join("new"), b"new bytes").unwrap();
        std::fs::write(destination.join("old"), b"previous bytes").unwrap();
        let batch = prepare_conflict_batch(
            "Copying",
            vec![file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: source.clone(),
                destination: destination.clone(),
            }],
            false,
        )
        .unwrap();
        let conflict = batch.conflicts.front().unwrap();
        std::fs::write(destination.join("changed"), b"racing bytes").unwrap();

        let error = resolve_conflict_task(
            conflict,
            ConflictDecision::Replace,
            &batch.reserved_destinations,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(std::fs::read(source.join("new")).unwrap(), b"new bytes");
        assert_eq!(
            std::fs::read(destination.join("old")).unwrap(),
            b"previous bytes"
        );
        assert_eq!(
            std::fs::read(destination.join("changed")).unwrap(),
            b"racing bytes"
        );
    }

    #[test]
    fn duplicate_names_inside_one_batch_require_a_non_destructive_choice() {
        let root = TestDirectory::new("batch-name");
        let first = root.0.join("one").join("item");
        let second = root.0.join("two").join("item");
        let destination = root.0.join("destination").join("item");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        let batch = prepare_conflict_batch(
            "Copying",
            vec![
                file_ops::TransferTask {
                    kind: file_ops::TransferKind::Copy,
                    source: first,
                    destination: destination.clone(),
                },
                file_ops::TransferTask {
                    kind: file_ops::TransferKind::Copy,
                    source: second,
                    destination: destination.clone(),
                },
            ],
            false,
        )
        .unwrap();
        let conflict = batch.conflicts.front().unwrap();

        assert_eq!(batch.ready.len(), 1);
        assert_eq!(batch.conflict_total, 1);
        assert!(conflict.destination_snapshot.is_none());
        assert!(resolve_conflict_task(
            conflict,
            ConflictDecision::Replace,
            &batch.reserved_destinations,
        )
        .is_err());
        let keep_both = resolve_conflict_task(
            conflict,
            ConflictDecision::KeepBoth,
            &batch.reserved_destinations,
        )
        .unwrap()
        .unwrap();
        assert!(keep_both.destination.ends_with("item 2"));
        assert!(!destination.exists());
    }

    #[test]
    fn moved_item_conflict_offers_only_snapshot_bound_replacement() {
        let root = TestDirectory::new("move");
        let source = root.0.join("incoming").join("item");
        let destination = root.0.join("destination").join("item");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&source, b"incoming").unwrap();
        std::fs::write(&destination, b"existing").unwrap();
        let batch = prepare_conflict_batch(
            "Moving",
            vec![file_ops::TransferTask {
                kind: file_ops::TransferKind::Move,
                source,
                destination,
            }],
            true,
        )
        .unwrap();
        let conflict = batch.conflicts.front().unwrap();

        assert!(conflict_prompt(conflict).contains("durably stages the move"));
        let replacement = resolve_conflict_task(
            conflict,
            ConflictDecision::Replace,
            &batch.reserved_destinations,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            replacement.kind,
            file_ops::TransferKind::MoveReplace(_)
        ));
    }
}
