use std::fmt;

use crate::{
    AttachmentId, AttachmentKind, FolderId, LibrarySnapshot, NoteId, NoteRecord, ValidationError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportScope {
    Note {
        note_id: NoteId,
        expected_note_revision: u64,
    },
    Folder {
        folder_id: FolderId,
        expected_folder_revision: u64,
    },
    Library {
        expected_library_revision: u64,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExportAttachment {
    pub id: AttachmentId,
    pub note_id: NoteId,
    pub kind: AttachmentKind,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

impl fmt::Debug for ExportAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExportAttachment")
            .field("id", &self.id)
            .field("note_id", &self.note_id)
            .field("kind", &self.kind)
            .field("byte_len", &self.byte_len)
            .field("sha256", &"<redacted>")
            .finish()
    }
}

/// Exact, path-free export review derived from one accepted library revision.
#[derive(Clone, PartialEq, Eq)]
pub struct ExportPlan {
    pub library_revision: u64,
    pub scope: ExportScope,
    pub note_ids: Vec<NoteId>,
    pub attachments: Vec<ExportAttachment>,
    pub markdown_bytes: u64,
    pub attachment_bytes: u64,
}

impl fmt::Debug for ExportPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExportPlan")
            .field("library_revision", &self.library_revision)
            .field("scope", &self.scope)
            .field("note_count", &self.note_ids.len())
            .field("attachment_count", &self.attachments.len())
            .field("markdown_bytes", &self.markdown_bytes)
            .field("attachment_bytes", &self.attachment_bytes)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportError {
    InvalidLibrary(ValidationError),
    RevisionConflict,
    MissingNote,
    MissingFolder,
    DeletedFolder,
    CollectionLimit,
    MarkdownRequiresSingleNote,
    MarkdownHasAttachments,
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLibrary(error) => return error.fmt(formatter),
            Self::RevisionConflict => "The Notes export review is stale",
            Self::MissingNote => "The note selected for export no longer exists",
            Self::MissingFolder => "The Notes folder selected for export no longer exists",
            Self::DeletedFolder => "The Notes folder selected for export is deleted",
            Self::CollectionLimit => "The Notes export exceeds a checked size limit",
            Self::MarkdownRequiresSingleNote => "Markdown export requires one selected note",
            Self::MarkdownHasAttachments => "Use a Notes bundle to export a note with attachments",
        })
    }
}

impl std::error::Error for ExportError {}

impl LibrarySnapshot {
    pub fn plan_export(&self, scope: ExportScope) -> Result<ExportPlan, ExportError> {
        self.validate().map_err(ExportError::InvalidLibrary)?;
        let mut note_ids = match scope {
            ExportScope::Note {
                note_id,
                expected_note_revision,
            } => {
                let note = self
                    .notes
                    .iter()
                    .find(|note| note.id == note_id)
                    .ok_or(ExportError::MissingNote)?;
                if note.revision != expected_note_revision {
                    return Err(ExportError::RevisionConflict);
                }
                vec![note_id]
            }
            ExportScope::Folder {
                folder_id,
                expected_folder_revision,
            } => {
                let folder = self
                    .folders
                    .iter()
                    .find(|folder| folder.id == folder_id)
                    .ok_or(ExportError::MissingFolder)?;
                if folder.revision != expected_folder_revision {
                    return Err(ExportError::RevisionConflict);
                }
                if folder.deleted {
                    return Err(ExportError::DeletedFolder);
                }
                self.notes
                    .iter()
                    .filter(|note| !note.deleted && note.folder_id == Some(folder_id))
                    .map(|note| note.id)
                    .collect()
            }
            ExportScope::Library {
                expected_library_revision,
            } => {
                if self.revision != expected_library_revision {
                    return Err(ExportError::RevisionConflict);
                }
                self.notes.iter().map(|note| note.id).collect()
            }
        };
        note_ids.sort_unstable();

        let mut attachments = self
            .attachments
            .iter()
            .filter(|attachment| {
                !attachment.deleted && note_ids.binary_search(&attachment.note_id).is_ok()
            })
            .map(|attachment| ExportAttachment {
                id: attachment.id,
                note_id: attachment.note_id,
                kind: attachment.kind,
                byte_len: attachment.byte_len,
                sha256: attachment.sha256,
            })
            .collect::<Vec<_>>();
        attachments.sort_by_key(|attachment| attachment.id);

        let markdown_bytes = self
            .notes
            .iter()
            .filter(|note| note_ids.binary_search(&note.id).is_ok())
            .try_fold(0_u64, |total, note| {
                total
                    .checked_add(render_export_markdown(note).len() as u64)
                    .ok_or(ExportError::CollectionLimit)
            })?;
        let attachment_bytes = attachments.iter().try_fold(0_u64, |total, attachment| {
            total
                .checked_add(attachment.byte_len)
                .ok_or(ExportError::CollectionLimit)
        })?;
        Ok(ExportPlan {
            library_revision: self.revision,
            scope,
            note_ids,
            attachments,
            markdown_bytes,
            attachment_bytes,
        })
    }
}

impl ExportPlan {
    pub fn validate(&self, snapshot: &LibrarySnapshot) -> Result<(), ExportError> {
        if snapshot.revision != self.library_revision {
            return Err(ExportError::RevisionConflict);
        }
        let exact = snapshot.plan_export(self.scope)?;
        if exact != *self {
            return Err(ExportError::RevisionConflict);
        }
        Ok(())
    }

    pub fn manifest(&self, snapshot: &LibrarySnapshot) -> Result<LibrarySnapshot, ExportError> {
        self.validate(snapshot)?;
        let mut manifest = snapshot.clone();
        manifest
            .notes
            .retain(|note| self.note_ids.binary_search(&note.id).is_ok());
        manifest.attachments.retain(|attachment| {
            self.attachments
                .binary_search_by_key(&attachment.id, |planned| planned.id)
                .is_ok()
        });
        manifest.folders.retain(|folder| match self.scope {
            ExportScope::Library { .. } => true,
            ExportScope::Folder { folder_id, .. } => folder.id == folder_id,
            ExportScope::Note { .. } => manifest
                .notes
                .iter()
                .any(|note| note.folder_id == Some(folder.id)),
        });
        manifest.validate().map_err(ExportError::InvalidLibrary)?;
        Ok(manifest)
    }

    /// Human-readable Markdown is intentionally content-only. A note with
    /// managed attachments must use the bundle format so export cannot silently
    /// lose referenced bytes.
    pub fn single_note_markdown(&self, snapshot: &LibrarySnapshot) -> Result<Vec<u8>, ExportError> {
        self.validate(snapshot)?;
        let ExportScope::Note { note_id, .. } = self.scope else {
            return Err(ExportError::MarkdownRequiresSingleNote);
        };
        if !self.attachments.is_empty() {
            return Err(ExportError::MarkdownHasAttachments);
        }
        let note = snapshot
            .notes
            .iter()
            .find(|note| note.id == note_id)
            .ok_or(ExportError::MissingNote)?;
        Ok(render_export_markdown(note))
    }
}

/// Deterministic UTF-8 companion document used by both Markdown and bundle
/// exports. Attachment bytes remain separate bundle entries.
pub fn render_export_markdown(note: &NoteRecord) -> Vec<u8> {
    let mut markdown = String::new();
    markdown.push_str("---\n");
    markdown.push_str("rmac-notes-export-version: 1\n");
    markdown.push_str(&format!("note-id: {}\n", note.id.get()));
    markdown.push_str(&format!("created-unix-ms: {}\n", note.created_unix_ms));
    markdown.push_str(&format!("modified-unix-ms: {}\n", note.modified_unix_ms));
    markdown.push_str(&format!("pinned: {}\n", note.pinned));
    markdown.push_str(&format!("trashed: {}\n", note.deleted));
    markdown.push_str("tags:\n");
    for tag in &note.tags {
        markdown.push_str("  - \"");
        push_quoted(&mut markdown, tag);
        markdown.push_str("\"\n");
    }
    markdown.push_str("---\n\n# ");
    markdown.push_str(&note.title);
    markdown.push_str("\n\n");
    markdown.push_str(&note.body);
    markdown.into_bytes()
}

fn push_quoted(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            _ => output.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttachmentRecord, FolderRecord, SortOrder};

    fn snapshot() -> LibrarySnapshot {
        let folder_id = FolderId::new(1).unwrap();
        let first_id = NoteId::new(1).unwrap();
        let second_id = NoteId::new(2).unwrap();
        let attachment_id = AttachmentId::new(1).unwrap();
        LibrarySnapshot {
            revision: 7,
            sort_order: SortOrder::Title,
            next_note_id: 3,
            next_folder_id: 2,
            next_attachment_id: 2,
            folders: vec![FolderRecord {
                id: folder_id,
                revision: 2,
                name: "Projects".into(),
                deleted: false,
            }],
            notes: vec![
                NoteRecord {
                    id: first_id,
                    revision: 3,
                    created_unix_ms: 10,
                    modified_unix_ms: 20,
                    title: "Roadmap".into(),
                    body: "Keep every byte 🦀".into(),
                    tags: vec!["quote \" and slash \\".into()],
                    folder_id: Some(folder_id),
                    pinned: true,
                    deleted: false,
                    attachments: vec![attachment_id],
                },
                NoteRecord {
                    id: second_id,
                    revision: 1,
                    created_unix_ms: 11,
                    modified_unix_ms: 21,
                    title: "Loose".into(),
                    body: "Plain body\n".into(),
                    tags: Vec::new(),
                    folder_id: None,
                    pinned: false,
                    deleted: false,
                    attachments: Vec::new(),
                },
            ],
            attachments: vec![AttachmentRecord {
                id: attachment_id,
                revision: 1,
                note_id: first_id,
                display_name: "private name.png".into(),
                kind: AttachmentKind::Png,
                byte_len: 4,
                sha256: [9; 32],
                deleted: false,
            }],
        }
    }

    #[test]
    fn exact_scopes_are_stable_sorted_and_revision_checked() {
        let snapshot = snapshot();
        let note = snapshot
            .plan_export(ExportScope::Note {
                note_id: NoteId::new(2).unwrap(),
                expected_note_revision: 1,
            })
            .unwrap();
        assert_eq!(note.note_ids, vec![NoteId::new(2).unwrap()]);
        assert!(note.attachments.is_empty());
        assert!(
            String::from_utf8(note.single_note_markdown(&snapshot).unwrap())
                .unwrap()
                .contains("# Loose\n\nPlain body\n")
        );

        let folder = snapshot
            .plan_export(ExportScope::Folder {
                folder_id: FolderId::new(1).unwrap(),
                expected_folder_revision: 2,
            })
            .unwrap();
        assert_eq!(folder.note_ids, vec![NoteId::new(1).unwrap()]);
        assert_eq!(folder.attachments[0].id, AttachmentId::new(1).unwrap());
        assert_eq!(folder.manifest(&snapshot).unwrap().folders.len(), 1);
        assert_eq!(
            folder.single_note_markdown(&snapshot),
            Err(ExportError::MarkdownRequiresSingleNote)
        );

        assert_eq!(
            snapshot.plan_export(ExportScope::Library {
                expected_library_revision: 6,
            }),
            Err(ExportError::RevisionConflict)
        );
    }

    #[test]
    fn markdown_is_utf8_deterministic_escaped_and_never_drops_attachments() {
        let snapshot = snapshot();
        let plan = snapshot
            .plan_export(ExportScope::Note {
                note_id: NoteId::new(1).unwrap(),
                expected_note_revision: 3,
            })
            .unwrap();
        assert_eq!(
            plan.single_note_markdown(&snapshot),
            Err(ExportError::MarkdownHasAttachments)
        );
        let markdown = render_export_markdown(&snapshot.notes[0]);
        let markdown = String::from_utf8(markdown).unwrap();
        assert!(markdown.contains("quote \\\" and slash \\\\"));
        assert!(markdown.ends_with("# Roadmap\n\nKeep every byte 🦀"));
        assert!(!format!("{plan:?}").contains("private name"));
        assert!(!format!("{:?}", plan.attachments[0]).contains("[9, 9"));
    }

    #[test]
    fn library_manifest_excludes_unreferenced_tombstones_but_retains_identity_sequences() {
        let mut snapshot = snapshot();
        snapshot.attachments.push(AttachmentRecord {
            id: AttachmentId::new(2).unwrap(),
            revision: 2,
            note_id: NoteId::new(2).unwrap(),
            display_name: "removed.png".into(),
            kind: AttachmentKind::Png,
            byte_len: 8,
            sha256: [8; 32],
            deleted: true,
        });
        snapshot.next_attachment_id = 3;
        snapshot.validate().unwrap();
        let plan = snapshot
            .plan_export(ExportScope::Library {
                expected_library_revision: 7,
            })
            .unwrap();
        let manifest = plan.manifest(&snapshot).unwrap();

        assert_eq!(plan.attachments.len(), 1);
        assert_eq!(manifest.attachments.len(), 1);
        assert_eq!(manifest.next_attachment_id, 3);
        assert!(manifest.validate().is_ok());
    }
}
