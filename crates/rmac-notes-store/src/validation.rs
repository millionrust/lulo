use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::{
    LibrarySnapshot, MAX_ATTACHMENTS, MAX_ATTACHMENTS_PER_NOTE, MAX_ATTACHMENT_BYTES,
    MAX_BODY_BYTES, MAX_FOLDERS, MAX_NAME_BYTES, MAX_NOTES, MAX_TAGS_PER_NOTE, MAX_TAG_BYTES,
    MAX_TITLE_BYTES,
};

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

pub(crate) fn validate_name(value: &str) -> Result<(), ValidationError> {
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

pub(crate) fn validate_tag(value: &str) -> Result<(), ValidationError> {
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
