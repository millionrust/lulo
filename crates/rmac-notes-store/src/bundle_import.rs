use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::{
    AttachmentId, AttachmentRecord, FolderId, FolderRecord, LibrarySnapshot, NoteId, NoteRecord,
    ValidationError, MAX_ATTACHMENTS, MAX_FOLDERS, MAX_NAME_BYTES, MAX_NOTES,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleCollisionPolicy {
    /// Preserve every imported record without replacing destination data.
    /// Identities that could reuse destination history are remapped, and live
    /// folder-name collisions receive a deterministic imported suffix.
    KeepBoth,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BundleImportReview {
    pub base_library_revision: u64,
    pub source_library_revision: u64,
    pub folder_count: usize,
    pub note_count: usize,
    pub attachment_count: usize,
    pub identity_collisions: usize,
    pub folder_name_collisions: usize,
    pub attachment_bytes: u64,
    pub source_bytes: u64,
    source_sha256: [u8; 32],
}

impl fmt::Debug for BundleImportReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BundleImportReview")
            .field("base_library_revision", &self.base_library_revision)
            .field("source_library_revision", &self.source_library_revision)
            .field("folder_count", &self.folder_count)
            .field("note_count", &self.note_count)
            .field("attachment_count", &self.attachment_count)
            .field("identity_collisions", &self.identity_collisions)
            .field("folder_name_collisions", &self.folder_name_collisions)
            .field("attachment_bytes", &self.attachment_bytes)
            .field("source_bytes", &self.source_bytes)
            .field("source_sha256", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BundleAttachmentImport {
    pub source_id: AttachmentId,
    pub destination_id: AttachmentId,
    pub destination_note_id: NoteId,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

impl fmt::Debug for BundleAttachmentImport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BundleAttachmentImport")
            .field("source_id", &self.source_id)
            .field("destination_id", &self.destination_id)
            .field("destination_note_id", &self.destination_note_id)
            .field("byte_len", &self.byte_len)
            .field("sha256", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BundleImportPlan {
    pub base_library_revision: u64,
    pub candidate_library_revision: u64,
    pub source_library_revision: u64,
    pub policy: BundleCollisionPolicy,
    pub folder_mappings: Vec<(FolderId, FolderId)>,
    pub note_mappings: Vec<(NoteId, NoteId)>,
    pub attachments: Vec<BundleAttachmentImport>,
    pub source_bytes: u64,
    pub attachment_bytes: u64,
    source_sha256: [u8; 32],
}

impl fmt::Debug for BundleImportPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BundleImportPlan")
            .field("base_library_revision", &self.base_library_revision)
            .field(
                "candidate_library_revision",
                &self.candidate_library_revision,
            )
            .field("source_library_revision", &self.source_library_revision)
            .field("policy", &self.policy)
            .field("folder_count", &self.folder_mappings.len())
            .field("note_count", &self.note_mappings.len())
            .field("attachment_count", &self.attachments.len())
            .field("source_bytes", &self.source_bytes)
            .field("attachment_bytes", &self.attachment_bytes)
            .field("source_sha256", &"<redacted>")
            .finish()
    }
}

impl BundleImportPlan {
    pub fn source_sha256(&self) -> [u8; 32] {
        self.source_sha256
    }

    /// Re-derive the complete import and prove that neither the reviewed base,
    /// source bundle, collision policy, mappings, nor candidate changed.
    pub fn validate_candidate(
        &self,
        base: &LibrarySnapshot,
        source: &LibrarySnapshot,
        candidate: &LibrarySnapshot,
    ) -> Result<(), BundlePlanError> {
        let planned = build_import(
            base,
            source,
            self.policy,
            self.source_bytes,
            self.source_sha256,
        )?;
        if planned.plan != *self || planned.candidate != *candidate {
            return Err(BundlePlanError::InvalidPlan);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PlannedBundleImport {
    candidate: LibrarySnapshot,
    plan: BundleImportPlan,
}

impl fmt::Debug for PlannedBundleImport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlannedBundleImport")
            .field("candidate_revision", &self.candidate.revision)
            .field("plan", &self.plan)
            .finish()
    }
}

impl PlannedBundleImport {
    pub fn candidate(&self) -> &LibrarySnapshot {
        &self.candidate
    }

    pub fn plan(&self) -> &BundleImportPlan {
        &self.plan
    }

    pub fn into_parts(self) -> (LibrarySnapshot, BundleImportPlan) {
        (self.candidate, self.plan)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundlePlanError {
    InvalidBase(ValidationError),
    InvalidSource(ValidationError),
    InvalidCandidate(ValidationError),
    InvalidSourceAttachments,
    InvalidSourceFingerprint,
    RevisionExhausted,
    IdentityExhausted,
    CollectionLimit,
    ByteCountOverflow,
    InvalidPlan,
}

impl fmt::Display for BundlePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBase(error)
            | Self::InvalidSource(error)
            | Self::InvalidCandidate(error) => return error.fmt(formatter),
            Self::InvalidSourceAttachments => {
                "The selected Notes bundle contains undeclared attachment state"
            }
            Self::InvalidSourceFingerprint => {
                "The selected Notes bundle has an invalid source identity"
            }
            Self::RevisionExhausted => "The Notes library revision sequence is exhausted",
            Self::IdentityExhausted => "The Notes identity sequence is exhausted",
            Self::CollectionLimit => "The Notes bundle exceeds a library collection limit",
            Self::ByteCountOverflow => "The Notes bundle byte total exceeds a checked limit",
            Self::InvalidPlan => "The reviewed Notes bundle import plan changed",
        })
    }
}

impl std::error::Error for BundlePlanError {}

impl LibrarySnapshot {
    pub fn review_bundle_import(
        &self,
        source: &LibrarySnapshot,
        source_bytes: u64,
        source_sha256: [u8; 32],
    ) -> Result<BundleImportReview, BundlePlanError> {
        self.validate().map_err(BundlePlanError::InvalidBase)?;
        source.validate().map_err(BundlePlanError::InvalidSource)?;
        validate_source(source, source_bytes, source_sha256)?;
        checked_combined_counts(self, source)?;

        let identity_collisions = source
            .folders
            .iter()
            .filter(|folder| folder.id.get() < self.next_folder_id)
            .count()
            .checked_add(
                source
                    .notes
                    .iter()
                    .filter(|note| note.id.get() < self.next_note_id)
                    .count(),
            )
            .and_then(|count| {
                count.checked_add(
                    source
                        .attachments
                        .iter()
                        .filter(|attachment| attachment.id.get() < self.next_attachment_id)
                        .count(),
                )
            })
            .ok_or(BundlePlanError::CollectionLimit)?;
        let existing_names = self
            .folders
            .iter()
            .filter(|folder| !folder.deleted)
            .map(|folder| folder.name.to_lowercase())
            .collect::<BTreeSet<_>>();
        let folder_name_collisions = source
            .folders
            .iter()
            .filter(|folder| {
                !folder.deleted && existing_names.contains(&folder.name.to_lowercase())
            })
            .count();
        let attachment_bytes = attachment_bytes(source)?;

        Ok(BundleImportReview {
            base_library_revision: self.revision,
            source_library_revision: source.revision,
            folder_count: source.folders.len(),
            note_count: source.notes.len(),
            attachment_count: source.attachments.len(),
            identity_collisions,
            folder_name_collisions,
            attachment_bytes,
            source_bytes,
            source_sha256,
        })
    }

    pub fn plan_bundle_import(
        &self,
        source: &LibrarySnapshot,
        policy: BundleCollisionPolicy,
        source_bytes: u64,
        source_sha256: [u8; 32],
    ) -> Result<PlannedBundleImport, BundlePlanError> {
        build_import(self, source, policy, source_bytes, source_sha256)
    }
}

fn build_import(
    base: &LibrarySnapshot,
    source: &LibrarySnapshot,
    policy: BundleCollisionPolicy,
    source_bytes: u64,
    source_sha256: [u8; 32],
) -> Result<PlannedBundleImport, BundlePlanError> {
    base.validate().map_err(BundlePlanError::InvalidBase)?;
    source.validate().map_err(BundlePlanError::InvalidSource)?;
    validate_source(source, source_bytes, source_sha256)?;
    checked_combined_counts(base, source)?;

    let candidate_revision = base
        .revision
        .checked_add(1)
        .ok_or(BundlePlanError::RevisionExhausted)?;
    let (folder_map, next_folder_id) = map_identities(
        source.folders.iter().map(|folder| folder.id.get()),
        base.next_folder_id,
        source.next_folder_id,
    )?;
    let (note_map, next_note_id) = map_identities(
        source.notes.iter().map(|note| note.id.get()),
        base.next_note_id,
        source.next_note_id,
    )?;
    let (attachment_map, next_attachment_id) = map_identities(
        source
            .attachments
            .iter()
            .map(|attachment| attachment.id.get()),
        base.next_attachment_id,
        source.next_attachment_id,
    )?;

    let mut candidate = base.clone();
    candidate.revision = candidate_revision;
    candidate.next_folder_id = next_folder_id;
    candidate.next_note_id = next_note_id;
    candidate.next_attachment_id = next_attachment_id;

    let mut used_folder_names = candidate
        .folders
        .iter()
        .filter(|folder| !folder.deleted)
        .map(|folder| folder.name.to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut source_folders = source.folders.iter().collect::<Vec<_>>();
    source_folders.sort_by_key(|folder| folder.id);
    for folder in source_folders {
        let destination_id = folder_id(
            *folder_map
                .get(&folder.id.get())
                .ok_or(BundlePlanError::InvalidPlan)?,
        )?;
        let name = if folder.deleted {
            folder.name.clone()
        } else {
            unique_imported_name(&folder.name, &mut used_folder_names)?
        };
        candidate.folders.push(FolderRecord {
            id: destination_id,
            revision: folder.revision,
            name,
            deleted: folder.deleted,
        });
    }

    let mut source_notes = source.notes.iter().collect::<Vec<_>>();
    source_notes.sort_by_key(|note| note.id);
    for note in source_notes {
        candidate
            .notes
            .push(remap_note(note, &folder_map, &note_map, &attachment_map)?);
    }

    let mut attachment_plans = Vec::with_capacity(source.attachments.len());
    let mut source_attachments = source.attachments.iter().collect::<Vec<_>>();
    source_attachments.sort_by_key(|attachment| attachment.id);
    for attachment in source_attachments {
        let remapped = remap_attachment(attachment, &note_map, &attachment_map)?;
        attachment_plans.push(BundleAttachmentImport {
            source_id: attachment.id,
            destination_id: remapped.id,
            destination_note_id: remapped.note_id,
            byte_len: remapped.byte_len,
            sha256: remapped.sha256,
        });
        candidate.attachments.push(remapped);
    }

    candidate
        .validate()
        .map_err(BundlePlanError::InvalidCandidate)?;
    let attachment_bytes = attachment_bytes(source)?;
    let plan = BundleImportPlan {
        base_library_revision: base.revision,
        candidate_library_revision: candidate_revision,
        source_library_revision: source.revision,
        policy,
        folder_mappings: typed_folder_mappings(&folder_map)?,
        note_mappings: typed_note_mappings(&note_map)?,
        attachments: attachment_plans,
        source_bytes,
        attachment_bytes,
        source_sha256,
    };
    Ok(PlannedBundleImport { candidate, plan })
}

fn validate_source(
    source: &LibrarySnapshot,
    source_bytes: u64,
    source_sha256: [u8; 32],
) -> Result<(), BundlePlanError> {
    if source_bytes == 0 || source_sha256 == [0; 32] {
        return Err(BundlePlanError::InvalidSourceFingerprint);
    }
    if source
        .attachments
        .iter()
        .any(|attachment| attachment.deleted)
    {
        return Err(BundlePlanError::InvalidSourceAttachments);
    }
    Ok(())
}

fn checked_combined_counts(
    base: &LibrarySnapshot,
    source: &LibrarySnapshot,
) -> Result<(), BundlePlanError> {
    if base
        .folders
        .len()
        .checked_add(source.folders.len())
        .is_none_or(|count| count > MAX_FOLDERS)
        || base
            .notes
            .len()
            .checked_add(source.notes.len())
            .is_none_or(|count| count > MAX_NOTES)
        || base
            .attachments
            .len()
            .checked_add(source.attachments.len())
            .is_none_or(|count| count > MAX_ATTACHMENTS)
    {
        return Err(BundlePlanError::CollectionLimit);
    }
    Ok(())
}

fn attachment_bytes(source: &LibrarySnapshot) -> Result<u64, BundlePlanError> {
    source
        .attachments
        .iter()
        .try_fold(0_u64, |total, attachment| {
            total
                .checked_add(attachment.byte_len)
                .ok_or(BundlePlanError::ByteCountOverflow)
        })
}

fn map_identities(
    source_ids: impl Iterator<Item = u64>,
    destination_next: u64,
    source_next: u64,
) -> Result<(BTreeMap<u64, u64>, u64), BundlePlanError> {
    let mut source_ids = source_ids.collect::<Vec<_>>();
    source_ids.sort_unstable();
    let mut used = source_ids
        .iter()
        .copied()
        .filter(|identity| *identity >= destination_next)
        .collect::<BTreeSet<_>>();
    let mut next = destination_next;
    let mut mappings = BTreeMap::new();
    for source_id in source_ids {
        let destination_id = if source_id >= destination_next {
            source_id
        } else {
            while used.contains(&next) {
                next = next
                    .checked_add(1)
                    .ok_or(BundlePlanError::IdentityExhausted)?;
            }
            let allocated = next;
            used.insert(allocated);
            next = next
                .checked_add(1)
                .ok_or(BundlePlanError::IdentityExhausted)?;
            allocated
        };
        mappings.insert(source_id, destination_id);
    }
    let maximum = used.iter().next_back().copied().unwrap_or_default();
    let next = next.max(source_next).max(
        maximum
            .checked_add(1)
            .ok_or(BundlePlanError::IdentityExhausted)?,
    );
    if next == 0 {
        return Err(BundlePlanError::IdentityExhausted);
    }
    Ok((mappings, next))
}

fn remap_note(
    note: &NoteRecord,
    folder_map: &BTreeMap<u64, u64>,
    note_map: &BTreeMap<u64, u64>,
    attachment_map: &BTreeMap<u64, u64>,
) -> Result<NoteRecord, BundlePlanError> {
    Ok(NoteRecord {
        id: note_id(
            *note_map
                .get(&note.id.get())
                .ok_or(BundlePlanError::InvalidPlan)?,
        )?,
        revision: note.revision,
        created_unix_ms: note.created_unix_ms,
        modified_unix_ms: note.modified_unix_ms,
        title: note.title.clone(),
        body: note.body.clone(),
        tags: note.tags.clone(),
        folder_id: note
            .folder_id
            .map(|id| {
                folder_map
                    .get(&id.get())
                    .copied()
                    .ok_or(BundlePlanError::InvalidPlan)
                    .and_then(folder_id)
            })
            .transpose()?,
        pinned: note.pinned,
        deleted: note.deleted,
        attachments: note
            .attachments
            .iter()
            .map(|id| {
                attachment_map
                    .get(&id.get())
                    .copied()
                    .ok_or(BundlePlanError::InvalidPlan)
                    .and_then(attachment_id)
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn remap_attachment(
    attachment: &AttachmentRecord,
    note_map: &BTreeMap<u64, u64>,
    attachment_map: &BTreeMap<u64, u64>,
) -> Result<AttachmentRecord, BundlePlanError> {
    Ok(AttachmentRecord {
        id: attachment_id(
            *attachment_map
                .get(&attachment.id.get())
                .ok_or(BundlePlanError::InvalidPlan)?,
        )?,
        revision: attachment.revision,
        note_id: note_id(
            *note_map
                .get(&attachment.note_id.get())
                .ok_or(BundlePlanError::InvalidPlan)?,
        )?,
        display_name: attachment.display_name.clone(),
        kind: attachment.kind,
        byte_len: attachment.byte_len,
        sha256: attachment.sha256,
        deleted: false,
    })
}

fn unique_imported_name(
    original: &str,
    used: &mut BTreeSet<String>,
) -> Result<String, BundlePlanError> {
    if used.insert(original.to_lowercase()) {
        return Ok(original.to_string());
    }
    for attempt in 1_u64..=MAX_FOLDERS as u64 + 1 {
        let suffix = if attempt == 1 {
            " (Imported)".to_string()
        } else {
            format!(" (Imported {attempt})")
        };
        let prefix = truncate_utf8(original, MAX_NAME_BYTES.saturating_sub(suffix.len()));
        let candidate = format!("{prefix}{suffix}");
        if used.insert(candidate.to_lowercase()) {
            return Ok(candidate);
        }
    }
    Err(BundlePlanError::CollectionLimit)
}

fn truncate_utf8(value: &str, maximum: usize) -> &str {
    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .take_while(|index| *index <= maximum)
        .last()
        .unwrap_or_default();
    &value[..end]
}

fn typed_folder_mappings(
    mappings: &BTreeMap<u64, u64>,
) -> Result<Vec<(FolderId, FolderId)>, BundlePlanError> {
    mappings
        .iter()
        .map(|(source, destination)| Ok((folder_id(*source)?, folder_id(*destination)?)))
        .collect()
}

fn typed_note_mappings(
    mappings: &BTreeMap<u64, u64>,
) -> Result<Vec<(NoteId, NoteId)>, BundlePlanError> {
    mappings
        .iter()
        .map(|(source, destination)| Ok((note_id(*source)?, note_id(*destination)?)))
        .collect()
}

fn folder_id(value: u64) -> Result<FolderId, BundlePlanError> {
    FolderId::new(value).ok_or(BundlePlanError::IdentityExhausted)
}

fn note_id(value: u64) -> Result<NoteId, BundlePlanError> {
    NoteId::new(value).ok_or(BundlePlanError::IdentityExhausted)
}

fn attachment_id(value: u64) -> Result<AttachmentId, BundlePlanError> {
    AttachmentId::new(value).ok_or(BundlePlanError::IdentityExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttachmentKind, SortOrder};

    fn source() -> LibrarySnapshot {
        let folder_id = FolderId::new(1).unwrap();
        let note_id = NoteId::new(1).unwrap();
        let attachment_id = AttachmentId::new(1).unwrap();
        LibrarySnapshot {
            revision: 8,
            sort_order: SortOrder::Title,
            next_note_id: 2,
            next_folder_id: 2,
            next_attachment_id: 2,
            folders: vec![FolderRecord {
                id: folder_id,
                revision: 3,
                name: "Projects".into(),
                deleted: false,
            }],
            notes: vec![NoteRecord {
                id: note_id,
                revision: 4,
                created_unix_ms: 10,
                modified_unix_ms: 20,
                title: "Imported private title".into(),
                body: "Imported private body".into(),
                tags: vec!["private-tag".into()],
                folder_id: Some(folder_id),
                pinned: true,
                deleted: false,
                attachments: vec![attachment_id],
            }],
            attachments: vec![AttachmentRecord {
                id: attachment_id,
                revision: 2,
                note_id,
                display_name: "private.png".into(),
                kind: AttachmentKind::Png,
                byte_len: 12,
                sha256: [7; 32],
                deleted: false,
            }],
        }
    }

    #[test]
    fn keep_both_remaps_destination_history_and_renames_live_folder_collisions() {
        let mut base = LibrarySnapshot::default();
        let mut transaction = crate::LibraryTransaction::begin(&base).unwrap();
        transaction.create_folder("Projects".into()).unwrap();
        transaction
            .create_note(crate::NewNote {
                created_unix_ms: 1,
                title: "Existing".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id: None,
            })
            .unwrap();
        base = transaction.finish().unwrap();
        let source = source();
        let review = base.review_bundle_import(&source, 500, [5; 32]).unwrap();

        assert_eq!(review.identity_collisions, 2);
        assert_eq!(review.folder_name_collisions, 1);
        let planned = base
            .plan_bundle_import(&source, BundleCollisionPolicy::KeepBoth, 500, [5; 32])
            .unwrap();
        let imported_folder = planned
            .candidate()
            .folders
            .iter()
            .find(|folder| folder.id != FolderId::new(1).unwrap())
            .unwrap();
        let imported_note = planned
            .candidate()
            .notes
            .iter()
            .find(|note| note.title == "Imported private title")
            .unwrap();
        assert_eq!(imported_folder.name, "Projects (Imported)");
        assert_eq!(imported_note.folder_id, Some(imported_folder.id));
        assert_ne!(imported_note.id, NoteId::new(1).unwrap());
        assert_eq!(
            planned.plan().attachments[0].destination_note_id,
            imported_note.id
        );
        planned
            .plan()
            .validate_candidate(&base, &source, planned.candidate())
            .unwrap();
        assert!(!format!("{planned:?}").contains("Imported private"));
        assert!(!format!("{:?}", planned.plan()).contains("[5, 5"));
    }

    #[test]
    fn free_future_source_id_is_preserved_without_reusing_a_purged_gap() {
        let base = LibrarySnapshot {
            next_note_id: 5,
            ..LibrarySnapshot::default()
        };
        base.validate().unwrap();
        let mut source = source();
        source.notes[0].id = NoteId::new(7).unwrap();
        source.attachments[0].note_id = NoteId::new(7).unwrap();
        source.next_note_id = 8;
        source.validate().unwrap();

        let planned = base
            .plan_bundle_import(&source, BundleCollisionPolicy::KeepBoth, 500, [5; 32])
            .unwrap();

        assert_eq!(
            planned.plan().note_mappings,
            vec![(NoteId::new(7).unwrap(), NoteId::new(7).unwrap())]
        );
        assert_eq!(planned.candidate().next_note_id, 8);

        source.notes[0].id = NoteId::new(3).unwrap();
        source.attachments[0].note_id = NoteId::new(3).unwrap();
        source.next_note_id = 4;
        source.validate().unwrap();
        let planned = base
            .plan_bundle_import(&source, BundleCollisionPolicy::KeepBoth, 500, [5; 32])
            .unwrap();
        assert_eq!(planned.plan().note_mappings[0].1, NoteId::new(5).unwrap());
    }

    #[test]
    fn source_tombstones_limits_and_tampered_candidates_fail_closed() {
        let base = LibrarySnapshot::default();
        let mut tombstoned = source();
        tombstoned.notes[0].attachments.clear();
        tombstoned.attachments[0].deleted = true;
        tombstoned.validate().unwrap();
        assert_eq!(
            base.plan_bundle_import(&tombstoned, BundleCollisionPolicy::KeepBoth, 500, [5; 32]),
            Err(BundlePlanError::InvalidSourceAttachments)
        );

        let source = source();
        let planned = base
            .plan_bundle_import(&source, BundleCollisionPolicy::KeepBoth, 500, [5; 32])
            .unwrap();
        let mut tampered = planned.candidate().clone();
        tampered.notes[0].title = "tampered".into();
        assert_eq!(
            planned.plan().validate_candidate(&base, &source, &tampered),
            Err(BundlePlanError::InvalidPlan)
        );
    }
}
