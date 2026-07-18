use std::fmt;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};

use rmac_notes_store::{
    encode, render_export_markdown, CodecError, ExportAttachment, ExportError as DomainExportError,
    ExportPlan, LibrarySnapshot,
};
use rmac_storage::{
    atomic_write_stream_checked, copy_verified_private_file, inspect_destination,
    DestinationBaseline, FileFingerprint, FileSystem, VerifiedCopyError,
};
use sha2::{Digest as _, Sha256};

use crate::{has_blocking_notice, managed_attachment_path, LoadedLibrary, NotesLibraryStore};

const BUNDLE_MAGIC: &[u8; 8] = b"RMNBNDL\0";
const BUNDLE_VERSION: u16 = 1;
pub const MAX_EXPORT_BUNDLE_BYTES: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    RmacBundle,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedExportDestination {
    path: PathBuf,
    baseline: DestinationBaseline,
}

impl PreparedExportDestination {
    pub fn review(path: PathBuf) -> Result<Self, ExportFailure> {
        if !is_normal_absolute(&path) || path.file_name().is_none() {
            return Err(ExportFailure::new(
                ExportOperation::ReviewDestination,
                ExportFailureKind::InvalidDestination,
            ));
        }
        let file_name = path.file_name().expect("validated above").to_owned();
        let parent = path.parent().ok_or_else(|| {
            ExportFailure::new(
                ExportOperation::ReviewDestination,
                ExportFailureKind::InvalidDestination,
            )
        })?;
        let canonical_parent = std::fs::canonicalize(parent)
            .map_err(|error| map_io(ExportOperation::ReviewDestination, error))?;
        let path = canonical_parent.join(file_name);
        let baseline = inspect_destination(&path, MAX_EXPORT_BUNDLE_BYTES)
            .map_err(|error| map_io(ExportOperation::ReviewDestination, error))?;
        Ok(Self { path, baseline })
    }
}

impl fmt::Debug for PreparedExportDestination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedExportDestination")
            .field("path", &"<redacted>")
            .field("baseline", &self.baseline)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExportOutcome {
    pub format: ExportFormat,
    pub library_revision: u64,
    pub note_count: usize,
    pub attachment_count: usize,
    pub markdown_bytes: u64,
    pub attachment_bytes: u64,
    pub output: FileFingerprint,
}

impl fmt::Debug for ExportOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExportOutcome")
            .field("format", &self.format)
            .field("library_revision", &self.library_revision)
            .field("note_count", &self.note_count)
            .field("attachment_count", &self.attachment_count)
            .field("markdown_bytes", &self.markdown_bytes)
            .field("attachment_bytes", &self.attachment_bytes)
            .field("output_bytes", &self.output.byte_len)
            .field("output_sha256", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportOperation {
    ReviewDestination,
    ValidatePlan,
    EncodeManifest,
    ReadManagedAttachment,
    WriteDestination,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFailureKind {
    Io(io::ErrorKind),
    Domain(DomainExportError),
    Manifest(CodecError),
    InvalidDestination,
    DestinationConflict,
    LibraryConflict,
    AttachmentMismatch,
    TooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportFailure {
    pub operation: ExportOperation,
    pub kind: ExportFailureKind,
}

impl ExportFailure {
    fn new(operation: ExportOperation, kind: ExportFailureKind) -> Self {
        Self { operation, kind }
    }
}

impl fmt::Display for ExportFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ExportFailureKind::Io(_) => "Notes could not complete the selected export",
            ExportFailureKind::Domain(error) => return error.fmt(formatter),
            ExportFailureKind::Manifest(error) => return error.fmt(formatter),
            ExportFailureKind::InvalidDestination => {
                "Notes cannot use the selected export destination"
            }
            ExportFailureKind::DestinationConflict => {
                "The export destination changed after it was selected"
            }
            ExportFailureKind::LibraryConflict => {
                "The Notes library changed or needs recovery before export"
            }
            ExportFailureKind::AttachmentMismatch => {
                "A managed attachment changed before Notes could export it"
            }
            ExportFailureKind::TooLarge => "The Notes export exceeds its streaming safety bound",
        })
    }
}

impl std::error::Error for ExportFailure {}

impl NotesLibraryStore<FileSystem> {
    pub fn export(
        &self,
        loaded: &LoadedLibrary,
        plan: &ExportPlan,
        format: ExportFormat,
        destination: PreparedExportDestination,
    ) -> Result<ExportOutcome, ExportFailure> {
        if destination.path.starts_with(&self.root) {
            return Err(ExportFailure::new(
                ExportOperation::ReviewDestination,
                ExportFailureKind::InvalidDestination,
            ));
        }
        let _guard = self
            .transaction_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if has_blocking_notice(loaded.notices()) {
            return Err(ExportFailure::new(
                ExportOperation::ValidatePlan,
                ExportFailureKind::LibraryConflict,
            ));
        }
        self.preflight(&loaded.baseline).map_err(|error| {
            let kind = match error.kind {
                crate::ErrorKind::Io(kind) => ExportFailureKind::Io(kind),
                _ => ExportFailureKind::LibraryConflict,
            };
            ExportFailure::new(ExportOperation::ValidatePlan, kind)
        })?;
        plan.validate(loaded.snapshot()).map_err(domain_failure)?;

        let output = match format {
            ExportFormat::Markdown => {
                let markdown = plan
                    .single_note_markdown(loaded.snapshot())
                    .map_err(domain_failure)?;
                atomic_write_stream_checked(
                    &destination.path,
                    destination.baseline,
                    MAX_EXPORT_BUNDLE_BYTES,
                    |file| file.write_all(&markdown),
                )
                .map_err(|error| map_io(ExportOperation::WriteDestination, error))?
            }
            ExportFormat::RmacBundle => {
                let manifest = plan.manifest(loaded.snapshot()).map_err(domain_failure)?;
                let manifest_bytes = encode(&manifest).map_err(|error| {
                    let kind = match error {
                        CodecError::TooLarge => ExportFailureKind::TooLarge,
                        error => ExportFailureKind::Manifest(error),
                    };
                    ExportFailure::new(ExportOperation::EncodeManifest, kind)
                })?;
                let expected_bytes = bundle_size(plan, manifest_bytes.len() as u64)?;
                if expected_bytes > MAX_EXPORT_BUNDLE_BYTES {
                    return Err(ExportFailure::new(
                        ExportOperation::ValidatePlan,
                        ExportFailureKind::TooLarge,
                    ));
                }
                let root = self.root.clone();
                let snapshot = loaded.snapshot();
                let mut copy_failure = None;
                let result = atomic_write_stream_checked(
                    &destination.path,
                    destination.baseline,
                    MAX_EXPORT_BUNDLE_BYTES,
                    |file| {
                        write_bundle(snapshot, plan, &manifest_bytes, file, |attachment, file| {
                            match copy_verified_private_file(
                                &managed_attachment_path(&root, attachment.id),
                                FileFingerprint {
                                    byte_len: attachment.byte_len,
                                    sha256: attachment.sha256,
                                },
                                file,
                            ) {
                                Ok(()) => Ok(()),
                                Err(error) => {
                                    copy_failure = Some(error);
                                    Err(io::Error::from(error.kind()))
                                }
                            }
                        })
                    },
                );
                result.map_err(|error| match copy_failure {
                    Some(VerifiedCopyError::Source(
                        io::ErrorKind::InvalidData | io::ErrorKind::NotFound,
                    )) => ExportFailure::new(
                        ExportOperation::ReadManagedAttachment,
                        ExportFailureKind::AttachmentMismatch,
                    ),
                    Some(VerifiedCopyError::Source(kind)) => ExportFailure::new(
                        ExportOperation::ReadManagedAttachment,
                        ExportFailureKind::Io(kind),
                    ),
                    Some(VerifiedCopyError::Destination(kind)) => ExportFailure::new(
                        ExportOperation::WriteDestination,
                        ExportFailureKind::Io(kind),
                    ),
                    None => map_io(ExportOperation::WriteDestination, error),
                })?
            }
        };

        Ok(ExportOutcome {
            format,
            library_revision: plan.library_revision,
            note_count: plan.note_ids.len(),
            attachment_count: plan.attachments.len(),
            markdown_bytes: plan.markdown_bytes,
            attachment_bytes: plan.attachment_bytes,
            output,
        })
    }
}

fn write_bundle<W, F>(
    snapshot: &LibrarySnapshot,
    plan: &ExportPlan,
    manifest: &[u8],
    output: &mut W,
    mut copy_attachment: F,
) -> io::Result<()>
where
    W: io::Write,
    F: FnMut(&ExportAttachment, &mut W) -> io::Result<()>,
{
    output.write_all(BUNDLE_MAGIC)?;
    output.write_all(&BUNDLE_VERSION.to_le_bytes())?;
    output.write_all(&plan.library_revision.to_le_bytes())?;
    write_bytes(output, manifest)?;
    output.write_all(&(plan.note_ids.len() as u64).to_le_bytes())?;
    let mut notes = snapshot
        .notes
        .iter()
        .filter(|note| plan.note_ids.binary_search(&note.id).is_ok())
        .collect::<Vec<_>>();
    notes.sort_by_key(|note| note.id);
    if notes.len() != plan.note_ids.len() {
        return Err(invalid_bundle());
    }
    for (note_id, note) in plan.note_ids.iter().zip(notes) {
        if note.id != *note_id {
            return Err(invalid_bundle());
        }
        let markdown = render_export_markdown(note);
        output.write_all(&note_id.get().to_le_bytes())?;
        write_bytes(output, &markdown)?;
    }
    output.write_all(&(plan.attachments.len() as u64).to_le_bytes())?;
    for attachment in &plan.attachments {
        output.write_all(&attachment.id.get().to_le_bytes())?;
        output.write_all(&attachment.byte_len.to_le_bytes())?;
        output.write_all(&attachment.sha256)?;
        copy_attachment(attachment, output)?;
    }
    Ok(())
}

fn write_bytes(output: &mut impl io::Write, bytes: &[u8]) -> io::Result<()> {
    output.write_all(&(bytes.len() as u64).to_le_bytes())?;
    output.write_all(&Sha256::digest(bytes))?;
    output.write_all(bytes)
}

fn bundle_size(plan: &ExportPlan, manifest_bytes: u64) -> Result<u64, ExportFailure> {
    let fixed = 8_u64 + 2 + 8 + 8 + 32 + 8 + 8;
    let note_headers = (plan.note_ids.len() as u64)
        .checked_mul(8 + 8 + 32)
        .ok_or_else(too_large)?;
    let attachment_headers = (plan.attachments.len() as u64)
        .checked_mul(8 + 8 + 32)
        .ok_or_else(too_large)?;
    fixed
        .checked_add(manifest_bytes)
        .and_then(|size| size.checked_add(note_headers))
        .and_then(|size| size.checked_add(plan.markdown_bytes))
        .and_then(|size| size.checked_add(attachment_headers))
        .and_then(|size| size.checked_add(plan.attachment_bytes))
        .ok_or_else(too_large)
}

fn too_large() -> ExportFailure {
    ExportFailure::new(ExportOperation::ValidatePlan, ExportFailureKind::TooLarge)
}

fn domain_failure(error: DomainExportError) -> ExportFailure {
    ExportFailure::new(
        ExportOperation::ValidatePlan,
        ExportFailureKind::Domain(error),
    )
}

fn map_io(operation: ExportOperation, error: io::Error) -> ExportFailure {
    let kind = match error.kind() {
        io::ErrorKind::AlreadyExists => ExportFailureKind::DestinationConflict,
        kind => ExportFailureKind::Io(kind),
    };
    ExportFailure::new(operation, kind)
}

fn invalid_bundle() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "export plan does not match snapshot",
    )
}

fn is_normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notes_store::{
        AttachmentId, AttachmentKind, AttachmentRecord, ExportScope, FolderId, FolderRecord,
        LibraryTransaction, NewAttachment, NewNote, NoteId, NoteRecord, SortOrder,
        MAX_LIBRARY_BYTES,
    };
    use std::collections::BTreeMap;

    fn snapshot() -> (LibrarySnapshot, BTreeMap<AttachmentId, Vec<u8>>) {
        let note_id = NoteId::new(1).unwrap();
        let attachment_id = AttachmentId::new(1).unwrap();
        let bytes = b"exact private attachment".to_vec();
        let snapshot = LibrarySnapshot {
            revision: 5,
            sort_order: SortOrder::Edited,
            next_note_id: 2,
            next_folder_id: 2,
            next_attachment_id: 2,
            folders: vec![FolderRecord {
                id: FolderId::new(1).unwrap(),
                revision: 1,
                name: "Projects".into(),
                deleted: false,
            }],
            notes: vec![NoteRecord {
                id: note_id,
                revision: 3,
                created_unix_ms: 10,
                modified_unix_ms: 20,
                title: "Private roadmap".into(),
                body: "Keep every byte".into(),
                tags: vec!["private-tag".into()],
                folder_id: Some(FolderId::new(1).unwrap()),
                pinned: true,
                deleted: false,
                attachments: vec![attachment_id],
            }],
            attachments: vec![AttachmentRecord {
                id: attachment_id,
                revision: 1,
                note_id,
                display_name: "private.png".into(),
                kind: AttachmentKind::Png,
                byte_len: bytes.len() as u64,
                sha256: Sha256::digest(&bytes).into(),
                deleted: false,
            }],
        };
        (snapshot, BTreeMap::from([(attachment_id, bytes)]))
    }

    #[test]
    fn bundle_stream_is_versioned_deterministic_and_contains_only_planned_bytes() {
        let (snapshot, attachments) = snapshot();
        let plan = snapshot
            .plan_export(ExportScope::Library {
                expected_library_revision: 5,
            })
            .unwrap();
        let manifest = encode(&plan.manifest(&snapshot).unwrap()).unwrap();
        let mut output = Vec::new();
        write_bundle(
            &snapshot,
            &plan,
            &manifest,
            &mut output,
            |attachment, output| output.write_all(attachments.get(&attachment.id).unwrap()),
        )
        .unwrap();

        assert!(output.starts_with(BUNDLE_MAGIC));
        assert_eq!(
            output.len() as u64,
            bundle_size(&plan, manifest.len() as u64).unwrap()
        );
        assert!(output
            .windows(b"exact private attachment".len())
            .any(|window| window == b"exact private attachment"));
        assert!(!format!("{plan:?}").contains("Private roadmap"));
    }

    #[test]
    fn destination_review_is_absolute_normal_and_path_private() {
        let relative = PreparedExportDestination::review(PathBuf::from("private.md"));
        assert_eq!(
            relative,
            Err(ExportFailure::new(
                ExportOperation::ReviewDestination,
                ExportFailureKind::InvalidDestination
            ))
        );

        let root =
            std::env::temp_dir().join(format!("rmac-notes-export-review-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let destination = root.join("private-name.rmacnotes");
        let reviewed = PreparedExportDestination::review(destination).unwrap();
        assert!(!format!("{reviewed:?}").contains("private-name"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checked_size_matches_every_binary_field() {
        let (snapshot, _) = snapshot();
        let plan = snapshot
            .plan_export(ExportScope::Note {
                note_id: NoteId::new(1).unwrap(),
                expected_note_revision: 3,
            })
            .unwrap();
        let manifest = encode(&plan.manifest(&snapshot).unwrap()).unwrap();
        assert!(bundle_size(&plan, manifest.len() as u64).unwrap() < MAX_EXPORT_BUNDLE_BYTES);
        assert!(manifest.len() <= MAX_LIBRARY_BYTES);
    }

    #[test]
    fn real_store_exports_verified_bundle_without_mutating_the_library() {
        let container =
            std::env::temp_dir().join(format!("rmac-notes-real-export-{}", std::process::id()));
        let library_root = container.join("library");
        let export_root = container.join("exports");
        std::fs::create_dir_all(&export_root).unwrap();
        let store = NotesLibraryStore::new(library_root.clone()).unwrap();
        let initial = store.load().unwrap();
        let mut create = LibraryTransaction::begin(initial.snapshot()).unwrap();
        let note_id = create
            .create_note(NewNote {
                created_unix_ms: 10,
                title: "Private roadmap".into(),
                body: "Keep every byte".into(),
                tags: vec!["private-tag".into()],
                folder_id: None,
            })
            .unwrap();
        let loaded = store
            .save(&initial, &create.finish().unwrap())
            .unwrap()
            .library;
        let attachment_bytes = b"verified managed attachment".to_vec();
        let mut attach = LibraryTransaction::begin(loaded.snapshot()).unwrap();
        let attachment = attach
            .add_attachment(
                note_id,
                1,
                11,
                NewAttachment {
                    display_name: "private.png".into(),
                    kind: AttachmentKind::Png,
                    byte_len: attachment_bytes.len() as u64,
                    sha256: Sha256::digest(&attachment_bytes).into(),
                },
            )
            .unwrap();
        let loaded = store
            .save(&loaded, &attach.finish().unwrap())
            .unwrap()
            .library;
        std::fs::create_dir_all(library_root.join("attachments")).unwrap();
        std::fs::write(
            managed_attachment_path(&library_root, attachment.attachment_id),
            &attachment_bytes,
        )
        .unwrap();
        let plan = loaded
            .snapshot()
            .plan_export(ExportScope::Note {
                note_id,
                expected_note_revision: 2,
            })
            .unwrap();
        let destination_path = export_root.join("roadmap.rmacnotes");
        let destination = PreparedExportDestination::review(destination_path.clone()).unwrap();

        let outcome = store
            .export(&loaded, &plan, ExportFormat::RmacBundle, destination)
            .unwrap();

        assert_eq!(outcome.note_count, 1);
        assert_eq!(outcome.attachment_count, 1);
        assert_eq!(outcome.attachment_bytes, attachment_bytes.len() as u64);
        assert!(std::fs::read(&destination_path)
            .unwrap()
            .windows(attachment_bytes.len())
            .any(|window| window == attachment_bytes));
        assert_eq!(store.load().unwrap().snapshot(), loaded.snapshot());

        std::fs::write(
            managed_attachment_path(&library_root, attachment.attachment_id),
            b"substituted managed bytes",
        )
        .unwrap();
        let rejected_path = export_root.join("substituted.rmacnotes");
        let rejected_destination =
            PreparedExportDestination::review(rejected_path.clone()).unwrap();
        assert_eq!(
            store.export(
                &loaded,
                &plan,
                ExportFormat::RmacBundle,
                rejected_destination
            ),
            Err(ExportFailure::new(
                ExportOperation::ReadManagedAttachment,
                ExportFailureKind::AttachmentMismatch
            ))
        );
        assert!(!rejected_path.exists());

        let inside = PreparedExportDestination::review(library_root.join("forbidden.md")).unwrap();
        assert_eq!(
            store.export(&loaded, &plan, ExportFormat::RmacBundle, inside),
            Err(ExportFailure::new(
                ExportOperation::ReviewDestination,
                ExportFailureKind::InvalidDestination
            ))
        );

        std::fs::write(library_root.join("library.bin"), b"external library change").unwrap();
        let conflicted_path = export_root.join("conflicted.rmacnotes");
        let conflicted_destination =
            PreparedExportDestination::review(conflicted_path.clone()).unwrap();
        assert_eq!(
            store.export(
                &loaded,
                &plan,
                ExportFormat::RmacBundle,
                conflicted_destination
            ),
            Err(ExportFailure::new(
                ExportOperation::ValidatePlan,
                ExportFailureKind::LibraryConflict
            ))
        );
        assert!(!conflicted_path.exists());
        drop(store);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn changed_managed_attachment_aborts_before_destination_publish() {
        let (snapshot, mut attachment_bytes) = snapshot();
        let plan = snapshot
            .plan_export(ExportScope::Library {
                expected_library_revision: 5,
            })
            .unwrap();
        let manifest = encode(&plan.manifest(&snapshot).unwrap()).unwrap();
        attachment_bytes.insert(AttachmentId::new(1).unwrap(), b"changed".to_vec());
        let mut output = Vec::new();

        let result = write_bundle(
            &snapshot,
            &plan,
            &manifest,
            &mut output,
            |attachment, output| {
                let bytes = attachment_bytes.get(&attachment.id).unwrap();
                if bytes.len() as u64 != attachment.byte_len
                    || <[u8; 32]>::from(Sha256::digest(bytes)) != attachment.sha256
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "changed attachment",
                    ));
                }
                output.write_all(bytes)
            },
        );

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
