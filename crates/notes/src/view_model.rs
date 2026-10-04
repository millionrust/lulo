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

/// File ▸ Lock Note / Application ▸ Close All Locked Notes / Notes ▸
/// Settings… ▸ Locked notes. Notes has no account password service and no
/// Touch ID on Linux (NOTES-13), so the password lives only in this
/// process's memory, hashed with SHA-256: it is never written to the
/// durable library, which is why it does not survive relaunch (docs/
/// parity.md documents this as an honest simplification, not iCloud
/// Keychain-grade protection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LockDialog {
    /// Notes ▸ Settings… ▸ Change Password…, or File ▸ Lock Note on an
    /// unlocked note before any password has ever been set: sets the one
    /// password that locks/unlocks every locked note in this session. The
    /// `Option<NoteId>` is the note to lock immediately afterwards, for
    /// the File ▸ Lock Note path; `None` for the Settings path, which has
    /// no note of its own in view.
    SetPassword(Option<NoteId>),
    /// File ▸ Lock Note on an already-locked, not-yet-unlocked note: ask
    /// for the password before showing its content.
    Unlock(NoteId),
    /// Notes ▸ Settings… ▸ Reset Password…: clears the password and
    /// unlocks every note, after confirmation.
    ConfirmReset,
}

/// A session-only Smart Folder (File ▸ New Smart Folder / New Smart Folder
/// with Tag Selection): a saved filter by one tag, not a real `FolderRecord`
/// (NOTES-11 — Notes has no tag browser or smart-folder storage yet). It
/// lives only in this session for the same reason `LockDialog`'s password
/// does: adding it to the durable library's schema is a bigger, riskier
/// change than this pass's scope (docs/parity.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SmartFolder {
    pub(super) id: u64,
    pub(super) name: String,
    pub(super) tag: String,
}

/// Format ▸ Maths Results / the toolbar's maths-results button. Lulo has
/// no expression evaluator wired to the editor yet; the three settings are
/// real, distinct, persisted-for-the-session choices (NOT-MENU-049..052),
/// but only `Off` has an effect today — see docs/parity.md.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum MathsResultsMode {
    #[default]
    Off,
    SuggestResults,
    InsertResults,
}

/// View ▸ Customise Toolbar…: the optional editor-toolbar buttons a person
/// can hide. Compose, Format and Search are always shown, like the Mac's
/// own undraggable items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ToolbarItem {
    Checklist,
    InsertTable,
    Media,
    MoveNote,
    MathsResults,
}

impl ToolbarItem {
    pub(super) const ALL: [Self; 5] = [
        Self::Checklist,
        Self::InsertTable,
        Self::Media,
        Self::MoveNote,
        Self::MathsResults,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Checklist => "Checklist",
            Self::InsertTable => "Table",
            Self::Media => "Media",
            Self::MoveNote => "Move Note…",
            Self::MathsResults => "Maths Results",
        }
    }
}
