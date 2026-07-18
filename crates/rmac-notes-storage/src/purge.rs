use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_notes_store::{
    encode, AttachmentId, LibrarySnapshot, NoteId, PurgePlan, MAX_ATTACHMENTS,
    MAX_ATTACHMENT_BYTES, MAX_NOTES,
};
use rmac_storage::{Backend, FileFingerprint};
use sha2::{Digest as _, Sha256};

const PURGE_MAGIC: &[u8; 8] = b"RMNPURG\0";
const PURGE_VERSION: u16 = 1;
pub(crate) const MAX_PURGE_INTENT_BYTES: usize =
    8 + 2 + 16 + 64 + 8 + (MAX_NOTES * 8) + 8 + (MAX_ATTACHMENTS * 48) + 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PurgeError {
    InvalidPlan,
    TooLarge,
    Malformed,
    Io(io::ErrorKind),
    Remove(io::ErrorKind),
    ReadbackMismatch,
    AttachmentMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PurgeAuthority {
    RolledBack,
    Accepted,
    AcceptedDescendant,
    Ambiguous,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CleanupAttachment {
    id: AttachmentId,
    byte_len: u64,
    sha256: [u8; 32],
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PurgeIntent {
    base_library_revision: u64,
    candidate_library_revision: u64,
    base_sha256: [u8; 32],
    candidate_sha256: [u8; 32],
    note_ids: Vec<NoteId>,
    attachments: Vec<CleanupAttachment>,
    attachment_bytes: u64,
}

impl PurgeIntent {
    pub(crate) fn prepare(
        base: &LibrarySnapshot,
        candidate: &LibrarySnapshot,
        plan: &PurgePlan,
    ) -> Result<Self, PurgeError> {
        base.validate().map_err(|_| PurgeError::InvalidPlan)?;
        candidate.validate().map_err(|_| PurgeError::InvalidPlan)?;
        if plan.base_library_revision != base.revision
            || plan.candidate_library_revision != candidate.revision
            || candidate.revision
                != base
                    .revision
                    .checked_add(1)
                    .ok_or(PurgeError::InvalidPlan)?
            || plan.note_ids.is_empty()
            || plan.note_ids.len() > MAX_NOTES
            || plan.attachment_ids.len() > MAX_ATTACHMENTS
            || !strictly_sorted(&plan.note_ids)
            || !strictly_sorted(&plan.attachment_ids)
        {
            return Err(PurgeError::InvalidPlan);
        }

        let note_ids = plan.note_ids.iter().copied().collect::<BTreeSet<_>>();
        if note_ids.len() != plan.note_ids.len()
            || note_ids.iter().any(|note_id| {
                !base
                    .notes
                    .iter()
                    .any(|note| note.id == *note_id && note.deleted)
            })
            || candidate
                .notes
                .iter()
                .any(|note| note_ids.contains(&note.id))
        {
            return Err(PurgeError::InvalidPlan);
        }

        let mut attachments = base
            .attachments
            .iter()
            .filter(|attachment| note_ids.contains(&attachment.note_id))
            .map(|attachment| CleanupAttachment {
                id: attachment.id,
                byte_len: attachment.byte_len,
                sha256: attachment.sha256,
            })
            .collect::<Vec<_>>();
        attachments.sort_by_key(|attachment| attachment.id);
        if attachments
            .iter()
            .map(|attachment| attachment.id)
            .ne(plan.attachment_ids.iter().copied())
            || candidate
                .attachments
                .iter()
                .any(|attachment| plan.attachment_ids.binary_search(&attachment.id).is_ok())
        {
            return Err(PurgeError::InvalidPlan);
        }
        let attachment_bytes = attachments.iter().try_fold(0_u64, |total, attachment| {
            total
                .checked_add(attachment.byte_len)
                .ok_or(PurgeError::InvalidPlan)
        })?;
        if attachment_bytes != plan.attachment_bytes {
            return Err(PurgeError::InvalidPlan);
        }

        let base_bytes = encode(base).map_err(|_| PurgeError::InvalidPlan)?;
        let candidate_bytes = encode(candidate).map_err(|_| PurgeError::InvalidPlan)?;
        let mut exact_candidate = base.clone();
        exact_candidate.revision = candidate.revision;
        exact_candidate
            .notes
            .retain(|note| !note_ids.contains(&note.id));
        exact_candidate
            .attachments
            .retain(|attachment| plan.attachment_ids.binary_search(&attachment.id).is_err());
        if encode(&exact_candidate).map_err(|_| PurgeError::InvalidPlan)? != candidate_bytes {
            return Err(PurgeError::InvalidPlan);
        }
        Ok(Self {
            base_library_revision: base.revision,
            candidate_library_revision: candidate.revision,
            base_sha256: digest(&base_bytes),
            candidate_sha256: digest(&candidate_bytes),
            note_ids: plan.note_ids.clone(),
            attachments,
            attachment_bytes,
        })
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, PurgeError> {
        if self.note_ids.is_empty()
            || self.note_ids.len() > MAX_NOTES
            || self.attachments.len() > MAX_ATTACHMENTS
            || !strictly_sorted(&self.note_ids)
            || !strictly_sorted_by_id(&self.attachments)
        {
            return Err(PurgeError::InvalidPlan);
        }
        let mut bytes = Vec::new();
        extend(&mut bytes, PURGE_MAGIC)?;
        extend(&mut bytes, &PURGE_VERSION.to_le_bytes())?;
        extend(&mut bytes, &self.base_library_revision.to_le_bytes())?;
        extend(&mut bytes, &self.candidate_library_revision.to_le_bytes())?;
        extend(&mut bytes, &self.base_sha256)?;
        extend(&mut bytes, &self.candidate_sha256)?;
        extend(&mut bytes, &(self.note_ids.len() as u64).to_le_bytes())?;
        for note_id in &self.note_ids {
            extend(&mut bytes, &note_id.get().to_le_bytes())?;
        }
        extend(&mut bytes, &(self.attachments.len() as u64).to_le_bytes())?;
        for attachment in &self.attachments {
            extend(&mut bytes, &attachment.id.get().to_le_bytes())?;
            extend(&mut bytes, &attachment.byte_len.to_le_bytes())?;
            extend(&mut bytes, &attachment.sha256)?;
        }
        extend(&mut bytes, &self.attachment_bytes.to_le_bytes())?;
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, PurgeError> {
        if bytes.len() > MAX_PURGE_INTENT_BYTES {
            return Err(PurgeError::TooLarge);
        }
        let mut reader = Reader::new(bytes);
        if reader.take(8)? != PURGE_MAGIC || reader.u16()? != PURGE_VERSION {
            return Err(PurgeError::Malformed);
        }
        let base_library_revision = reader.u64()?;
        let candidate_library_revision = reader.u64()?;
        let base_sha256 = reader.array()?;
        let candidate_sha256 = reader.array()?;
        let note_count = reader.count(MAX_NOTES)?;
        let mut note_ids = Vec::with_capacity(note_count);
        for _ in 0..note_count {
            note_ids.push(NoteId::new(reader.u64()?).ok_or(PurgeError::Malformed)?);
        }
        let attachment_count = reader.count(MAX_ATTACHMENTS)?;
        let mut attachments = Vec::with_capacity(attachment_count);
        for _ in 0..attachment_count {
            let id = AttachmentId::new(reader.u64()?).ok_or(PurgeError::Malformed)?;
            let byte_len = reader.u64()?;
            let sha256 = reader.array()?;
            if byte_len > MAX_ATTACHMENT_BYTES || sha256 == [0; 32] {
                return Err(PurgeError::Malformed);
            }
            attachments.push(CleanupAttachment {
                id,
                byte_len,
                sha256,
            });
        }
        let attachment_bytes = reader.u64()?;
        if !reader.is_empty()
            || base_library_revision == 0
            || candidate_library_revision != base_library_revision.checked_add(1).unwrap_or(0)
            || note_ids.is_empty()
            || !strictly_sorted(&note_ids)
            || !strictly_sorted_by_id(&attachments)
            || attachments.iter().try_fold(0_u64, |total, attachment| {
                total.checked_add(attachment.byte_len)
            }) != Some(attachment_bytes)
        {
            return Err(PurgeError::Malformed);
        }
        Ok(Self {
            base_library_revision,
            candidate_library_revision,
            base_sha256,
            candidate_sha256,
            note_ids,
            attachments,
            attachment_bytes,
        })
    }

    pub(crate) fn authority(
        &self,
        snapshot: &LibrarySnapshot,
    ) -> Result<PurgeAuthority, PurgeError> {
        let encoded = encode(snapshot).map_err(|_| PurgeError::Malformed)?;
        let hash = digest(&encoded);
        if snapshot.revision == self.base_library_revision && hash == self.base_sha256 {
            return Ok(PurgeAuthority::RolledBack);
        }
        if snapshot.revision == self.candidate_library_revision && hash == self.candidate_sha256 {
            return Ok(PurgeAuthority::Accepted);
        }
        if snapshot.revision > self.candidate_library_revision
            && self
                .note_ids
                .iter()
                .all(|note_id| snapshot.notes.iter().all(|note| note.id != *note_id))
            && self.attachments.iter().all(|planned| {
                snapshot
                    .attachments
                    .iter()
                    .all(|attachment| attachment.id != planned.id)
            })
        {
            return Ok(PurgeAuthority::AcceptedDescendant);
        }
        Ok(PurgeAuthority::Ambiguous)
    }

    pub(crate) fn cleanup<B: Backend>(&self, root: &Path, backend: &B) -> Result<(), PurgeError> {
        for attachment in &self.attachments {
            let path = managed_attachment_path(root, attachment.id);
            let fingerprint =
                match backend.fingerprint_bounded_no_follow(&path, attachment.byte_len) {
                    Ok(fingerprint) => fingerprint,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                        return Err(PurgeError::AttachmentMismatch)
                    }
                    Err(error) => return Err(PurgeError::Io(error.kind())),
                };
            if fingerprint
                != (FileFingerprint {
                    byte_len: attachment.byte_len,
                    sha256: attachment.sha256,
                })
            {
                return Err(PurgeError::AttachmentMismatch);
            }
            backend
                .remove_file_durable(&path)
                .map_err(|error| PurgeError::Remove(error.kind()))?;
            match backend.fingerprint_bounded_no_follow(&path, attachment.byte_len) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => return Err(PurgeError::ReadbackMismatch),
            }
        }
        Ok(())
    }
}

impl fmt::Debug for PurgeIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PurgeIntent")
            .field("base_library_revision", &self.base_library_revision)
            .field(
                "candidate_library_revision",
                &self.candidate_library_revision,
            )
            .field("note_count", &self.note_ids.len())
            .field("attachment_count", &self.attachments.len())
            .field("attachment_bytes", &self.attachment_bytes)
            .finish()
    }
}

fn managed_attachment_path(root: &Path, id: AttachmentId) -> PathBuf {
    root.join("attachments")
        .join(format!("{:020}.bin", id.get()))
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn strictly_sorted_by_id(values: &[CleanupAttachment]) -> bool {
    values.windows(2).all(|pair| pair[0].id < pair[1].id)
}

fn extend(output: &mut Vec<u8>, value: &[u8]) -> Result<(), PurgeError> {
    let next = output
        .len()
        .checked_add(value.len())
        .ok_or(PurgeError::TooLarge)?;
    if next > MAX_PURGE_INTENT_BYTES {
        return Err(PurgeError::TooLarge);
    }
    output.extend_from_slice(value);
    Ok(())
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

    fn take(&mut self, count: usize) -> Result<&'a [u8], PurgeError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or(PurgeError::Malformed)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(PurgeError::Malformed)?;
        self.cursor = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, PurgeError> {
        self.take(2)?
            .try_into()
            .map(u16::from_le_bytes)
            .map_err(|_| PurgeError::Malformed)
    }

    fn u64(&mut self) -> Result<u64, PurgeError> {
        self.take(8)?
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| PurgeError::Malformed)
    }

    fn array(&mut self) -> Result<[u8; 32], PurgeError> {
        self.take(32)?.try_into().map_err(|_| PurgeError::Malformed)
    }

    fn count(&mut self, maximum: usize) -> Result<usize, PurgeError> {
        let count = usize::try_from(self.u64()?).map_err(|_| PurgeError::Malformed)?;
        (count <= maximum)
            .then_some(count)
            .ok_or(PurgeError::Malformed)
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use rmac_notes_store::{
        AttachmentKind, AttachmentRecord, LibraryTransaction, NoteRecord, SortOrder,
    };

    fn fixture() -> (
        LibrarySnapshot,
        LibrarySnapshot,
        PurgePlan,
        Vec<(PathBuf, Vec<u8>)>,
    ) {
        let note_id = NoteId::new(1).unwrap();
        let first_id = AttachmentId::new(1).unwrap();
        let second_id = AttachmentId::new(2).unwrap();
        let first = b"first managed image".to_vec();
        let second = b"second managed image".to_vec();
        let base = LibrarySnapshot {
            revision: 4,
            sort_order: SortOrder::Edited,
            next_note_id: 2,
            next_folder_id: 1,
            next_attachment_id: 3,
            folders: Vec::new(),
            notes: vec![NoteRecord {
                id: note_id,
                revision: 2,
                created_unix_ms: 1,
                modified_unix_ms: 2,
                title: "Private title".into(),
                body: "Private body".into(),
                tags: vec!["private-tag".into()],
                folder_id: None,
                pinned: false,
                deleted: true,
                attachments: vec![first_id],
            }],
            attachments: vec![
                AttachmentRecord {
                    id: first_id,
                    revision: 1,
                    note_id,
                    display_name: "private-first.png".into(),
                    kind: AttachmentKind::Png,
                    byte_len: first.len() as u64,
                    sha256: digest(&first),
                    deleted: false,
                },
                AttachmentRecord {
                    id: second_id,
                    revision: 2,
                    note_id,
                    display_name: "private-orphan.png".into(),
                    kind: AttachmentKind::Png,
                    byte_len: second.len() as u64,
                    sha256: digest(&second),
                    deleted: true,
                },
            ],
        };
        base.validate().unwrap();
        let mut transaction = LibraryTransaction::begin(&base).unwrap();
        let plan = transaction.purge_trashed_note(note_id, 2).unwrap();
        let candidate = transaction.finish().unwrap();
        let root = Path::new("/virtual/library");
        (
            base,
            candidate,
            plan,
            vec![
                (managed_attachment_path(root, first_id), first),
                (managed_attachment_path(root, second_id), second),
            ],
        )
    }

    #[derive(Clone, Default)]
    struct FakeBackend(Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>);

    impl FakeBackend {
        fn insert(&self, path: PathBuf, bytes: Vec<u8>) {
            self.0.lock().unwrap().insert(path, bytes);
        }

        fn contains(&self, path: &Path) -> bool {
            self.0.lock().unwrap().contains_key(path)
        }
    }

    impl Backend for FakeBackend {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.0
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.0
                .lock()
                .unwrap()
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }

    #[test]
    fn versioned_intent_round_trips_without_private_content() {
        let (base, candidate, plan, _) = fixture();
        let intent = PurgeIntent::prepare(&base, &candidate, &plan).unwrap();
        let encoded = intent.encode().unwrap();
        let debug = format!("{intent:?}");

        assert_eq!(&encoded[..8], PURGE_MAGIC);
        assert_eq!(PurgeIntent::decode(&encoded).unwrap(), intent);
        assert!(!debug.contains("Private title"));
        assert!(!debug.contains("private-first.png"));
        assert!(debug.contains("attachment_count"));

        let mut trailing = encoded;
        trailing.push(0);
        assert_eq!(PurgeIntent::decode(&trailing), Err(PurgeError::Malformed));
    }

    #[test]
    fn preparation_rejects_tampered_scope_revision_and_total() {
        let (base, candidate, plan, _) = fixture();

        let mut wrong_revision = plan.clone();
        wrong_revision.base_library_revision -= 1;
        assert_eq!(
            PurgeIntent::prepare(&base, &candidate, &wrong_revision),
            Err(PurgeError::InvalidPlan)
        );

        let mut missing_attachment = plan.clone();
        missing_attachment.attachment_ids.pop();
        assert_eq!(
            PurgeIntent::prepare(&base, &candidate, &missing_attachment),
            Err(PurgeError::InvalidPlan)
        );

        let mut wrong_total = plan.clone();
        wrong_total.attachment_bytes += 1;
        assert_eq!(
            PurgeIntent::prepare(&base, &candidate, &wrong_total),
            Err(PurgeError::InvalidPlan)
        );

        let mut unrelated_change = candidate;
        unrelated_change.sort_order = SortOrder::Title;
        assert_eq!(
            PurgeIntent::prepare(&base, &unrelated_change, &plan),
            Err(PurgeError::InvalidPlan)
        );
    }

    #[test]
    fn authority_requires_exact_base_candidate_or_proven_descendant_absence() {
        let (base, candidate, plan, _) = fixture();
        let intent = PurgeIntent::prepare(&base, &candidate, &plan).unwrap();

        assert_eq!(intent.authority(&base).unwrap(), PurgeAuthority::RolledBack);
        assert_eq!(
            intent.authority(&candidate).unwrap(),
            PurgeAuthority::Accepted
        );
        let mut descendant = candidate.clone();
        descendant.revision += 1;
        assert_eq!(
            intent.authority(&descendant).unwrap(),
            PurgeAuthority::AcceptedDescendant
        );
        let mut ambiguous = base.clone();
        ambiguous.revision += 2;
        assert_eq!(
            intent.authority(&ambiguous).unwrap(),
            PurgeAuthority::Ambiguous
        );
    }

    #[test]
    fn cleanup_deletes_only_exact_files_and_is_restart_idempotent() {
        let (base, candidate, plan, files) = fixture();
        let intent = PurgeIntent::prepare(&base, &candidate, &plan).unwrap();
        let backend = FakeBackend::default();
        for (path, bytes) in &files {
            backend.insert(path.clone(), bytes.clone());
        }

        intent
            .cleanup(Path::new("/virtual/library"), &backend)
            .unwrap();
        assert!(files.iter().all(|(path, _)| !backend.contains(path)));
        intent
            .cleanup(Path::new("/virtual/library"), &backend)
            .unwrap();
    }

    #[test]
    fn cleanup_refuses_changed_attachment_bytes_without_deleting_them() {
        let (base, candidate, plan, files) = fixture();
        let intent = PurgeIntent::prepare(&base, &candidate, &plan).unwrap();
        let backend = FakeBackend::default();
        backend.insert(files[0].0.clone(), b"substituted bytes".to_vec());

        assert_eq!(
            intent.cleanup(Path::new("/virtual/library"), &backend),
            Err(PurgeError::AttachmentMismatch)
        );
        assert!(backend.contains(&files[0].0));
    }
}
