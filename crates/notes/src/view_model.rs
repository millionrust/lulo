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

/// File ▸ Lock Note / Remove Lock, the locked-note placeholder, and Notes ▸
/// Settings… ▸ Change/Reset Password…. The view only collects passwords:
/// the runtime worker stretches them with Argon2id and seals locked notes
/// (title, body, tags, attachments) with XChaCha20-Poly1305 in the library,
/// so the password and the lock survive relaunch (NOTES-13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LockDialog {
    /// No locked-notes password exists yet: Password, Verify and Hint, then
    /// lock `then_lock` (File ▸ Lock Note) or just set it (Settings).
    CreatePassword { then_lock: Option<(NoteId, u64)> },
    /// File ▸ Lock Note while the password is closed: prove it to lock.
    LockWithPassword { note_id: NoteId, revision: u64 },
    /// The locked-note placeholder's View Note…: opens every note locked
    /// with the same password for this session.
    Unlock(NoteId),
    /// File ▸ Remove Lock on a closed note.
    RemoveLock { note_id: NoteId, revision: u64 },
    /// Notes ▸ Settings… ▸ Change Password…: old, new, verify, hint.
    ChangePassword,
    /// Notes ▸ Settings… ▸ Reset Password…: confirmation first…
    ConfirmReset,
    /// …then the new password, which only future locks use.
    ResetPassword,
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

/// Notes ▸ Settings… ▸ New notes start with:. Notes keeps the title in its
/// own field rather than the Mac's single-document first line (NOTES-01),
/// so this instead pre-seeds the new note's body with the matching
/// Markdown paragraph-style marker and places the caret after it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum NewNoteBodyStyle {
    #[default]
    Title,
    Heading,
    Body,
}

impl NewNoteBodyStyle {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Heading => "Heading",
            Self::Body => "Body",
        }
    }

    pub(super) fn marker(self) -> &'static str {
        match self {
            Self::Title => "# ",
            Self::Heading => "## ",
            Self::Body => "",
        }
    }
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
