use super::*;

pub(super) enum TransferEvent {
    Progress(file_ops::TransferProgress),
    Finished {
        report: file_ops::TransferReport,
        recovery_reviews: std::io::Result<Vec<operation_journal::RecoveryReview>>,
        undo_availability: std::io::Result<Option<undo_journal::UndoAvailability>>,
    },
}

pub(super) enum UndoEvent {
    Progress(file_ops::CopyActivity),
    Finished {
        outcome: std::io::Result<Option<undo_journal::UndoOutcome>>,
        availability: std::io::Result<Option<undo_journal::UndoAvailability>>,
    },
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy)]
pub(super) enum TrashTaskKind {
    Move,
    Restore,
    Delete,
}

#[cfg(any(target_os = "linux", test))]
pub(super) struct TrashCompletion {
    pub(super) kind: TrashTaskKind,
    pub(super) completed: usize,
    pub(super) cancelled: bool,
    pub(super) failures: Vec<file_ops::Failure>,
    pub(super) recovery: std::io::Result<(
        trash_store::TrashRecovery,
        Vec<trash_store::TrashRecoveryReview>,
    )>,
    pub(super) undo_availability: std::io::Result<Option<undo_journal::UndoAvailability>>,
}

#[cfg(any(target_os = "linux", test))]
pub(super) enum TrashEvent {
    Progress { processed: usize, total: usize },
    Finished(TrashCompletion),
}

#[derive(Clone)]
pub(super) struct ActiveTransfer {
    pub(super) label: SharedString,
    pub(super) phase: file_ops::TransferPhase,
    pub(super) processed: usize,
    pub(super) total: usize,
    pub(super) bytes_processed: u64,
    pub(super) bytes_total: u64,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) cancelling: bool,
    pub(super) keep_unfinished_in_clipboard: bool,
    pub(super) retained_clipboard: Vec<PathBuf>,
}

#[derive(Clone)]
pub(super) struct ActiveUndo {
    pub(super) label: SharedString,
    pub(super) phase: file_ops::TransferPhase,
    pub(super) bytes_processed: u64,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) cancelling: bool,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone)]
pub(super) struct ActiveTrash {
    pub(super) label: SharedString,
    pub(super) processed: usize,
    pub(super) total: usize,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) cancelling: bool,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone)]
pub(super) struct DeleteConfirmation {
    pub(super) items: Vec<trash_store::TrashedItem>,
}

#[derive(Clone)]
pub(super) struct OpenWithPicker {
    pub(super) path: PathBuf,
    pub(super) association: Option<rmac_apps::FileAssociation>,
    pub(super) selected: usize,
    pub(super) make_default: bool,
    pub(super) busy: bool,
    pub(super) error: Option<SharedString>,
}

#[derive(Clone)]
pub(super) struct QuickLookPanel {
    pub(super) paths: Vec<PathBuf>,
    pub(super) current: usize,
    pub(super) content: Option<quick_look::Content>,
    pub(super) error: Option<SharedString>,
    pub(super) cancel: Arc<AtomicBool>,
}
