use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::lock::{
    hint_valid, kdf_params_valid, LOCK_TAG_BYTES, SEALED_ATTACHMENT_OVERHEAD,
    VERIFIER_CIPHERTEXT_BYTES,
};
use crate::{
    LibrarySnapshot, MAX_ATTACHMENTS, MAX_ATTACHMENTS_PER_NOTE, MAX_ATTACHMENT_BYTES,
    MAX_BODY_BYTES, MAX_FOLDERS, MAX_LOCK_KEYS, MAX_NAME_BYTES, MAX_NOTES, MAX_SEALED_NOTE_BYTES,
    MAX_SMART_FOLDERS, MAX_TAGS_PER_NOTE, MAX_TAG_BYTES, MAX_TITLE_BYTES, SEALED_ATTACHMENT_NAME,
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
            || self.smart_folders.len() > MAX_SMART_FOLDERS
            || self.lock_keys.len() > MAX_LOCK_KEYS
        {
            return Err(ValidationError::CollectionLimit);
        }
        self.validate_lock_keys()?;
        self.validate_smart_folders()?;

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
            if let Some(lock) = &note.lock {
                if self.lock_key(lock.key_id).is_none() {
                    return Err(ValidationError::MissingReference);
                }
                if lock.sealed.ciphertext.len() < LOCK_TAG_BYTES
                    || lock.sealed.ciphertext.len() > MAX_SEALED_NOTE_BYTES
                {
                    return Err(ValidationError::InvalidLock);
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
            if let Some(key_id) = attachment.sealed_key {
                if self.lock_key(key_id).is_none() {
                    return Err(ValidationError::MissingReference);
                }
                if attachment.byte_len <= SEALED_ATTACHMENT_OVERHEAD {
                    return Err(ValidationError::InvalidAttachment);
                }
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

    /// The durable form of a library: every locked note's title, body and
    /// tags are empty (its sealed payload holds them) and every sealed
    /// attachment carries only the generic name. Encoding refuses anything
    /// else, so plaintext of a locked note can never be written by mistake.
    /// A decrypted in-memory view for the UI deliberately fails this check.
    pub fn validate_at_rest(&self) -> Result<(), ValidationError> {
        for note in &self.notes {
            if note.lock.is_some()
                && (!note.title.is_empty() || !note.body.is_empty() || !note.tags.is_empty())
            {
                return Err(ValidationError::PlaintextInLockedNote);
            }
        }
        for attachment in &self.attachments {
            if attachment.sealed_key.is_some() && attachment.display_name != SEALED_ATTACHMENT_NAME
            {
                return Err(ValidationError::PlaintextInLockedNote);
            }
        }
        Ok(())
    }

    fn validate_lock_keys(&self) -> Result<(), ValidationError> {
        let mut ids = BTreeSet::new();
        for key in &self.lock_keys {
            if key.id == 0 || key.id >= self.next_lock_key_id || !ids.insert(key.id) {
                return Err(ValidationError::InvalidLock);
            }
            if !kdf_params_valid(key.kdf)
                || !hint_valid(&key.hint)
                || key.verifier.ciphertext.len() != VERIFIER_CIPHERTEXT_BYTES
            {
                return Err(ValidationError::InvalidLock);
            }
        }
        if self.next_lock_key_id == 0 || self.next_lock_key_id == u32::MAX {
            return Err(ValidationError::InvalidNextId);
        }
        if self
            .current_lock_key
            .is_some_and(|current| !ids.contains(&current))
        {
            return Err(ValidationError::MissingReference);
        }
        Ok(())
    }

    fn validate_smart_folders(&self) -> Result<(), ValidationError> {
        let ids = unique_map(self.smart_folders.iter().map(|record| (record.id, record)))?;
        validate_next_id(
            self.next_smart_folder_id,
            ids.keys().map(|id| id.get()).max(),
        )?;
        let mut names = BTreeSet::new();
        for folder in &self.smart_folders {
            validate_name(&folder.name)?;
            validate_tag(&folder.tag)?;
            if !names.insert(folder.name.to_lowercase()) {
                return Err(ValidationError::DuplicateName);
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

/// Whether `value` is acceptable as a folder, Smart Folder or attachment
/// display name.
pub fn is_valid_name(value: &str) -> bool {
    validate_name(value).is_ok()
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
    InvalidLock,
    PlaintextInLockedNote,
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
            Self::InvalidLock => "the Notes library contains an invalid locked-note record",
            Self::PlaintextInLockedNote => {
                "a locked note's plaintext must never be stored in the Notes library"
            }
        })
    }
}

impl std::error::Error for ValidationError {}
