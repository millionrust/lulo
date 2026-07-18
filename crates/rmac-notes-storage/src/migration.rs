use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rmac_notes_store::{
    AttachmentId, AttachmentKind, AttachmentRecord, FolderId, FolderRecord, LibrarySnapshot,
    NoteId, NoteRecord, SortOrder, ValidationError, MAX_ATTACHMENTS, MAX_ATTACHMENTS_PER_NOTE,
    MAX_ATTACHMENT_BYTES, MAX_BODY_BYTES, MAX_FOLDERS, MAX_LIBRARY_BYTES, MAX_NAME_BYTES,
    MAX_NOTES, MAX_TAGS_PER_NOTE, MAX_TITLE_BYTES,
};
use sha2::{Digest as _, Sha256};

const MAX_LEGACY_PATH_BYTES: usize = 4096;
const MAX_TOTAL_ATTACHMENT_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyNoteInput {
    pub relative_path: String,
    pub bytes: Vec<u8>,
    pub created_unix_ms: u64,
    pub modified_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyAttachmentInput {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyLibraryInput {
    pub notes: Vec<LegacyNoteInput>,
    pub attachments: Vec<LegacyAttachmentInput>,
    pub pinned_note_paths: Vec<String>,
    pub sort_order: SortOrder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedNoteSource {
    pub note_id: NoteId,
    pub relative_path: String,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedAttachment {
    pub attachment_id: AttachmentId,
    pub note_id: NoteId,
    pub relative_path: String,
    pub byte_len: u64,
    pub sha256: [u8; 32],
    pub kind: AttachmentKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryFile {
    pub relative_path: String,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrationWarning {
    StalePins(usize),
    DuplicateTags(usize),
    UnsupportedAttachmentReferences(usize),
    UnclaimedFiles(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationPlan {
    pub snapshot: LibrarySnapshot,
    pub note_sources: Vec<PlannedNoteSource>,
    pub attachments: Vec<PlannedAttachment>,
    pub recovery_files: Vec<RecoveryFile>,
    pub warnings: Vec<MigrationWarning>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationError {
    CollectionLimit,
    SourceTooLarge,
    InvalidPath,
    DuplicatePath,
    NormalizedFolderCollision,
    InvalidUtf8,
    InvalidTimestamp,
    InvalidMetadata,
    InvalidSnapshot(ValidationError),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CollectionLimit => "the legacy Notes library exceeds a collection safety limit",
            Self::SourceTooLarge => "the legacy Notes library exceeds a byte safety limit",
            Self::InvalidPath => "the legacy Notes library contains an invalid relative path",
            Self::DuplicatePath => "the legacy Notes library contains a duplicate path",
            Self::NormalizedFolderCollision => {
                "legacy folder names collide after normalization and need review"
            }
            Self::InvalidUtf8 => "a legacy note is not valid UTF-8",
            Self::InvalidTimestamp => "a legacy note contains invalid timestamps",
            Self::InvalidMetadata => "a legacy note contains unsupported metadata",
            Self::InvalidSnapshot(error) => return error.fmt(formatter),
        })
    }
}

impl std::error::Error for MigrationError {}

pub fn plan_legacy_library(mut input: LegacyLibraryInput) -> Result<MigrationPlan, MigrationError> {
    if input.notes.len() > MAX_NOTES
        || input.attachments.len() > MAX_ATTACHMENTS
        || input.pinned_note_paths.len() > MAX_NOTES
    {
        return Err(MigrationError::CollectionLimit);
    }
    let note_bytes = input
        .notes
        .iter()
        .try_fold(0_usize, |total, note| total.checked_add(note.bytes.len()));
    if note_bytes.is_none_or(|total| total > MAX_LIBRARY_BYTES) {
        return Err(MigrationError::SourceTooLarge);
    }
    let attachment_bytes = input
        .attachments
        .iter()
        .try_fold(0_u64, |total, attachment| {
            total.checked_add(attachment.bytes.len() as u64)
        });
    if attachment_bytes.is_none_or(|total| total > MAX_TOTAL_ATTACHMENT_BYTES) {
        return Err(MigrationError::SourceTooLarge);
    }

    input
        .notes
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    input
        .attachments
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    reject_duplicate_paths(input.notes.iter().map(|note| note.relative_path.as_str()))?;
    reject_duplicate_paths(
        input
            .attachments
            .iter()
            .map(|attachment| attachment.relative_path.as_str()),
    )?;

    let parsed_paths = input
        .notes
        .iter()
        .map(|note| parse_note_path(&note.relative_path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut folder_names = parsed_paths
        .iter()
        .filter_map(|path| path.folder.clone())
        .collect::<Vec<_>>();
    folder_names.sort();
    folder_names.dedup();
    if folder_names.len() > MAX_FOLDERS {
        return Err(MigrationError::CollectionLimit);
    }
    let mut normalized_folders = BTreeSet::new();
    if folder_names
        .iter()
        .any(|name| !normalized_folders.insert(name.to_lowercase()))
    {
        return Err(MigrationError::NormalizedFolderCollision);
    }

    let mut folders = Vec::with_capacity(folder_names.len());
    let mut folder_ids = BTreeMap::new();
    for (index, name) in folder_names.into_iter().enumerate() {
        let id = FolderId::new(index as u64 + 1).ok_or(MigrationError::CollectionLimit)?;
        folder_ids.insert(name.clone(), id);
        folders.push(FolderRecord {
            id,
            revision: 1,
            name,
            deleted: false,
        });
    }

    let pinned = input
        .pinned_note_paths
        .iter()
        .map(|path| validate_relative_path(path).map(|_| path.as_str()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let attachment_inputs = input
        .attachments
        .iter()
        .map(|attachment| {
            validate_attachment_path(&attachment.relative_path)?;
            Ok((attachment.relative_path.as_str(), attachment))
        })
        .collect::<Result<BTreeMap<_, _>, MigrationError>>()?;

    let mut notes = Vec::with_capacity(input.notes.len());
    let mut note_sources = Vec::with_capacity(input.notes.len());
    let mut attachments = Vec::new();
    let mut planned_attachments = Vec::new();
    let mut claimed_files = BTreeSet::new();
    let mut duplicate_tags = 0_usize;
    let mut unsupported_references = 0_usize;
    let mut next_attachment_id = 1_u64;

    for ((index, source), parsed_path) in input.notes.iter().enumerate().zip(parsed_paths) {
        if source.modified_unix_ms < source.created_unix_ms {
            return Err(MigrationError::InvalidTimestamp);
        }
        let text = std::str::from_utf8(&source.bytes).map_err(|_| MigrationError::InvalidUtf8)?;
        let (title, body, raw_tags) = parse_legacy_document(text);
        if title.len() > MAX_TITLE_BYTES || body.len() > MAX_BODY_BYTES {
            return Err(MigrationError::SourceTooLarge);
        }
        let (tags, removed_tags) = normalize_tags(raw_tags)?;
        duplicate_tags = duplicate_tags.saturating_add(removed_tags);
        let note_id = NoteId::new(index as u64 + 1).ok_or(MigrationError::CollectionLimit)?;
        let mut attachment_ids = Vec::new();
        let mut note_targets = BTreeSet::new();
        for target in image_targets(&body) {
            if !note_targets.insert(target.clone()) {
                continue;
            }
            if validate_attachment_path(&target).is_err() {
                unsupported_references = unsupported_references.saturating_add(1);
                continue;
            }
            let Some(attachment) = attachment_inputs.get(target.as_str()) else {
                unsupported_references = unsupported_references.saturating_add(1);
                continue;
            };
            let Some(kind) = attachment_kind(&attachment.bytes) else {
                unsupported_references = unsupported_references.saturating_add(1);
                continue;
            };
            if attachment.bytes.len() as u64 > MAX_ATTACHMENT_BYTES
                || attachment_ids.len() >= MAX_ATTACHMENTS_PER_NOTE
            {
                return Err(MigrationError::SourceTooLarge);
            }
            let attachment_id =
                AttachmentId::new(next_attachment_id).ok_or(MigrationError::CollectionLimit)?;
            next_attachment_id = next_attachment_id
                .checked_add(1)
                .ok_or(MigrationError::CollectionLimit)?;
            let hash = digest(&attachment.bytes);
            attachment_ids.push(attachment_id);
            claimed_files.insert(attachment.relative_path.as_str());
            attachments.push(AttachmentRecord {
                id: attachment_id,
                revision: 1,
                note_id,
                display_name: attachment.relative_path.clone(),
                kind,
                byte_len: attachment.bytes.len() as u64,
                sha256: hash,
                deleted: false,
            });
            planned_attachments.push(PlannedAttachment {
                attachment_id,
                note_id,
                relative_path: attachment.relative_path.clone(),
                byte_len: attachment.bytes.len() as u64,
                sha256: hash,
                kind,
            });
        }
        let pinned_note = pinned.contains(source.relative_path.as_str());
        notes.push(NoteRecord {
            id: note_id,
            revision: 1,
            created_unix_ms: source.created_unix_ms,
            modified_unix_ms: source.modified_unix_ms,
            title,
            body,
            tags,
            folder_id: parsed_path
                .folder
                .as_ref()
                .and_then(|name| folder_ids.get(name).copied()),
            pinned: pinned_note,
            deleted: false,
            attachments: attachment_ids,
        });
        note_sources.push(PlannedNoteSource {
            note_id,
            relative_path: source.relative_path.clone(),
            byte_len: source.bytes.len() as u64,
            sha256: digest(&source.bytes),
        });
    }

    let recovery_files = input
        .attachments
        .iter()
        .filter(|attachment| !claimed_files.contains(attachment.relative_path.as_str()))
        .map(|attachment| RecoveryFile {
            relative_path: attachment.relative_path.clone(),
            byte_len: attachment.bytes.len() as u64,
            sha256: digest(&attachment.bytes),
        })
        .collect::<Vec<_>>();
    let stale_pins = pinned
        .iter()
        .filter(|path| {
            !input
                .notes
                .iter()
                .any(|note| note.relative_path.as_str() == **path)
        })
        .count();
    let snapshot = LibrarySnapshot {
        revision: 2,
        sort_order: input.sort_order,
        next_note_id: notes.len() as u64 + 1,
        next_folder_id: folders.len() as u64 + 1,
        next_attachment_id,
        folders,
        notes,
        attachments,
    };
    snapshot
        .validate()
        .map_err(MigrationError::InvalidSnapshot)?;

    let mut warnings = Vec::new();
    if stale_pins > 0 {
        warnings.push(MigrationWarning::StalePins(stale_pins));
    }
    if duplicate_tags > 0 {
        warnings.push(MigrationWarning::DuplicateTags(duplicate_tags));
    }
    if unsupported_references > 0 {
        warnings.push(MigrationWarning::UnsupportedAttachmentReferences(
            unsupported_references,
        ));
    }
    if !recovery_files.is_empty() {
        warnings.push(MigrationWarning::UnclaimedFiles(recovery_files.len()));
    }
    Ok(MigrationPlan {
        snapshot,
        note_sources,
        attachments: planned_attachments,
        recovery_files,
        warnings,
    })
}

#[derive(Clone, Debug)]
struct ParsedNotePath {
    folder: Option<String>,
}

fn parse_note_path(path: &str) -> Result<ParsedNotePath, MigrationError> {
    validate_relative_path(path)?;
    let parts = path.split('/').collect::<Vec<_>>();
    if !parts.last().is_some_and(|name| name.ends_with(".md")) {
        return Err(MigrationError::InvalidPath);
    }
    match parts.as_slice() {
        [_name] => Ok(ParsedNotePath { folder: None }),
        [folder, _name] => {
            validate_display_name(folder)?;
            Ok(ParsedNotePath {
                folder: Some((*folder).to_string()),
            })
        }
        _ => Err(MigrationError::InvalidPath),
    }
}

fn validate_attachment_path(path: &str) -> Result<(), MigrationError> {
    validate_relative_path(path)?;
    if path.contains('/') || path.ends_with(".md") || matches!(path, ".pinned" | ".sort") {
        return Err(MigrationError::InvalidPath);
    }
    validate_display_name(path)
}

fn validate_relative_path(path: &str) -> Result<(), MigrationError> {
    if path.is_empty()
        || path.len() > MAX_LEGACY_PATH_BYTES
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        Err(MigrationError::InvalidPath)
    } else {
        Ok(())
    }
}

fn validate_display_name(name: &str) -> Result<(), MigrationError> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        Err(MigrationError::InvalidMetadata)
    } else {
        Ok(())
    }
}

fn reject_duplicate_paths<'a>(paths: impl Iterator<Item = &'a str>) -> Result<(), MigrationError> {
    let mut unique = BTreeSet::new();
    for path in paths {
        validate_relative_path(path)?;
        if !unique.insert(path) {
            return Err(MigrationError::DuplicatePath);
        }
    }
    Ok(())
}

fn parse_legacy_document(text: &str) -> (String, String, Vec<String>) {
    let mut tags = Vec::new();
    let mut lines = text.lines().collect::<Vec<_>>();
    if let Some(last) = lines.last() {
        if let Some(raw_tags) = last
            .trim()
            .strip_prefix("<!--tags:")
            .and_then(|value| value.strip_suffix("-->"))
        {
            tags = raw_tags.split(',').map(str::to_string).collect();
            lines.pop();
        }
    }
    let joined = lines.join("\n");
    let mut document = joined.splitn(2, '\n');
    (
        document.next().unwrap_or_default().to_string(),
        document.next().unwrap_or_default().to_string(),
        tags,
    )
}

fn normalize_tags(tags: Vec<String>) -> Result<(Vec<String>, usize), MigrationError> {
    if tags.len() > MAX_TAGS_PER_NOTE.saturating_mul(4) {
        return Err(MigrationError::CollectionLimit);
    }
    let mut normalized = Vec::new();
    let mut identities = BTreeSet::new();
    let mut duplicates = 0;
    for raw in tags {
        let tag = raw.trim().trim_start_matches('#').trim();
        if tag.is_empty() {
            continue;
        }
        if tag.len() > rmac_notes_store::MAX_TAG_BYTES || tag.chars().any(char::is_control) {
            return Err(MigrationError::InvalidMetadata);
        }
        if identities.insert(tag.to_lowercase()) {
            normalized.push(tag.to_string());
        } else {
            duplicates += 1;
        }
    }
    if normalized.len() > MAX_TAGS_PER_NOTE {
        return Err(MigrationError::CollectionLimit);
    }
    Ok((normalized, duplicates))
}

fn image_targets(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            let open = line.find("](")?;
            let end = line.rfind(')')?;
            if !line.starts_with("![") || end <= open + 2 {
                return None;
            }
            let target = &line[open + 2..end];
            Some(target.to_string())
        })
        .collect()
}

fn attachment_kind(bytes: &[u8]) -> Option<AttachmentKind> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(AttachmentKind::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(AttachmentKind::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(AttachmentKind::Gif)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(AttachmentKind::WebP)
    } else {
        None
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        b"\x89PNG\r\n\x1a\nfixture".to_vec()
    }

    fn fixture() -> LegacyLibraryInput {
        LegacyLibraryInput {
            notes: vec![
                LegacyNoteInput {
                    relative_path: "note-1.md".into(),
                    bytes: b"Root\nSee image\n![](diagram.png)\n<!--tags: rmac, RMAC, plan-->"
                        .to_vec(),
                    created_unix_ms: 10,
                    modified_unix_ms: 20,
                },
                LegacyNoteInput {
                    relative_path: "Projects/note-2.md".into(),
                    bytes: b"Second\nBody".to_vec(),
                    created_unix_ms: 30,
                    modified_unix_ms: 40,
                },
            ],
            attachments: vec![
                LegacyAttachmentInput {
                    relative_path: "diagram.png".into(),
                    bytes: png(),
                },
                LegacyAttachmentInput {
                    relative_path: "orphan.bin".into(),
                    bytes: b"unclaimed".to_vec(),
                },
            ],
            pinned_note_paths: vec!["Projects/note-2.md".into(), "missing.md".into()],
            sort_order: SortOrder::Created,
        }
    }

    #[test]
    fn migration_preserves_notes_folders_pins_sort_and_safe_attachments() {
        let plan = plan_legacy_library(fixture()).unwrap();
        assert_eq!(plan.snapshot.revision, 2);
        assert_eq!(plan.snapshot.sort_order, SortOrder::Created);
        assert_eq!(plan.snapshot.folders[0].name, "Projects");
        assert_eq!(plan.snapshot.notes.len(), 2);
        assert_eq!(plan.snapshot.notes[0].title, "Second");
        assert!(plan.snapshot.notes[0].pinned);
        assert_eq!(plan.snapshot.notes[1].title, "Root");
        assert_eq!(plan.snapshot.notes[1].tags, ["rmac", "plan"]);
        assert_eq!(plan.attachments.len(), 1);
        assert_eq!(plan.recovery_files[0].relative_path, "orphan.bin");
        assert!(plan.warnings.contains(&MigrationWarning::StalePins(1)));
        assert!(plan.warnings.contains(&MigrationWarning::DuplicateTags(1)));
        assert!(plan.warnings.contains(&MigrationWarning::UnclaimedFiles(1)));
    }

    #[test]
    fn input_order_does_not_change_stable_ids_or_plan() {
        let input = fixture();
        let mut reversed = input.clone();
        reversed.notes.reverse();
        reversed.attachments.reverse();
        assert_eq!(
            plan_legacy_library(input).unwrap(),
            plan_legacy_library(reversed).unwrap()
        );
    }

    #[test]
    fn invalid_paths_utf8_timestamps_and_folder_collisions_fail_closed() {
        let mut traversal = fixture();
        traversal.notes[0].relative_path = "../escape.md".into();
        assert_eq!(
            plan_legacy_library(traversal),
            Err(MigrationError::InvalidPath)
        );

        let mut utf8 = fixture();
        utf8.notes[0].bytes = vec![0xff];
        assert_eq!(plan_legacy_library(utf8), Err(MigrationError::InvalidUtf8));

        let mut timestamps = fixture();
        timestamps.notes[0].created_unix_ms = 30;
        timestamps.notes[0].modified_unix_ms = 20;
        assert_eq!(
            plan_legacy_library(timestamps),
            Err(MigrationError::InvalidTimestamp)
        );

        let mut collision = fixture();
        collision.notes.push(LegacyNoteInput {
            relative_path: "projects/note-3.md".into(),
            bytes: b"Third".to_vec(),
            created_unix_ms: 1,
            modified_unix_ms: 1,
        });
        assert_eq!(
            plan_legacy_library(collision),
            Err(MigrationError::NormalizedFolderCollision)
        );
    }

    #[test]
    fn unsupported_references_remain_text_and_are_reported() {
        let mut input = fixture();
        input.notes[0].bytes = b"Root\n![](orphan.bin)".to_vec();
        let plan = plan_legacy_library(input).unwrap();
        assert!(plan.snapshot.notes[1].body.contains("![](orphan.bin)"));
        assert!(plan
            .warnings
            .contains(&MigrationWarning::UnsupportedAttachmentReferences(1)));
        assert!(plan
            .recovery_files
            .iter()
            .any(|file| file.relative_path == "orphan.bin"));
    }
}
