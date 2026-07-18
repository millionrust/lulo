use std::fmt;
use std::io;
use std::path::Path;

use rmac_notes_store::{
    encode, AttachmentId, LibrarySnapshot, NoteId, OrphanCollectionPlan, MAX_ATTACHMENT_BYTES,
};
use rmac_storage::{Backend, FileFingerprint};
use sha2::{Digest as _, Sha256};

use crate::managed_attachment_path;

const ORPHAN_MAGIC: &[u8; 8] = b"RMNORPH\0";
const ORPHAN_VERSION: u16 = 1;
pub(crate) const MAX_ORPHAN_INTENT_BYTES: usize = 8 + 2 + 16 + 64 + 8 + 8 + 8 + 8 + 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrphanError {
    InvalidPlan,
    Malformed,
    Io(io::ErrorKind),
    Remove(io::ErrorKind),
    ReadbackMismatch,
    AttachmentMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrphanAuthority {
    RolledBack,
    Accepted,
    AcceptedDescendant,
    Ambiguous,
}

/// Private durable authority for collecting one exact orphaned attachment.
///
/// The base/candidate hashes bind metadata authority while the attachment
/// identity binds the only filesystem object that may be removed. Debug output
/// deliberately omits both hashes.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct OrphanIntent {
    base_library_revision: u64,
    candidate_library_revision: u64,
    base_sha256: [u8; 32],
    candidate_sha256: [u8; 32],
    attachment_id: AttachmentId,
    attachment_revision: u64,
    owner_note_id: NoteId,
    byte_len: u64,
    sha256: [u8; 32],
}

impl OrphanIntent {
    pub(crate) fn prepare(
        base: &LibrarySnapshot,
        candidate: &LibrarySnapshot,
        plan: &OrphanCollectionPlan,
    ) -> Result<Self, OrphanError> {
        plan.validate_candidate(base, candidate)
            .map_err(|_| OrphanError::InvalidPlan)?;
        let attachment = base
            .attachments
            .iter()
            .find(|attachment| attachment.id == plan.attachment_id)
            .ok_or(OrphanError::InvalidPlan)?;
        if attachment.revision != plan.attachment_revision
            || attachment.note_id != plan.owner_note_id
            || attachment.byte_len != plan.byte_len
            || attachment.sha256 != plan.sha256
            || !attachment.deleted
        {
            return Err(OrphanError::InvalidPlan);
        }
        let base_bytes = encode(base).map_err(|_| OrphanError::InvalidPlan)?;
        let candidate_bytes = encode(candidate).map_err(|_| OrphanError::InvalidPlan)?;
        Ok(Self {
            base_library_revision: base.revision,
            candidate_library_revision: candidate.revision,
            base_sha256: digest(&base_bytes),
            candidate_sha256: digest(&candidate_bytes),
            attachment_id: attachment.id,
            attachment_revision: attachment.revision,
            owner_note_id: attachment.note_id,
            byte_len: attachment.byte_len,
            sha256: attachment.sha256,
        })
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_ORPHAN_INTENT_BYTES);
        bytes.extend_from_slice(ORPHAN_MAGIC);
        bytes.extend_from_slice(&ORPHAN_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.base_library_revision.to_le_bytes());
        bytes.extend_from_slice(&self.candidate_library_revision.to_le_bytes());
        bytes.extend_from_slice(&self.base_sha256);
        bytes.extend_from_slice(&self.candidate_sha256);
        bytes.extend_from_slice(&self.attachment_id.get().to_le_bytes());
        bytes.extend_from_slice(&self.attachment_revision.to_le_bytes());
        bytes.extend_from_slice(&self.owner_note_id.get().to_le_bytes());
        bytes.extend_from_slice(&self.byte_len.to_le_bytes());
        bytes.extend_from_slice(&self.sha256);
        bytes
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, OrphanError> {
        if bytes.len() != MAX_ORPHAN_INTENT_BYTES {
            return Err(OrphanError::Malformed);
        }
        let mut reader = Reader::new(bytes);
        if reader.take(8)? != ORPHAN_MAGIC || reader.u16()? != ORPHAN_VERSION {
            return Err(OrphanError::Malformed);
        }
        let base_library_revision = reader.u64()?;
        let candidate_library_revision = reader.u64()?;
        let base_sha256 = reader.array()?;
        let candidate_sha256 = reader.array()?;
        let attachment_id = AttachmentId::new(reader.u64()?).ok_or(OrphanError::Malformed)?;
        let attachment_revision = reader.u64()?;
        let owner_note_id = NoteId::new(reader.u64()?).ok_or(OrphanError::Malformed)?;
        let byte_len = reader.u64()?;
        let sha256 = reader.array()?;
        if !reader.is_empty()
            || base_library_revision == 0
            || candidate_library_revision != base_library_revision.checked_add(1).unwrap_or(0)
            || attachment_revision == 0
            || byte_len == 0
            || byte_len > MAX_ATTACHMENT_BYTES
            || base_sha256 == [0; 32]
            || candidate_sha256 == [0; 32]
            || sha256 == [0; 32]
        {
            return Err(OrphanError::Malformed);
        }
        Ok(Self {
            base_library_revision,
            candidate_library_revision,
            base_sha256,
            candidate_sha256,
            attachment_id,
            attachment_revision,
            owner_note_id,
            byte_len,
            sha256,
        })
    }

    pub(crate) fn authority(
        &self,
        snapshot: &LibrarySnapshot,
    ) -> Result<OrphanAuthority, OrphanError> {
        let encoded = encode(snapshot).map_err(|_| OrphanError::Malformed)?;
        let hash = digest(&encoded);
        if snapshot.revision == self.base_library_revision && hash == self.base_sha256 {
            return Ok(OrphanAuthority::RolledBack);
        }
        if snapshot.revision == self.candidate_library_revision && hash == self.candidate_sha256 {
            return Ok(OrphanAuthority::Accepted);
        }
        if snapshot.revision > self.candidate_library_revision
            && snapshot
                .attachments
                .iter()
                .all(|attachment| attachment.id != self.attachment_id)
        {
            return Ok(OrphanAuthority::AcceptedDescendant);
        }
        Ok(OrphanAuthority::Ambiguous)
    }

    pub(crate) fn cleanup<B: Backend>(&self, root: &Path, backend: &B) -> Result<(), OrphanError> {
        let path = managed_attachment_path(root, self.attachment_id);
        let fingerprint = match backend.fingerprint_bounded_no_follow(&path, self.byte_len) {
            Ok(fingerprint) => fingerprint,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return Err(OrphanError::AttachmentMismatch)
            }
            Err(error) => return Err(OrphanError::Io(error.kind())),
        };
        if fingerprint
            != (FileFingerprint {
                byte_len: self.byte_len,
                sha256: self.sha256,
            })
        {
            return Err(OrphanError::AttachmentMismatch);
        }
        backend
            .remove_file_durable(&path)
            .map_err(|error| OrphanError::Remove(error.kind()))?;
        match backend.fingerprint_bounded_no_follow(&path, self.byte_len) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            _ => Err(OrphanError::ReadbackMismatch),
        }
    }
}

impl fmt::Debug for OrphanIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OrphanIntent")
            .field("base_library_revision", &self.base_library_revision)
            .field(
                "candidate_library_revision",
                &self.candidate_library_revision,
            )
            .field("attachment_id", &self.attachment_id)
            .field("attachment_revision", &self.attachment_revision)
            .field("owner_note_id", &self.owner_note_id)
            .field("byte_len", &self.byte_len)
            .field("sha256", &"<redacted>")
            .finish()
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], OrphanError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or(OrphanError::Malformed)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(OrphanError::Malformed)?;
        self.cursor = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, OrphanError> {
        self.take(2)?
            .try_into()
            .map(u16::from_le_bytes)
            .map_err(|_| OrphanError::Malformed)
    }

    fn u64(&mut self) -> Result<u64, OrphanError> {
        self.take(8)?
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| OrphanError::Malformed)
    }

    fn array(&mut self) -> Result<[u8; 32], OrphanError> {
        self.take(32)?
            .try_into()
            .map_err(|_| OrphanError::Malformed)
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notes_store::{AttachmentKind, AttachmentRecord, NoteRecord, SortOrder};

    fn fixture() -> (LibrarySnapshot, LibrarySnapshot, OrphanCollectionPlan) {
        let note_id = NoteId::new(4).unwrap();
        let attachment_id = AttachmentId::new(7).unwrap();
        let base = LibrarySnapshot {
            revision: 9,
            sort_order: SortOrder::Edited,
            next_note_id: 5,
            next_folder_id: 1,
            next_attachment_id: 8,
            folders: Vec::new(),
            notes: vec![NoteRecord {
                id: note_id,
                revision: 3,
                created_unix_ms: 10,
                modified_unix_ms: 20,
                title: "Private".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id: None,
                pinned: false,
                deleted: false,
                attachments: Vec::new(),
            }],
            attachments: vec![AttachmentRecord {
                id: attachment_id,
                revision: 2,
                note_id,
                display_name: "private.png".into(),
                kind: AttachmentKind::Png,
                byte_len: 13,
                sha256: [8; 32],
                deleted: true,
            }],
        };
        let mut transaction = rmac_notes_store::LibraryTransaction::begin(&base).unwrap();
        let plan = transaction
            .collect_orphaned_attachment(attachment_id, 2)
            .unwrap();
        let candidate = transaction.finish().unwrap();
        (base, candidate, plan)
    }

    #[test]
    fn intent_round_trips_and_classifies_exact_authority() {
        let (base, candidate, plan) = fixture();
        let intent = OrphanIntent::prepare(&base, &candidate, &plan).unwrap();
        let bytes = intent.encode();

        assert_eq!(bytes.len(), MAX_ORPHAN_INTENT_BYTES);
        assert_eq!(OrphanIntent::decode(&bytes).unwrap(), intent);
        assert_eq!(
            intent.authority(&base).unwrap(),
            OrphanAuthority::RolledBack
        );
        assert_eq!(
            intent.authority(&candidate).unwrap(),
            OrphanAuthority::Accepted
        );
        assert!(!format!("{intent:?}").contains("[8, 8"));
    }

    #[test]
    fn intent_rejects_inexact_candidates_and_malformed_bytes() {
        let (base, candidate, plan) = fixture();
        let mut changed = candidate.clone();
        changed.notes[0].title = "Also changed".into();
        assert_eq!(
            OrphanIntent::prepare(&base, &changed, &plan),
            Err(OrphanError::InvalidPlan)
        );

        let intent = OrphanIntent::prepare(&base, &candidate, &plan).unwrap();
        let mut bytes = intent.encode();
        bytes[8] = 9;
        assert_eq!(OrphanIntent::decode(&bytes), Err(OrphanError::Malformed));
    }
}
