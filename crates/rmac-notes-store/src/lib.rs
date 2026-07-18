//! Versioned, bounded domain records for the local rmac Notes library.
//!
//! This crate contains no UI, filesystem, portal, or async runtime. A storage
//! adapter can validate and encode a complete transaction candidate before it
//! changes durable state, and can decode records without lossy fallbacks or
//! unbounded allocation.

mod bundle_import;
mod codec;
mod export;
mod mutation;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub use bundle_import::{
    BundleAttachmentImport, BundleCollisionPolicy, BundleImportPlan, BundleImportReview,
    BundlePlanError, PlannedBundleImport,
};
pub use codec::{decode, encode, CodecError, MAX_LIBRARY_BYTES, SCHEMA_VERSION};
pub use export::{render_export_markdown, ExportAttachment, ExportError, ExportPlan, ExportScope};
pub use mutation::{
    AttachmentImportPlan, LibraryTransaction, MutationError, NewAttachment, NewNote, NoteChanges,
    OrphanCollectionPlan, PurgePlan,
};

pub const MAX_NOTES: usize = 100_000;
pub const MAX_FOLDERS: usize = 10_000;
pub const MAX_ATTACHMENTS: usize = 200_000;
pub const MAX_TAGS_PER_NOTE: usize = 32;
pub const MAX_ATTACHMENTS_PER_NOTE: usize = 128;
pub const MAX_TITLE_BYTES: usize = 16 * 1024;
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_NAME_BYTES: usize = 1024;
pub const MAX_TAG_BYTES: usize = 256;
pub const MAX_ATTACHMENT_BYTES: u64 = 256 * 1024 * 1024;

macro_rules! stable_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);

        impl $name {
            pub fn new(value: u64) -> Option<Self> {
                (value != 0).then_some(Self(value))
            }

            pub fn get(self) -> u64 {
                self.0
            }
        }
    };
}

stable_id!(NoteId);
stable_id!(FolderId);
stable_id!(AttachmentId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    Png,
    Jpeg,
    Gif,
    WebP,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Edited,
    Created,
    Title,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRecord {
    pub id: FolderId,
    pub revision: u64,
    pub name: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentRecord {
    pub id: AttachmentId,
    pub revision: u64,
    pub note_id: NoteId,
    pub display_name: String,
    pub kind: AttachmentKind,
    pub byte_len: u64,
    pub sha256: [u8; 32],
    pub deleted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRecord {
    pub id: NoteId,
    pub revision: u64,
    pub created_unix_ms: u64,
    pub modified_unix_ms: u64,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub folder_id: Option<FolderId>,
    pub pinned: bool,
    pub deleted: bool,
    pub attachments: Vec<AttachmentId>,
}

/// Complete authoritative library snapshot. Vector order is not identity or
/// list order; encoding canonicalizes records by stable ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibrarySnapshot {
    pub revision: u64,
    pub sort_order: SortOrder,
    pub next_note_id: u64,
    pub next_folder_id: u64,
    pub next_attachment_id: u64,
    pub folders: Vec<FolderRecord>,
    pub notes: Vec<NoteRecord>,
    pub attachments: Vec<AttachmentRecord>,
}

impl Default for LibrarySnapshot {
    fn default() -> Self {
        Self {
            revision: 1,
            sort_order: SortOrder::Edited,
            next_note_id: 1,
            next_folder_id: 1,
            next_attachment_id: 1,
            folders: Vec::new(),
            notes: Vec::new(),
            attachments: Vec::new(),
        }
    }
}

impl LibrarySnapshot {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.revision == 0
            || self.folders.iter().any(|record| record.revision == 0)
            || self.notes.iter().any(|record| record.revision == 0)
            || self.attachments.iter().any(|record| record.revision == 0)
        {
            return Err(ValidationError::InvalidRevision);
        }
        if self.notes.len() > MAX_NOTES
            || self.folders.len() > MAX_FOLDERS
            || self.attachments.len() > MAX_ATTACHMENTS
        {
            return Err(ValidationError::CollectionLimit);
        }

        let folders = unique_map(self.folders.iter().map(|record| (record.id, record)))?;
        let notes = unique_map(self.notes.iter().map(|record| (record.id, record)))?;
        let attachments = unique_map(self.attachments.iter().map(|record| (record.id, record)))?;
        validate_next_id(self.next_folder_id, folders.keys().map(|id| id.get()).max())?;
        validate_next_id(self.next_note_id, notes.keys().map(|id| id.get()).max())?;
        validate_next_id(
            self.next_attachment_id,
            attachments.keys().map(|id| id.get()).max(),
        )?;

        let mut folder_names = BTreeSet::new();
        for folder in &self.folders {
            validate_name(&folder.name)?;
            if !folder.deleted && !folder_names.insert(folder.name.to_lowercase()) {
                return Err(ValidationError::DuplicateName);
            }
        }

        for note in &self.notes {
            if note.title.len() > MAX_TITLE_BYTES
                || note.body.len() > MAX_BODY_BYTES
                || note.title.chars().any(char::is_control)
                || note.body.contains('\0')
            {
                return Err(ValidationError::InvalidText);
            }
            if note.modified_unix_ms < note.created_unix_ms {
                return Err(ValidationError::InvalidTimestamp);
            }
            if note.pinned && note.deleted {
                return Err(ValidationError::InvalidDeletedState);
            }
            if let Some(folder_id) = note.folder_id {
                let folder = folders
                    .get(&folder_id)
                    .ok_or(ValidationError::MissingReference)?;
                if !note.deleted && folder.deleted {
                    return Err(ValidationError::InvalidDeletedState);
                }
            }
            if note.tags.len() > MAX_TAGS_PER_NOTE
                || note.attachments.len() > MAX_ATTACHMENTS_PER_NOTE
            {
                return Err(ValidationError::CollectionLimit);
            }
            let mut tags = BTreeSet::new();
            for tag in &note.tags {
                validate_tag(tag)?;
                if !tags.insert(tag.to_lowercase()) {
                    return Err(ValidationError::DuplicateName);
                }
            }
            let mut note_attachments = BTreeSet::new();
            for attachment_id in &note.attachments {
                if !note_attachments.insert(*attachment_id) {
                    return Err(ValidationError::DuplicateId);
                }
                let attachment = attachments
                    .get(attachment_id)
                    .ok_or(ValidationError::MissingReference)?;
                if attachment.note_id != note.id || attachment.deleted {
                    return Err(ValidationError::InconsistentAttachment);
                }
            }
        }

        for attachment in &self.attachments {
            validate_name(&attachment.display_name)?;
            if attachment.byte_len == 0
                || attachment.byte_len > MAX_ATTACHMENT_BYTES
                || attachment.sha256 == [0; 32]
            {
                return Err(ValidationError::InvalidAttachment);
            }
            let owner = notes
                .get(&attachment.note_id)
                .ok_or(ValidationError::MissingReference)?;
            let referenced = owner.attachments.contains(&attachment.id);
            if referenced == attachment.deleted {
                return Err(ValidationError::InconsistentAttachment);
            }
        }
        Ok(())
    }
}

fn unique_map<K: Ord, V>(
    records: impl Iterator<Item = (K, V)>,
) -> Result<BTreeMap<K, V>, ValidationError> {
    let mut map = BTreeMap::new();
    for (id, record) in records {
        if map.insert(id, record).is_some() {
            return Err(ValidationError::DuplicateId);
        }
    }
    Ok(map)
}

fn validate_next_id(next: u64, maximum: Option<u64>) -> Result<(), ValidationError> {
    if next == 0 || next == u64::MAX || maximum.is_some_and(|maximum| next <= maximum) {
        Err(ValidationError::InvalidNextId)
    } else {
        Ok(())
    }
}

fn validate_name(value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > MAX_NAME_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.contains('/')
        || value.contains('\\')
        || matches!(value, "." | "..")
    {
        Err(ValidationError::InvalidName)
    } else {
        Ok(())
    }
}

fn validate_tag(value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > MAX_TAG_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(ValidationError::InvalidTag)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    InvalidRevision,
    CollectionLimit,
    DuplicateId,
    InvalidNextId,
    InvalidName,
    DuplicateName,
    InvalidText,
    InvalidTag,
    InvalidTimestamp,
    MissingReference,
    InvalidDeletedState,
    InvalidAttachment,
    InconsistentAttachment,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRevision => "the Notes library contains an invalid revision",
            Self::CollectionLimit => "the Notes library exceeds a collection safety limit",
            Self::DuplicateId => "the Notes library contains a duplicate identity",
            Self::InvalidNextId => "the Notes library identity sequence is invalid",
            Self::InvalidName => "the Notes library contains an invalid name",
            Self::DuplicateName => "the Notes library contains a duplicate normalized name",
            Self::InvalidText => "the Notes library contains invalid or excessive text",
            Self::InvalidTag => "the Notes library contains an invalid tag",
            Self::InvalidTimestamp => "the Notes library contains an invalid timestamp",
            Self::MissingReference => "the Notes library contains a missing reference",
            Self::InvalidDeletedState => "the Notes library contains an invalid deletion state",
            Self::InvalidAttachment => "the Notes library contains invalid attachment metadata",
            Self::InconsistentAttachment => {
                "the Notes library contains an inconsistent attachment reference"
            }
        })
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn fixture() -> LibrarySnapshot {
        let note_id = NoteId::new(1).unwrap();
        let folder_id = FolderId::new(1).unwrap();
        let attachment_id = AttachmentId::new(1).unwrap();
        LibrarySnapshot {
            revision: 4,
            sort_order: SortOrder::Title,
            next_note_id: 2,
            next_folder_id: 2,
            next_attachment_id: 2,
            folders: vec![FolderRecord {
                id: folder_id,
                revision: 1,
                name: "Projects".into(),
                deleted: false,
            }],
            notes: vec![NoteRecord {
                id: note_id,
                revision: 3,
                created_unix_ms: 10,
                modified_unix_ms: 20,
                title: "Roadmap".into(),
                body: "- [ ] transaction store\nनमस्ते".into(),
                tags: vec!["rmac".into(), "Planning".into()],
                folder_id: Some(folder_id),
                pinned: true,
                deleted: false,
                attachments: vec![attachment_id],
            }],
            attachments: vec![AttachmentRecord {
                id: attachment_id,
                revision: 1,
                note_id,
                display_name: "diagram.png".into(),
                kind: AttachmentKind::Png,
                byte_len: 2048,
                sha256: [7; 32],
                deleted: false,
            }],
        }
    }

    #[test]
    fn stable_ids_are_nonzero_and_path_independent() {
        assert_eq!(NoteId::new(0), None);
        assert_eq!(NoteId::new(42).unwrap().get(), 42);
        assert!(fixture().validate().is_ok());
    }

    #[test]
    fn duplicate_and_dangling_identities_fail_closed() {
        let mut duplicate = fixture();
        duplicate.notes.push(duplicate.notes[0].clone());
        assert_eq!(duplicate.validate(), Err(ValidationError::DuplicateId));

        let mut dangling = fixture();
        dangling.notes[0].folder_id = Some(FolderId::new(99).unwrap());
        assert_eq!(dangling.validate(), Err(ValidationError::MissingReference));
    }

    #[test]
    fn attachment_ownership_and_deleted_state_are_exact() {
        let mut wrong_owner = fixture();
        wrong_owner.attachments[0].note_id = NoteId::new(2).unwrap();
        assert_eq!(
            wrong_owner.validate(),
            Err(ValidationError::InconsistentAttachment)
        );

        let mut pinned_trash = fixture();
        pinned_trash.notes[0].deleted = true;
        assert_eq!(
            pinned_trash.validate(),
            Err(ValidationError::InvalidDeletedState)
        );
    }

    #[test]
    fn names_tags_text_and_sequences_are_bounded() {
        let mut invalid = fixture();
        invalid.folders[0].name = "../escape".into();
        assert_eq!(invalid.validate(), Err(ValidationError::InvalidName));

        invalid.folders[0].name = " bad ".into();
        assert_eq!(invalid.validate(), Err(ValidationError::InvalidName));

        let mut duplicate_tag = fixture();
        duplicate_tag.notes[0].tags.push("RMAC".into());
        assert_eq!(
            duplicate_tag.validate(),
            Err(ValidationError::DuplicateName)
        );

        let mut sequence = fixture();
        sequence.next_note_id = 1;
        assert_eq!(sequence.validate(), Err(ValidationError::InvalidNextId));
    }
}
