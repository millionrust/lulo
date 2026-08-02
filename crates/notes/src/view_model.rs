use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecoveryDecision {
    RestoreOriginal,
    PreserveCopy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FolderDialog {
    Rename(FolderId),
    Delete(FolderId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PurgeDialog {
    Note {
        note_id: NoteId,
        note_revision: u64,
        attachment_count: usize,
        attachment_bytes: u64,
    },
    EmptyTrash {
        library_revision: u64,
        note_count: usize,
        attachment_count: usize,
        attachment_bytes: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MoveDialog {
    pub(super) note_id: NoteId,
    pub(super) note_revision: u64,
    pub(super) current_folder: Option<FolderId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AttachmentDialog {
    Remove {
        note_id: NoteId,
        note_revision: u64,
        attachment_id: AttachmentId,
        attachment_revision: u64,
        byte_len: u64,
    },
    CollectOrphan {
        attachment_id: AttachmentId,
        attachment_revision: u64,
        byte_len: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ExportReview {
    pub(super) library_revision: u64,
    pub(super) scope: ExportScope,
    pub(super) note_count: usize,
    pub(super) attachment_count: usize,
    pub(super) markdown_bytes: u64,
    pub(super) attachment_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExportDialog {
    Review(ExportReview),
    Complete(ExportOutcome),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BundleImportCompletion {
    pub(super) folder_count: usize,
    pub(super) note_count: usize,
    pub(super) attachment_count: usize,
    pub(super) attachment_bytes: u64,
    pub(super) maintenance_pending: bool,
}
