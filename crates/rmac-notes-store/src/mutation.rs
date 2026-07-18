use std::collections::BTreeSet;
use std::fmt;

use crate::{
    validate_name, validate_tag, AttachmentId, AttachmentKind, AttachmentRecord, FolderId,
    FolderRecord, LibrarySnapshot, NoteId, NoteRecord, SortOrder, ValidationError, MAX_ATTACHMENTS,
    MAX_ATTACHMENTS_PER_NOTE, MAX_ATTACHMENT_BYTES, MAX_BODY_BYTES, MAX_TAGS_PER_NOTE,
    MAX_TITLE_BYTES,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewNote {
    pub created_unix_ms: u64,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub folder_id: Option<FolderId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteChanges {
    pub modified_unix_ms: u64,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
}

/// Metadata for one fully read and validated image selected for import.
///
/// The source path and raw bytes deliberately do not enter the domain model.
/// Storage must prove that the bytes it stages have this exact length and
/// digest before publishing the resulting library candidate.
#[derive(Clone, PartialEq, Eq)]
pub struct NewAttachment {
    pub display_name: String,
    pub kind: AttachmentKind,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

impl fmt::Debug for NewAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewAttachment")
            .field("display_name", &"<redacted>")
            .field("kind", &self.kind)
            .field("byte_len", &self.byte_len)
            .field("sha256", &"<redacted>")
            .finish()
    }
}

/// Exact metadata boundary for one managed attachment import.
///
/// Import is an exclusive transaction. Storage persists this scope before it
/// stages bytes, then publishes only the candidate that can be re-derived from
/// the accepted base through this plan. The digest remains redacted from debug
/// output because attachment bytes are private user data.
#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentImportPlan {
    pub base_library_revision: u64,
    pub candidate_library_revision: u64,
    pub note_id: NoteId,
    pub base_note_revision: u64,
    pub candidate_note_revision: u64,
    pub attachment_id: AttachmentId,
    pub kind: AttachmentKind,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

impl fmt::Debug for AttachmentImportPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttachmentImportPlan")
            .field("base_library_revision", &self.base_library_revision)
            .field(
                "candidate_library_revision",
                &self.candidate_library_revision,
            )
            .field("note_id", &self.note_id)
            .field("base_note_revision", &self.base_note_revision)
            .field("candidate_note_revision", &self.candidate_note_revision)
            .field("attachment_id", &self.attachment_id)
            .field("kind", &self.kind)
            .field("byte_len", &self.byte_len)
            .field("sha256", &"<redacted>")
            .finish()
    }
}

impl AttachmentImportPlan {
    /// Prove that `candidate` is exactly `base` plus this one attachment.
    ///
    /// The display name and modified timestamp are recovered from the
    /// candidate, then the ordinary mutation is replayed and compared in full.
    /// This keeps storage from maintaining a second copy of domain invariants.
    pub fn validate_candidate(
        &self,
        base: &LibrarySnapshot,
        candidate: &LibrarySnapshot,
    ) -> Result<(), MutationError> {
        if base.revision != self.base_library_revision
            || candidate.revision != self.candidate_library_revision
        {
            return Err(MutationError::RevisionConflict);
        }
        candidate
            .validate()
            .map_err(MutationError::InvalidCandidate)?;
        let attachment = candidate
            .attachments
            .iter()
            .find(|attachment| attachment.id == self.attachment_id)
            .ok_or(MutationError::InvalidCandidate(
                ValidationError::MissingReference,
            ))?;
        if attachment.note_id != self.note_id
            || attachment.kind != self.kind
            || attachment.byte_len != self.byte_len
            || attachment.sha256 != self.sha256
        {
            return Err(MutationError::InvalidCandidate(
                ValidationError::InconsistentAttachment,
            ));
        }
        let note = candidate
            .notes
            .iter()
            .find(|note| note.id == self.note_id)
            .ok_or(MutationError::InvalidCandidate(
                ValidationError::MissingReference,
            ))?;
        if note.revision != self.candidate_note_revision {
            return Err(MutationError::RevisionConflict);
        }

        let mut transaction = LibraryTransaction::begin(base)?;
        let replayed = transaction.add_attachment(
            self.note_id,
            self.base_note_revision,
            note.modified_unix_ms,
            NewAttachment {
                display_name: attachment.display_name.clone(),
                kind: attachment.kind,
                byte_len: attachment.byte_len,
                sha256: attachment.sha256,
            },
        )?;
        let expected = transaction.finish()?;
        if replayed != *self || expected != *candidate {
            return Err(MutationError::InvalidCandidate(
                ValidationError::InconsistentAttachment,
            ));
        }
        Ok(())
    }
}

/// Exact identities removed from one candidate library revision.
///
/// Managed attachment bytes remain untouched until the metadata candidate is
/// durably accepted. Storage uses this bounded plan for post-commit cleanup;
/// presenting permanent deletion before that cleanup is verified is forbidden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgePlan {
    pub base_library_revision: u64,
    pub candidate_library_revision: u64,
    pub note_ids: Vec<NoteId>,
    pub attachment_ids: Vec<AttachmentId>,
    pub attachment_bytes: u64,
}

impl NoteChanges {
    /// Validate content bounds and tag invariants without requiring a base
    /// note. Storage-side recovery records use this before retaining a draft.
    pub fn validate_content(&self) -> Result<(), MutationError> {
        validate_note_content(&self.title, &self.body, &self.tags)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationError {
    InvalidBase(ValidationError),
    InvalidCandidate(ValidationError),
    RevisionExhausted,
    IdentityExhausted,
    RevisionConflict,
    MissingFolder,
    MissingNote,
    MissingAttachment,
    DeletedFolder,
    DeletedNote,
    AttachmentNotOwned,
    AttachmentAlreadyRemoved,
    NoteNotTrashed,
    AttachmentImportRequiresExclusiveTransaction,
    PurgeRequiresExclusiveTransaction,
    NoChanges,
}

impl fmt::Display for MutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBase(error) | Self::InvalidCandidate(error) => {
                return error.fmt(formatter)
            }
            Self::RevisionExhausted => "the Notes library revision sequence is exhausted",
            Self::IdentityExhausted => "the Notes identity sequence is exhausted",
            Self::RevisionConflict => "the note or folder changed before this edit was accepted",
            Self::MissingFolder => "the selected Notes folder no longer exists",
            Self::MissingNote => "the selected note no longer exists",
            Self::MissingAttachment => "the selected note attachment no longer exists",
            Self::DeletedFolder => "the selected Notes folder is in Notes Trash",
            Self::DeletedNote => "the selected note is in Notes Trash",
            Self::AttachmentNotOwned => "the selected attachment does not belong to this note",
            Self::AttachmentAlreadyRemoved => {
                "the selected attachment reference was already removed"
            }
            Self::NoteNotTrashed => "the selected note is not in Notes Trash",
            Self::AttachmentImportRequiresExclusiveTransaction => {
                "an attachment import requires a separate Notes transaction"
            }
            Self::PurgeRequiresExclusiveTransaction => {
                "permanent deletion requires a separate Notes transaction"
            }
            Self::NoChanges => "the Notes transaction contains no changes",
        })
    }
}

impl std::error::Error for MutationError {}

/// A complete candidate derived from one validated durable snapshot.
///
/// It is intentionally not exposed until [`finish`](Self::finish) validates
/// every cross-record invariant. Callers discard the transaction on any error
/// and pass only a finished candidate to the storage adapter.
pub struct LibraryTransaction {
    base_revision: u64,
    candidate: LibrarySnapshot,
    changed: bool,
}

impl LibraryTransaction {
    pub fn begin(base: &LibrarySnapshot) -> Result<Self, MutationError> {
        base.validate().map_err(MutationError::InvalidBase)?;
        let revision = base
            .revision
            .checked_add(1)
            .ok_or(MutationError::RevisionExhausted)?;
        let mut candidate = base.clone();
        candidate.revision = revision;
        Ok(Self {
            base_revision: base.revision,
            candidate,
            changed: false,
        })
    }

    pub fn create_folder(&mut self, name: String) -> Result<FolderId, MutationError> {
        validate_name(&name).map_err(MutationError::InvalidCandidate)?;
        let identity = name.to_lowercase();
        if self
            .candidate
            .folders
            .iter()
            .any(|folder| !folder.deleted && folder.name.to_lowercase() == identity)
        {
            return Err(MutationError::InvalidCandidate(
                ValidationError::DuplicateName,
            ));
        }
        let id =
            FolderId::new(self.candidate.next_folder_id).ok_or(MutationError::IdentityExhausted)?;
        self.candidate.next_folder_id = next_identity(self.candidate.next_folder_id)?;
        self.candidate.folders.push(FolderRecord {
            id,
            revision: 1,
            name,
            deleted: false,
        });
        self.changed = true;
        Ok(id)
    }

    pub fn rename_folder(
        &mut self,
        id: FolderId,
        expected_revision: u64,
        name: String,
    ) -> Result<bool, MutationError> {
        validate_name(&name).map_err(MutationError::InvalidCandidate)?;
        let identity = name.to_lowercase();
        if self.candidate.folders.iter().any(|folder| {
            folder.id != id && !folder.deleted && folder.name.to_lowercase() == identity
        }) {
            return Err(MutationError::InvalidCandidate(
                ValidationError::DuplicateName,
            ));
        }
        let folder = self
            .candidate
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(MutationError::MissingFolder)?;
        require_revision(folder.revision, expected_revision)?;
        if folder.deleted {
            return Err(MutationError::DeletedFolder);
        }
        if folder.name == name {
            return Ok(false);
        }
        folder.revision = next_revision(folder.revision)?;
        folder.name = name;
        self.changed = true;
        Ok(true)
    }

    pub fn delete_folder(
        &mut self,
        id: FolderId,
        expected_revision: u64,
    ) -> Result<usize, MutationError> {
        let folder = self
            .candidate
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(MutationError::MissingFolder)?;
        require_revision(folder.revision, expected_revision)?;
        if folder.deleted {
            return Err(MutationError::DeletedFolder);
        }
        folder.revision = next_revision(folder.revision)?;
        folder.deleted = true;

        let mut moved = 0_usize;
        for note in &mut self.candidate.notes {
            if !note.deleted && note.folder_id == Some(id) {
                note.revision = next_revision(note.revision)?;
                note.folder_id = None;
                moved = moved.saturating_add(1);
            }
        }
        self.changed = true;
        Ok(moved)
    }

    pub fn create_note(&mut self, note: NewNote) -> Result<NoteId, MutationError> {
        validate_note_content(&note.title, &note.body, &note.tags)?;
        require_live_folder(&self.candidate, note.folder_id)?;
        let id =
            NoteId::new(self.candidate.next_note_id).ok_or(MutationError::IdentityExhausted)?;
        self.candidate.next_note_id = next_identity(self.candidate.next_note_id)?;
        self.candidate.notes.push(NoteRecord {
            id,
            revision: 1,
            created_unix_ms: note.created_unix_ms,
            modified_unix_ms: note.created_unix_ms,
            title: note.title,
            body: note.body,
            tags: note.tags,
            folder_id: note.folder_id,
            pinned: false,
            deleted: false,
            attachments: Vec::new(),
        });
        self.changed = true;
        Ok(id)
    }

    pub fn edit_note(
        &mut self,
        id: NoteId,
        expected_revision: u64,
        changes: NoteChanges,
    ) -> Result<bool, MutationError> {
        changes.validate_content()?;
        let note = self.note_mut(id, expected_revision)?;
        if changes.modified_unix_ms < note.created_unix_ms {
            return Err(MutationError::InvalidCandidate(
                ValidationError::InvalidTimestamp,
            ));
        }
        if note.title == changes.title
            && note.body == changes.body
            && note.tags == changes.tags
            && note.modified_unix_ms == changes.modified_unix_ms
        {
            return Ok(false);
        }
        note.revision = next_revision(note.revision)?;
        note.modified_unix_ms = changes.modified_unix_ms;
        note.title = changes.title;
        note.body = changes.body;
        note.tags = changes.tags;
        self.changed = true;
        Ok(true)
    }

    pub fn move_note(
        &mut self,
        id: NoteId,
        expected_revision: u64,
        folder_id: Option<FolderId>,
    ) -> Result<bool, MutationError> {
        require_live_folder(&self.candidate, folder_id)?;
        let note = self.note_mut(id, expected_revision)?;
        if note.folder_id == folder_id {
            return Ok(false);
        }
        note.revision = next_revision(note.revision)?;
        note.folder_id = folder_id;
        self.changed = true;
        Ok(true)
    }

    pub fn set_note_pinned(
        &mut self,
        id: NoteId,
        expected_revision: u64,
        pinned: bool,
    ) -> Result<bool, MutationError> {
        let note = self.note_mut(id, expected_revision)?;
        if note.pinned == pinned {
            return Ok(false);
        }
        note.revision = next_revision(note.revision)?;
        note.pinned = pinned;
        self.changed = true;
        Ok(true)
    }

    pub fn trash_note(&mut self, id: NoteId, expected_revision: u64) -> Result<(), MutationError> {
        let note = self.note_mut(id, expected_revision)?;
        note.revision = next_revision(note.revision)?;
        note.pinned = false;
        note.deleted = true;
        self.changed = true;
        Ok(())
    }

    pub fn restore_note(
        &mut self,
        id: NoteId,
        expected_revision: u64,
    ) -> Result<Option<FolderId>, MutationError> {
        let folder_id = {
            let note = self
                .candidate
                .notes
                .iter()
                .find(|note| note.id == id)
                .ok_or(MutationError::MissingNote)?;
            require_revision(note.revision, expected_revision)?;
            if !note.deleted {
                return Err(MutationError::RevisionConflict);
            }
            note.folder_id.filter(|folder_id| {
                self.candidate
                    .folders
                    .iter()
                    .any(|folder| folder.id == *folder_id && !folder.deleted)
            })
        };
        let note = self
            .candidate
            .notes
            .iter_mut()
            .find(|note| note.id == id)
            .ok_or(MutationError::MissingNote)?;
        note.revision = next_revision(note.revision)?;
        note.folder_id = folder_id;
        note.deleted = false;
        self.changed = true;
        Ok(folder_id)
    }

    /// Attach one exact, already validated image to one live note.
    ///
    /// This mutation is exclusive because storage must stage the corresponding
    /// bytes and persist a recovery intent before this exact metadata candidate
    /// can become authoritative.
    pub fn add_attachment(
        &mut self,
        note_id: NoteId,
        expected_note_revision: u64,
        modified_unix_ms: u64,
        attachment: NewAttachment,
    ) -> Result<AttachmentImportPlan, MutationError> {
        if self.changed {
            return Err(MutationError::AttachmentImportRequiresExclusiveTransaction);
        }
        validate_name(&attachment.display_name).map_err(MutationError::InvalidCandidate)?;
        if attachment.byte_len == 0
            || attachment.byte_len > MAX_ATTACHMENT_BYTES
            || attachment.sha256 == [0; 32]
        {
            return Err(MutationError::InvalidCandidate(
                ValidationError::InvalidAttachment,
            ));
        }
        if self.candidate.attachments.len() >= MAX_ATTACHMENTS {
            return Err(MutationError::InvalidCandidate(
                ValidationError::CollectionLimit,
            ));
        }

        let note_index = self
            .candidate
            .notes
            .iter()
            .position(|note| note.id == note_id)
            .ok_or(MutationError::MissingNote)?;
        let note = &self.candidate.notes[note_index];
        require_revision(note.revision, expected_note_revision)?;
        if note.deleted {
            return Err(MutationError::DeletedNote);
        }
        if note.attachments.len() >= MAX_ATTACHMENTS_PER_NOTE {
            return Err(MutationError::InvalidCandidate(
                ValidationError::CollectionLimit,
            ));
        }
        if modified_unix_ms < note.modified_unix_ms {
            return Err(MutationError::InvalidCandidate(
                ValidationError::InvalidTimestamp,
            ));
        }

        let attachment_id = AttachmentId::new(self.candidate.next_attachment_id)
            .ok_or(MutationError::IdentityExhausted)?;
        self.candidate.next_attachment_id = next_identity(self.candidate.next_attachment_id)?;
        let candidate_note_revision = next_revision(expected_note_revision)?;
        let note = &mut self.candidate.notes[note_index];
        note.revision = candidate_note_revision;
        note.modified_unix_ms = modified_unix_ms;
        note.attachments.push(attachment_id);
        self.candidate.attachments.push(AttachmentRecord {
            id: attachment_id,
            revision: 1,
            note_id,
            display_name: attachment.display_name,
            kind: attachment.kind,
            byte_len: attachment.byte_len,
            sha256: attachment.sha256,
            deleted: false,
        });
        self.changed = true;
        Ok(AttachmentImportPlan {
            base_library_revision: self.base_revision,
            candidate_library_revision: self.candidate.revision,
            note_id,
            base_note_revision: expected_note_revision,
            candidate_note_revision,
            attachment_id,
            kind: attachment.kind,
            byte_len: attachment.byte_len,
            sha256: attachment.sha256,
        })
    }

    /// Remove one exact live attachment reference and retain its bytes as an
    /// authoritative orphan tombstone.
    ///
    /// This transaction never authorizes filesystem deletion. A separate
    /// storage-backed collection operation must review the accepted tombstone
    /// before the managed bytes or record can be removed.
    pub fn remove_attachment_reference(
        &mut self,
        note_id: NoteId,
        expected_note_revision: u64,
        attachment_id: AttachmentId,
        expected_attachment_revision: u64,
        modified_unix_ms: u64,
    ) -> Result<(), MutationError> {
        let note_index = self
            .candidate
            .notes
            .iter()
            .position(|note| note.id == note_id)
            .ok_or(MutationError::MissingNote)?;
        let attachment_index = self
            .candidate
            .attachments
            .iter()
            .position(|attachment| attachment.id == attachment_id)
            .ok_or(MutationError::MissingAttachment)?;
        let note = &self.candidate.notes[note_index];
        let attachment = &self.candidate.attachments[attachment_index];
        require_revision(note.revision, expected_note_revision)?;
        require_revision(attachment.revision, expected_attachment_revision)?;
        if note.deleted {
            return Err(MutationError::DeletedNote);
        }
        if attachment.note_id != note_id {
            return Err(MutationError::AttachmentNotOwned);
        }
        if attachment.deleted {
            return Err(MutationError::AttachmentAlreadyRemoved);
        }
        if !note.attachments.contains(&attachment_id) {
            return Err(MutationError::AttachmentNotOwned);
        }
        if modified_unix_ms < note.modified_unix_ms {
            return Err(MutationError::InvalidCandidate(
                ValidationError::InvalidTimestamp,
            ));
        }
        let note_revision = next_revision(note.revision)?;
        let attachment_revision = next_revision(attachment.revision)?;

        let note = &mut self.candidate.notes[note_index];
        note.revision = note_revision;
        note.modified_unix_ms = modified_unix_ms;
        note.attachments.retain(|id| *id != attachment_id);
        let attachment = &mut self.candidate.attachments[attachment_index];
        attachment.revision = attachment_revision;
        attachment.deleted = true;
        self.changed = true;
        Ok(())
    }

    /// Remove one exact trashed note and all attachment records it owns.
    ///
    /// The returned plan does not authorize deleting managed bytes until this
    /// transaction's candidate revision has been durably accepted.
    pub fn purge_trashed_note(
        &mut self,
        id: NoteId,
        expected_revision: u64,
    ) -> Result<PurgePlan, MutationError> {
        if self.changed {
            return Err(MutationError::PurgeRequiresExclusiveTransaction);
        }
        let note_index = self
            .candidate
            .notes
            .iter()
            .position(|note| note.id == id)
            .ok_or(MutationError::MissingNote)?;
        let note = &self.candidate.notes[note_index];
        require_revision(note.revision, expected_revision)?;
        if !note.deleted {
            return Err(MutationError::NoteNotTrashed);
        }
        let plan = purge_plan(
            &self.candidate,
            self.base_revision,
            std::iter::once(id).collect(),
        )?;
        self.candidate.notes.remove(note_index);
        self.candidate
            .attachments
            .retain(|attachment| attachment.note_id != id);
        self.changed = true;
        Ok(plan)
    }

    /// Remove exactly the notes that were in Trash at the reviewed library
    /// revision. A newer library revision must be reviewed again so a note
    /// trashed after confirmation is never swept into the operation.
    pub fn empty_trash(
        &mut self,
        expected_library_revision: u64,
    ) -> Result<PurgePlan, MutationError> {
        if self.changed {
            return Err(MutationError::PurgeRequiresExclusiveTransaction);
        }
        require_revision(self.base_revision, expected_library_revision)?;
        let note_ids = self
            .candidate
            .notes
            .iter()
            .filter(|note| note.deleted)
            .map(|note| note.id)
            .collect::<BTreeSet<_>>();
        if note_ids.is_empty() {
            return Err(MutationError::NoChanges);
        }
        let plan = purge_plan(&self.candidate, self.base_revision, note_ids.clone())?;
        self.candidate
            .notes
            .retain(|note| !note_ids.contains(&note.id));
        self.candidate
            .attachments
            .retain(|attachment| !note_ids.contains(&attachment.note_id));
        self.changed = true;
        Ok(plan)
    }

    pub fn set_sort_order(&mut self, sort_order: SortOrder) -> bool {
        if self.candidate.sort_order == sort_order {
            return false;
        }
        self.candidate.sort_order = sort_order;
        self.changed = true;
        true
    }

    pub fn finish(self) -> Result<LibrarySnapshot, MutationError> {
        if !self.changed {
            return Err(MutationError::NoChanges);
        }
        self.candidate
            .validate()
            .map_err(MutationError::InvalidCandidate)?;
        Ok(self.candidate)
    }

    fn note_mut(
        &mut self,
        id: NoteId,
        expected_revision: u64,
    ) -> Result<&mut NoteRecord, MutationError> {
        let note = self
            .candidate
            .notes
            .iter_mut()
            .find(|note| note.id == id)
            .ok_or(MutationError::MissingNote)?;
        require_revision(note.revision, expected_revision)?;
        if note.deleted {
            return Err(MutationError::DeletedNote);
        }
        Ok(note)
    }
}

fn purge_plan(
    snapshot: &LibrarySnapshot,
    base_library_revision: u64,
    note_ids: BTreeSet<NoteId>,
) -> Result<PurgePlan, MutationError> {
    let mut attachment_ids = Vec::new();
    let mut attachment_bytes = 0_u64;
    for attachment in &snapshot.attachments {
        if note_ids.contains(&attachment.note_id) {
            attachment_ids.push(attachment.id);
            attachment_bytes = attachment_bytes.checked_add(attachment.byte_len).ok_or(
                MutationError::InvalidCandidate(ValidationError::CollectionLimit),
            )?;
        }
    }
    attachment_ids.sort_unstable();
    Ok(PurgePlan {
        base_library_revision,
        candidate_library_revision: snapshot.revision,
        note_ids: note_ids.into_iter().collect(),
        attachment_ids,
        attachment_bytes,
    })
}

fn require_live_folder(
    snapshot: &LibrarySnapshot,
    folder_id: Option<FolderId>,
) -> Result<(), MutationError> {
    let Some(folder_id) = folder_id else {
        return Ok(());
    };
    let folder = snapshot
        .folders
        .iter()
        .find(|folder| folder.id == folder_id)
        .ok_or(MutationError::MissingFolder)?;
    if folder.deleted {
        Err(MutationError::DeletedFolder)
    } else {
        Ok(())
    }
}

fn validate_note_content(title: &str, body: &str, tags: &[String]) -> Result<(), MutationError> {
    if title.len() > MAX_TITLE_BYTES
        || body.len() > MAX_BODY_BYTES
        || title.chars().any(char::is_control)
        || body.contains('\0')
    {
        return Err(MutationError::InvalidCandidate(
            ValidationError::InvalidText,
        ));
    }
    if tags.len() > MAX_TAGS_PER_NOTE {
        return Err(MutationError::InvalidCandidate(
            ValidationError::CollectionLimit,
        ));
    }
    let mut unique = BTreeSet::new();
    for tag in tags {
        validate_tag(tag).map_err(MutationError::InvalidCandidate)?;
        if !unique.insert(tag.to_lowercase()) {
            return Err(MutationError::InvalidCandidate(
                ValidationError::DuplicateName,
            ));
        }
    }
    Ok(())
}

fn require_revision(actual: u64, expected: u64) -> Result<(), MutationError> {
    if actual == expected {
        Ok(())
    } else {
        Err(MutationError::RevisionConflict)
    }
}

fn next_revision(current: u64) -> Result<u64, MutationError> {
    current
        .checked_add(1)
        .ok_or(MutationError::RevisionExhausted)
}

fn next_identity(current: u64) -> Result<u64, MutationError> {
    let next = current
        .checked_add(1)
        .ok_or(MutationError::IdentityExhausted)?;
    if next == u64::MAX {
        Err(MutationError::IdentityExhausted)
    } else {
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> LibrarySnapshot {
        let folder_id = FolderId::new(1).unwrap();
        let note_id = NoteId::new(1).unwrap();
        LibrarySnapshot {
            revision: 7,
            sort_order: SortOrder::Edited,
            next_note_id: 2,
            next_folder_id: 2,
            next_attachment_id: 1,
            folders: vec![FolderRecord {
                id: folder_id,
                revision: 2,
                name: "Projects".into(),
                deleted: false,
            }],
            notes: vec![NoteRecord {
                id: note_id,
                revision: 3,
                created_unix_ms: 10,
                modified_unix_ms: 20,
                title: "Roadmap".into(),
                body: "Body".into(),
                tags: vec!["rmac".into()],
                folder_id: Some(folder_id),
                pinned: true,
                deleted: false,
                attachments: Vec::new(),
            }],
            attachments: Vec::new(),
        }
    }

    #[test]
    fn complete_transaction_uses_stable_ids_and_monotonic_revisions() {
        let base = snapshot();
        let mut transaction = LibraryTransaction::begin(&base).unwrap();
        let folder_id = transaction.create_folder("Ideas".into()).unwrap();
        let note_id = transaction
            .create_note(NewNote {
                created_unix_ms: 30,
                title: "New".into(),
                body: "Draft".into(),
                tags: vec!["idea".into()],
                folder_id: Some(folder_id),
            })
            .unwrap();
        transaction.set_sort_order(SortOrder::Title);

        let candidate = transaction.finish().unwrap();

        assert_eq!(candidate.revision, base.revision + 1);
        assert_eq!(folder_id.get(), 2);
        assert_eq!(note_id.get(), 2);
        assert_eq!(candidate.next_folder_id, 3);
        assert_eq!(candidate.next_note_id, 3);
        assert_eq!(candidate.sort_order, SortOrder::Title);
        assert!(candidate.validate().is_ok());
    }

    #[test]
    fn exact_record_revision_rejects_stale_edits_without_changing_the_record() {
        let base = snapshot();
        let note_id = NoteId::new(1).unwrap();
        let mut transaction = LibraryTransaction::begin(&base).unwrap();

        assert_eq!(
            transaction.edit_note(
                note_id,
                2,
                NoteChanges {
                    modified_unix_ms: 30,
                    title: "Stale".into(),
                    body: "Overwrite".into(),
                    tags: Vec::new(),
                }
            ),
            Err(MutationError::RevisionConflict)
        );
        assert_eq!(transaction.finish().unwrap_err(), MutationError::NoChanges);
    }

    #[test]
    fn deleting_a_folder_moves_live_notes_but_retains_trashed_restore_context() {
        let mut base = snapshot();
        let folder_id = FolderId::new(1).unwrap();
        let mut trashed = base.notes[0].clone();
        trashed.id = NoteId::new(2).unwrap();
        trashed.revision = 1;
        trashed.deleted = true;
        trashed.pinned = false;
        base.notes.push(trashed);
        base.next_note_id = 3;
        let mut transaction = LibraryTransaction::begin(&base).unwrap();

        assert_eq!(transaction.delete_folder(folder_id, 2).unwrap(), 1);
        let candidate = transaction.finish().unwrap();

        assert!(candidate.folders[0].deleted);
        assert_eq!(candidate.notes[0].folder_id, None);
        assert_eq!(candidate.notes[0].revision, 4);
        assert_eq!(candidate.notes[1].folder_id, Some(folder_id));
        assert!(candidate.validate().is_ok());
    }

    #[test]
    fn trash_and_restore_are_separate_revision_checked_transitions() {
        let base = snapshot();
        let note_id = NoteId::new(1).unwrap();
        let mut trash = LibraryTransaction::begin(&base).unwrap();
        trash.trash_note(note_id, 3).unwrap();
        let trashed = trash.finish().unwrap();
        assert!(trashed.notes[0].deleted);
        assert!(!trashed.notes[0].pinned);

        let mut restore = LibraryTransaction::begin(&trashed).unwrap();
        assert_eq!(
            restore.restore_note(note_id, 4).unwrap(),
            Some(FolderId::new(1).unwrap())
        );
        let restored = restore.finish().unwrap();
        assert!(!restored.notes[0].deleted);
        assert_eq!(restored.notes[0].revision, 5);
    }

    fn new_attachment() -> NewAttachment {
        NewAttachment {
            display_name: "private-roadmap.png".into(),
            kind: AttachmentKind::Png,
            byte_len: 4096,
            sha256: [9; 32],
        }
    }

    #[test]
    fn attachment_import_is_one_exact_revision_checked_candidate() {
        let base = snapshot();
        let note_id = NoteId::new(1).unwrap();
        let mut transaction = LibraryTransaction::begin(&base).unwrap();

        let plan = transaction
            .add_attachment(note_id, 3, 25, new_attachment())
            .unwrap();
        let candidate = transaction.finish().unwrap();

        assert_eq!(plan.base_library_revision, 7);
        assert_eq!(plan.candidate_library_revision, 8);
        assert_eq!(plan.note_id, note_id);
        assert_eq!(plan.base_note_revision, 3);
        assert_eq!(plan.candidate_note_revision, 4);
        assert_eq!(plan.attachment_id, AttachmentId::new(1).unwrap());
        assert_eq!(plan.byte_len, 4096);
        assert_eq!(candidate.next_attachment_id, 2);
        assert_eq!(candidate.notes[0].revision, 4);
        assert_eq!(candidate.notes[0].modified_unix_ms, 25);
        assert_eq!(candidate.notes[0].attachments, vec![plan.attachment_id]);
        assert_eq!(candidate.attachments.len(), 1);
        assert_eq!(candidate.attachments[0].id, plan.attachment_id);
        assert_eq!(candidate.attachments[0].note_id, note_id);
        assert_eq!(candidate.attachments[0].display_name, "private-roadmap.png");
        assert!(!candidate.attachments[0].deleted);
        plan.validate_candidate(&base, &candidate).unwrap();
    }

    #[test]
    fn attachment_import_rejects_stale_deleted_invalid_and_mixed_mutations() {
        let base = snapshot();
        let note_id = NoteId::new(1).unwrap();

        let mut stale = LibraryTransaction::begin(&base).unwrap();
        assert_eq!(
            stale.add_attachment(note_id, 2, 25, new_attachment()),
            Err(MutationError::RevisionConflict)
        );
        assert_eq!(stale.finish().unwrap_err(), MutationError::NoChanges);

        let mut invalid_name = LibraryTransaction::begin(&base).unwrap();
        let mut attachment = new_attachment();
        attachment.display_name = "../private.png".into();
        assert_eq!(
            invalid_name.add_attachment(note_id, 3, 25, attachment),
            Err(MutationError::InvalidCandidate(
                ValidationError::InvalidName
            ))
        );

        let mut invalid_bytes = LibraryTransaction::begin(&base).unwrap();
        let mut attachment = new_attachment();
        attachment.byte_len = 0;
        assert_eq!(
            invalid_bytes.add_attachment(note_id, 3, 25, attachment),
            Err(MutationError::InvalidCandidate(
                ValidationError::InvalidAttachment
            ))
        );

        let mut backwards_time = LibraryTransaction::begin(&base).unwrap();
        assert_eq!(
            backwards_time.add_attachment(note_id, 3, 19, new_attachment()),
            Err(MutationError::InvalidCandidate(
                ValidationError::InvalidTimestamp
            ))
        );

        let mut deleted_base = base.clone();
        deleted_base.notes[0].pinned = false;
        deleted_base.notes[0].deleted = true;
        let mut deleted = LibraryTransaction::begin(&deleted_base).unwrap();
        assert_eq!(
            deleted.add_attachment(note_id, 3, 25, new_attachment()),
            Err(MutationError::DeletedNote)
        );

        let mut mixed = LibraryTransaction::begin(&base).unwrap();
        assert!(mixed.set_sort_order(SortOrder::Title));
        assert_eq!(
            mixed.add_attachment(note_id, 3, 25, new_attachment()),
            Err(MutationError::AttachmentImportRequiresExclusiveTransaction)
        );
    }

    #[test]
    fn attachment_import_plan_rederives_full_candidate_and_redacts_private_data() {
        let base = snapshot();
        let note_id = NoteId::new(1).unwrap();
        let attachment = new_attachment();
        let mut transaction = LibraryTransaction::begin(&base).unwrap();
        let plan = transaction
            .add_attachment(note_id, 3, 25, attachment.clone())
            .unwrap();
        let candidate = transaction.finish().unwrap();

        let mut unrelated_change = candidate.clone();
        unrelated_change.notes[0].title = "Changed too".into();
        assert_eq!(
            plan.validate_candidate(&base, &unrelated_change),
            Err(MutationError::InvalidCandidate(
                ValidationError::InconsistentAttachment
            ))
        );

        let mut changed_bytes = candidate.clone();
        changed_bytes.attachments[0].sha256 = [8; 32];
        assert_eq!(
            plan.validate_candidate(&base, &changed_bytes),
            Err(MutationError::InvalidCandidate(
                ValidationError::InconsistentAttachment
            ))
        );

        let attachment_debug = format!("{attachment:?}");
        let plan_debug = format!("{plan:?}");
        assert!(!attachment_debug.contains("private-roadmap.png"));
        assert!(!attachment_debug.contains("[9, 9"));
        assert!(!plan_debug.contains("[9, 9"));
        assert!(plan_debug.contains("attachment_id"));
    }

    #[test]
    fn attachment_reference_removal_tombstones_metadata_without_losing_identity() {
        let base = snapshot();
        let note_id = NoteId::new(1).unwrap();
        let mut import = LibraryTransaction::begin(&base).unwrap();
        let plan = import
            .add_attachment(note_id, 3, 25, new_attachment())
            .unwrap();
        let attached = import.finish().unwrap();
        let mut remove = LibraryTransaction::begin(&attached).unwrap();

        remove
            .remove_attachment_reference(note_id, 4, plan.attachment_id, 1, 30)
            .unwrap();
        let detached = remove.finish().unwrap();

        assert_eq!(detached.revision, attached.revision + 1);
        assert_eq!(detached.notes[0].revision, 5);
        assert_eq!(detached.notes[0].modified_unix_ms, 30);
        assert!(detached.notes[0].attachments.is_empty());
        assert_eq!(detached.attachments.len(), 1);
        assert_eq!(detached.attachments[0].id, plan.attachment_id);
        assert_eq!(detached.attachments[0].revision, 2);
        assert!(detached.attachments[0].deleted);
        assert_eq!(detached.next_attachment_id, 2);
        detached.validate().unwrap();

        let mut repeated = LibraryTransaction::begin(&detached).unwrap();
        assert_eq!(
            repeated.remove_attachment_reference(note_id, 5, plan.attachment_id, 2, 31),
            Err(MutationError::AttachmentAlreadyRemoved)
        );
        assert_eq!(repeated.finish().unwrap_err(), MutationError::NoChanges);
    }

    #[test]
    fn attachment_reference_removal_rejects_stale_wrong_owner_and_trash() {
        let base = snapshot();
        let note_id = NoteId::new(1).unwrap();
        let mut import = LibraryTransaction::begin(&base).unwrap();
        let plan = import
            .add_attachment(note_id, 3, 25, new_attachment())
            .unwrap();
        let attached = import.finish().unwrap();

        let mut stale_note = LibraryTransaction::begin(&attached).unwrap();
        assert_eq!(
            stale_note.remove_attachment_reference(note_id, 3, plan.attachment_id, 1, 30),
            Err(MutationError::RevisionConflict)
        );
        let mut stale_attachment = LibraryTransaction::begin(&attached).unwrap();
        assert_eq!(
            stale_attachment.remove_attachment_reference(note_id, 4, plan.attachment_id, 2, 30),
            Err(MutationError::RevisionConflict)
        );

        let mut second_note = attached.clone();
        let mut other = second_note.notes[0].clone();
        other.id = NoteId::new(2).unwrap();
        other.revision = 1;
        other.attachments.clear();
        second_note.notes.push(other);
        second_note.next_note_id = 3;
        second_note.validate().unwrap();
        let mut wrong_owner = LibraryTransaction::begin(&second_note).unwrap();
        assert_eq!(
            wrong_owner.remove_attachment_reference(
                NoteId::new(2).unwrap(),
                1,
                plan.attachment_id,
                1,
                30,
            ),
            Err(MutationError::AttachmentNotOwned)
        );

        let mut trash = LibraryTransaction::begin(&attached).unwrap();
        trash.trash_note(note_id, 4).unwrap();
        let trashed = trash.finish().unwrap();
        let mut remove_from_trash = LibraryTransaction::begin(&trashed).unwrap();
        assert_eq!(
            remove_from_trash.remove_attachment_reference(note_id, 5, plan.attachment_id, 1, 30,),
            Err(MutationError::DeletedNote)
        );
    }

    #[test]
    fn permanent_delete_requires_trash_and_returns_exact_cleanup_identities() {
        let mut base = snapshot();
        let note_id = NoteId::new(1).unwrap();
        let attachment_id = AttachmentId::new(1).unwrap();
        base.notes[0].pinned = false;
        base.notes[0].deleted = true;
        base.notes[0].attachments.push(attachment_id);
        base.next_attachment_id = 2;
        base.attachments.push(AttachmentRecord {
            id: attachment_id,
            revision: 1,
            note_id,
            display_name: "diagram.png".into(),
            kind: AttachmentKind::Png,
            byte_len: 99,
            sha256: [1; 32],
            deleted: false,
        });
        base.validate().unwrap();

        let mut transaction = LibraryTransaction::begin(&base).unwrap();
        let plan = transaction.purge_trashed_note(note_id, 3).unwrap();
        let candidate = transaction.finish().unwrap();

        assert_eq!(plan.base_library_revision, 7);
        assert_eq!(plan.candidate_library_revision, 8);
        assert_eq!(plan.note_ids, vec![note_id]);
        assert_eq!(plan.attachment_ids, vec![attachment_id]);
        assert_eq!(plan.attachment_bytes, 99);
        assert!(candidate.notes.is_empty());
        assert!(candidate.attachments.is_empty());
        assert_eq!(candidate.next_note_id, 2);
        assert_eq!(candidate.next_attachment_id, 2);
        candidate.validate().unwrap();

        let mut live = LibraryTransaction::begin(&snapshot()).unwrap();
        assert_eq!(
            live.purge_trashed_note(note_id, 3),
            Err(MutationError::NoteNotTrashed)
        );
        assert_eq!(live.finish().unwrap_err(), MutationError::NoChanges);
    }

    #[test]
    fn empty_trash_is_exact_revision_bounded_and_includes_owned_orphans() {
        let mut base = snapshot();
        let live_note_id = NoteId::new(1).unwrap();
        let trashed_note_id = NoteId::new(2).unwrap();
        let live_attachment_id = AttachmentId::new(1).unwrap();
        let orphan_attachment_id = AttachmentId::new(2).unwrap();
        let mut trashed = base.notes[0].clone();
        trashed.id = trashed_note_id;
        trashed.revision = 1;
        trashed.pinned = false;
        trashed.deleted = true;
        trashed.attachments = vec![live_attachment_id];
        base.notes.push(trashed);
        base.next_note_id = 3;
        base.next_attachment_id = 3;
        base.attachments.extend([
            AttachmentRecord {
                id: live_attachment_id,
                revision: 1,
                note_id: trashed_note_id,
                display_name: "kept-until-purge.png".into(),
                kind: AttachmentKind::Png,
                byte_len: 10,
                sha256: [1; 32],
                deleted: false,
            },
            AttachmentRecord {
                id: orphan_attachment_id,
                revision: 2,
                note_id: trashed_note_id,
                display_name: "orphaned-before-purge.png".into(),
                kind: AttachmentKind::Png,
                byte_len: 20,
                sha256: [2; 32],
                deleted: true,
            },
        ]);
        base.validate().unwrap();

        let mut mixed = LibraryTransaction::begin(&base).unwrap();
        assert!(mixed.set_sort_order(SortOrder::Title));
        assert_eq!(
            mixed.empty_trash(7),
            Err(MutationError::PurgeRequiresExclusiveTransaction)
        );

        let mut transaction = LibraryTransaction::begin(&base).unwrap();
        assert_eq!(
            transaction.empty_trash(6),
            Err(MutationError::RevisionConflict)
        );
        let plan = transaction.empty_trash(7).unwrap();
        let candidate = transaction.finish().unwrap();

        assert_eq!(plan.note_ids, vec![trashed_note_id]);
        assert_eq!(
            plan.attachment_ids,
            vec![live_attachment_id, orphan_attachment_id]
        );
        assert_eq!(plan.attachment_bytes, 30);
        assert_eq!(candidate.notes.len(), 1);
        assert_eq!(candidate.notes[0].id, live_note_id);
        assert!(candidate.attachments.is_empty());
        candidate.validate().unwrap();

        let mut empty = LibraryTransaction::begin(&candidate).unwrap();
        assert_eq!(
            empty.empty_trash(candidate.revision),
            Err(MutationError::NoChanges)
        );
        assert_eq!(empty.finish().unwrap_err(), MutationError::NoChanges);
    }

    #[test]
    fn invalid_content_missing_folders_and_identity_exhaustion_fail_closed() {
        let base = snapshot();
        let mut invalid = LibraryTransaction::begin(&base).unwrap();
        assert_eq!(
            invalid.create_note(NewNote {
                created_unix_ms: 1,
                title: "Bad\nTitle".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id: None,
            }),
            Err(MutationError::InvalidCandidate(
                ValidationError::InvalidText
            ))
        );
        assert_eq!(
            invalid.create_note(NewNote {
                created_unix_ms: 1,
                title: "Missing folder".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id: FolderId::new(99),
            }),
            Err(MutationError::MissingFolder)
        );

        let mut exhausted = base;
        exhausted.next_note_id = u64::MAX - 1;
        let mut transaction = LibraryTransaction::begin(&exhausted).unwrap();
        assert_eq!(
            transaction.create_note(NewNote {
                created_unix_ms: 1,
                title: "No identity".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id: None,
            }),
            Err(MutationError::IdentityExhausted)
        );
    }
}
