use std::fmt;
use std::fs::File;
use std::io::{self, Read as _, Seek as _, SeekFrom};
use std::path::{Component, Path, PathBuf};

use rmac_notes_store::{
    decode, encode, render_export_markdown, AttachmentId, BundleCollisionPolicy, BundleImportPlan,
    BundleImportReview, BundlePlanError, CodecError, LibrarySnapshot, PlannedBundleImport,
    MAX_ATTACHMENTS, MAX_LIBRARY_BYTES,
};
use rmac_storage::{
    fingerprint_bounded_regular_no_follow, open_regular_no_follow, Backend, FileFingerprint,
};
use sha2::{Digest as _, Sha256};

use crate::{
    attachment::validate_image_bytes,
    export::{BUNDLE_MAGIC, BUNDLE_VERSION},
    managed_attachment_path, PreviewError, MAX_EXPORT_BUNDLE_BYTES, MAX_IMPORTED_IMAGE_BYTES,
};

const INTENT_MAGIC: &[u8; 8] = b"RMNBIMP\0";
const INTENT_VERSION: u16 = 1;
pub(crate) const MAX_BUNDLE_IMPORT_INTENT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleImportOperation {
    ReviewSource,
    ReadHeader,
    DecodeManifest,
    VerifyNote,
    VerifyAttachment,
    RecheckSource,
    StageAttachment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleImportErrorKind {
    Io(io::ErrorKind),
    TooLarge,
    UnsupportedVersion,
    Malformed,
    HashMismatch,
    NonCanonicalManifest,
    Codec(CodecError),
    SourceChanged,
    Plan(BundlePlanError),
    AttachmentMismatch,
    UnsupportedAttachment,
    AttachmentTooLarge,
    AttachmentDecode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BundleImportError {
    pub operation: BundleImportOperation,
    pub kind: BundleImportErrorKind,
}

impl BundleImportError {
    fn new(operation: BundleImportOperation, kind: BundleImportErrorKind) -> Self {
        Self { operation, kind }
    }
}

impl fmt::Display for BundleImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            BundleImportErrorKind::Io(_) => "Notes could not read the selected bundle",
            BundleImportErrorKind::TooLarge => {
                "The selected Notes bundle exceeds its streaming safety bound"
            }
            BundleImportErrorKind::UnsupportedVersion => {
                "The selected Notes bundle uses an unsupported version"
            }
            BundleImportErrorKind::Malformed => "The selected Notes bundle is malformed",
            BundleImportErrorKind::HashMismatch => {
                "The selected Notes bundle failed content verification"
            }
            BundleImportErrorKind::NonCanonicalManifest => {
                "The selected Notes bundle manifest is not canonical"
            }
            BundleImportErrorKind::Codec(error) => return error.fmt(formatter),
            BundleImportErrorKind::SourceChanged => {
                "The selected Notes bundle changed after review"
            }
            BundleImportErrorKind::Plan(error) => return error.fmt(formatter),
            BundleImportErrorKind::AttachmentMismatch => {
                "Notes found conflicting managed bytes while importing the bundle"
            }
            BundleImportErrorKind::UnsupportedAttachment => {
                "The selected Notes bundle contains an unsupported attachment format"
            }
            BundleImportErrorKind::AttachmentTooLarge => {
                "The selected Notes bundle contains an oversized attachment"
            }
            BundleImportErrorKind::AttachmentDecode => {
                "The selected Notes bundle contains a malformed image attachment"
            }
        })
    }
}

impl std::error::Error for BundleImportError {}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BundleAttachmentEntry {
    source_id: AttachmentId,
    offset: u64,
    byte_len: u64,
    sha256: [u8; 32],
}

impl fmt::Debug for BundleAttachmentEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BundleAttachmentEntry")
            .field("source_id", &self.source_id)
            .field("offset", &self.offset)
            .field("byte_len", &self.byte_len)
            .field("sha256", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedBundleImport {
    source_path: PathBuf,
    source: FileFingerprint,
    manifest: LibrarySnapshot,
    attachments: Vec<BundleAttachmentEntry>,
}

impl fmt::Debug for PreparedBundleImport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedBundleImport")
            .field("source_path", &"<redacted>")
            .field("source_bytes", &self.source.byte_len)
            .field("source_sha256", &"<redacted>")
            .field("source_library_revision", &self.manifest.revision)
            .field("folder_count", &self.manifest.folders.len())
            .field("note_count", &self.manifest.notes.len())
            .field("attachment_count", &self.attachments.len())
            .finish()
    }
}

impl PreparedBundleImport {
    pub fn review(&self, base: &LibrarySnapshot) -> Result<BundleImportReview, BundlePlanError> {
        base.review_bundle_import(&self.manifest, self.source.byte_len, self.source.sha256)
    }

    pub fn plan(
        &self,
        base: &LibrarySnapshot,
        policy: BundleCollisionPolicy,
    ) -> Result<PlannedBundleImport, BundlePlanError> {
        base.plan_bundle_import(
            &self.manifest,
            policy,
            self.source.byte_len,
            self.source.sha256,
        )
    }

    pub(crate) fn source_is_within(&self, root: &Path) -> bool {
        self.source_path.starts_with(root)
    }

    fn validate_plan(
        &self,
        base: &LibrarySnapshot,
        candidate: &LibrarySnapshot,
        plan: &BundleImportPlan,
    ) -> Result<(), BundleImportError> {
        if plan.source_bytes != self.source.byte_len || plan.source_sha256() != self.source.sha256 {
            return Err(BundleImportError::new(
                BundleImportOperation::RecheckSource,
                BundleImportErrorKind::SourceChanged,
            ));
        }
        plan.validate_candidate(base, &self.manifest, candidate)
            .map_err(|error| {
                BundleImportError::new(
                    BundleImportOperation::RecheckSource,
                    BundleImportErrorKind::Plan(error),
                )
            })
    }

    fn stage<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
        plan: &BundleImportPlan,
    ) -> Result<(), BundleImportError> {
        self.recheck_source()?;
        let mut source = open_regular_no_follow(&self.source_path)
            .map_err(|error| io_failure(BundleImportOperation::RecheckSource, error))?;
        for planned in &plan.attachments {
            let entry = self
                .attachments
                .binary_search_by_key(&planned.source_id, |entry| entry.source_id)
                .ok()
                .and_then(|index| self.attachments.get(index))
                .ok_or_else(|| {
                    BundleImportError::new(
                        BundleImportOperation::StageAttachment,
                        BundleImportErrorKind::Plan(BundlePlanError::InvalidPlan),
                    )
                })?;
            if entry.byte_len != planned.byte_len || entry.sha256 != planned.sha256 {
                return Err(BundleImportError::new(
                    BundleImportOperation::StageAttachment,
                    BundleImportErrorKind::Plan(BundlePlanError::InvalidPlan),
                ));
            }
            let destination = managed_attachment_path(root, planned.destination_id);
            match backend.fingerprint_bounded_no_follow(&destination, planned.byte_len) {
                Ok(existing)
                    if existing.byte_len == planned.byte_len
                        && existing.sha256 == planned.sha256 =>
                {
                    continue;
                }
                Ok(_) => return Err(attachment_mismatch()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(io_failure(BundleImportOperation::StageAttachment, error))
                }
            }
            let parent = destination.parent().ok_or_else(|| {
                io_failure(
                    BundleImportOperation::StageAttachment,
                    io::Error::from(io::ErrorKind::InvalidInput),
                )
            })?;
            backend
                .create_dir_all_private(parent)
                .map_err(|error| io_failure(BundleImportOperation::StageAttachment, error))?;
            source
                .seek(SeekFrom::Start(entry.offset))
                .map_err(|error| io_failure(BundleImportOperation::StageAttachment, error))?;
            let mut range = (&mut source).take(entry.byte_len);
            let write = backend.write_new_private_stream(&destination, &mut range, entry.byte_len);
            match write {
                Ok(written)
                    if written.byte_len == entry.byte_len
                        && written.sha256 == entry.sha256
                        && range.limit() == 0 =>
                {
                    continue;
                }
                Ok(written) => {
                    remove_created_mismatch(backend, &destination, written);
                    return Err(attachment_mismatch());
                }
                Err(write_error) => {
                    match backend.fingerprint_bounded_no_follow(&destination, entry.byte_len) {
                        Ok(existing)
                            if existing.byte_len == entry.byte_len
                                && existing.sha256 == entry.sha256 =>
                        {
                            continue;
                        }
                        Ok(_) => return Err(attachment_mismatch()),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            return Err(io_failure(
                                BundleImportOperation::StageAttachment,
                                write_error,
                            ));
                        }
                        Err(_) => {
                            return Err(io_failure(
                                BundleImportOperation::StageAttachment,
                                write_error,
                            ));
                        }
                    }
                }
            }
        }
        self.recheck_source()
    }

    fn recheck_source(&self) -> Result<(), BundleImportError> {
        let current =
            fingerprint_bounded_regular_no_follow(&self.source_path, MAX_EXPORT_BUNDLE_BYTES)
                .map_err(|error| io_failure(BundleImportOperation::RecheckSource, error))?;
        if current != self.source {
            return Err(BundleImportError::new(
                BundleImportOperation::RecheckSource,
                BundleImportErrorKind::SourceChanged,
            ));
        }
        Ok(())
    }
}

pub fn prepare_bundle_import(path: &Path) -> Result<PreparedBundleImport, BundleImportError> {
    let source_path = canonical_selected_path(path)?;
    let file = open_regular_no_follow(&source_path)
        .map_err(|error| io_failure(BundleImportOperation::ReviewSource, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| io_failure(BundleImportOperation::ReviewSource, error))?;
    if metadata.len() == 0 || metadata.len() > MAX_EXPORT_BUNDLE_BYTES {
        return Err(BundleImportError::new(
            BundleImportOperation::ReviewSource,
            BundleImportErrorKind::TooLarge,
        ));
    }
    let mut reader = BundleReader::new(file);
    if reader.array(BundleImportOperation::ReadHeader)? != *BUNDLE_MAGIC {
        return Err(malformed(BundleImportOperation::ReadHeader));
    }
    let version = reader.u16(BundleImportOperation::ReadHeader)?;
    if version != BUNDLE_VERSION {
        return Err(BundleImportError::new(
            BundleImportOperation::ReadHeader,
            BundleImportErrorKind::UnsupportedVersion,
        ));
    }
    let library_revision = reader.u64(BundleImportOperation::ReadHeader)?;
    let manifest_bytes = reader.hashed_bytes(
        BundleImportOperation::DecodeManifest,
        MAX_LIBRARY_BYTES as u64,
    )?;
    let manifest = decode(&manifest_bytes).map_err(|error| {
        BundleImportError::new(
            BundleImportOperation::DecodeManifest,
            BundleImportErrorKind::Codec(error),
        )
    })?;
    if manifest.revision != library_revision {
        return Err(malformed(BundleImportOperation::DecodeManifest));
    }
    let canonical = encode(&manifest).map_err(|error| {
        BundleImportError::new(
            BundleImportOperation::DecodeManifest,
            BundleImportErrorKind::Codec(error),
        )
    })?;
    if canonical != manifest_bytes {
        return Err(BundleImportError::new(
            BundleImportOperation::DecodeManifest,
            BundleImportErrorKind::NonCanonicalManifest,
        ));
    }

    let note_count = reader.count(BundleImportOperation::VerifyNote, manifest.notes.len())?;
    if note_count != manifest.notes.len() {
        return Err(malformed(BundleImportOperation::VerifyNote));
    }
    let mut notes = manifest.notes.iter().collect::<Vec<_>>();
    notes.sort_by_key(|note| note.id);
    for note in notes {
        let note_id = reader.u64(BundleImportOperation::VerifyNote)?;
        if note_id != note.id.get() {
            return Err(malformed(BundleImportOperation::VerifyNote));
        }
        let expected = render_export_markdown(note);
        reader.compare_hashed_bytes(BundleImportOperation::VerifyNote, &expected)?;
    }

    if manifest
        .attachments
        .iter()
        .any(|attachment| attachment.deleted)
    {
        return Err(malformed(BundleImportOperation::VerifyAttachment));
    }
    let attachment_count = reader.count(
        BundleImportOperation::VerifyAttachment,
        manifest.attachments.len(),
    )?;
    if attachment_count != manifest.attachments.len() {
        return Err(malformed(BundleImportOperation::VerifyAttachment));
    }
    let mut manifest_attachments = manifest.attachments.iter().collect::<Vec<_>>();
    manifest_attachments.sort_by_key(|attachment| attachment.id);
    let mut attachments = Vec::with_capacity(attachment_count);
    for attachment in manifest_attachments {
        let id = reader.u64(BundleImportOperation::VerifyAttachment)?;
        let byte_len = reader.u64(BundleImportOperation::VerifyAttachment)?;
        let sha256 = reader.array(BundleImportOperation::VerifyAttachment)?;
        if id != attachment.id.get()
            || byte_len != attachment.byte_len
            || sha256 != attachment.sha256
        {
            return Err(malformed(BundleImportOperation::VerifyAttachment));
        }
        let offset = reader.position();
        let bytes = reader.exact_hashed_bytes(
            BundleImportOperation::VerifyAttachment,
            byte_len,
            sha256,
            MAX_IMPORTED_IMAGE_BYTES as u64,
        )?;
        validate_image_bytes(&bytes, attachment.kind).map_err(|error| {
            let kind = match error {
                PreviewError::Unsupported => BundleImportErrorKind::UnsupportedAttachment,
                PreviewError::TooLarge => BundleImportErrorKind::AttachmentTooLarge,
                _ => BundleImportErrorKind::AttachmentDecode,
            };
            BundleImportError::new(BundleImportOperation::VerifyAttachment, kind)
        })?;
        attachments.push(BundleAttachmentEntry {
            source_id: attachment.id,
            offset,
            byte_len,
            sha256,
        });
    }
    let source = reader.finish(metadata.len())?;
    Ok(PreparedBundleImport {
        source_path,
        source,
        manifest,
        attachments,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BundleIntentAuthority {
    RolledBack,
    Accepted,
    Ambiguous,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct IntentAttachment {
    destination_id: AttachmentId,
    byte_len: u64,
    sha256: [u8; 32],
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BundleImportIntent {
    base_library_revision: u64,
    candidate_library_revision: u64,
    source_library_revision: u64,
    source_bytes: u64,
    base_sha256: [u8; 32],
    candidate_sha256: [u8; 32],
    source_sha256: [u8; 32],
    attachments: Vec<IntentAttachment>,
}

impl fmt::Debug for BundleImportIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BundleImportIntent")
            .field("base_library_revision", &self.base_library_revision)
            .field(
                "candidate_library_revision",
                &self.candidate_library_revision,
            )
            .field("source_library_revision", &self.source_library_revision)
            .field("source_bytes", &self.source_bytes)
            .field("attachment_count", &self.attachments.len())
            .field("hashes", &"<redacted>")
            .finish()
    }
}

impl BundleImportIntent {
    pub(crate) fn prepare(
        base: &LibrarySnapshot,
        candidate: &LibrarySnapshot,
        plan: &BundleImportPlan,
        prepared: &PreparedBundleImport,
    ) -> Result<Self, BundleImportError> {
        prepared.validate_plan(base, candidate, plan)?;
        let base_bytes = encode(base).map_err(codec_plan_failure)?;
        let candidate_bytes = encode(candidate).map_err(codec_plan_failure)?;
        let mut attachments = plan
            .attachments
            .iter()
            .map(|attachment| IntentAttachment {
                destination_id: attachment.destination_id,
                byte_len: attachment.byte_len,
                sha256: attachment.sha256,
            })
            .collect::<Vec<_>>();
        attachments.sort_by_key(|attachment| attachment.destination_id);
        Ok(Self {
            base_library_revision: plan.base_library_revision,
            candidate_library_revision: plan.candidate_library_revision,
            source_library_revision: plan.source_library_revision,
            source_bytes: plan.source_bytes,
            base_sha256: Sha256::digest(base_bytes).into(),
            candidate_sha256: Sha256::digest(candidate_bytes).into(),
            source_sha256: plan.source_sha256(),
            attachments,
        })
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, BundleImportError> {
        let required = 8_usize
            .checked_add(2 + 8 * 4 + 32 * 3 + 4)
            .and_then(|size| size.checked_add(self.attachments.len().checked_mul(8 + 8 + 32)?))
            .ok_or_else(intent_too_large)?;
        if required > MAX_BUNDLE_IMPORT_INTENT_BYTES {
            return Err(intent_too_large());
        }
        let mut bytes = Vec::with_capacity(required);
        bytes.extend_from_slice(INTENT_MAGIC);
        bytes.extend_from_slice(&INTENT_VERSION.to_le_bytes());
        for value in [
            self.base_library_revision,
            self.candidate_library_revision,
            self.source_library_revision,
            self.source_bytes,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&self.base_sha256);
        bytes.extend_from_slice(&self.candidate_sha256);
        bytes.extend_from_slice(&self.source_sha256);
        bytes.extend_from_slice(&(self.attachments.len() as u32).to_le_bytes());
        for attachment in &self.attachments {
            bytes.extend_from_slice(&attachment.destination_id.get().to_le_bytes());
            bytes.extend_from_slice(&attachment.byte_len.to_le_bytes());
            bytes.extend_from_slice(&attachment.sha256);
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, BundleImportError> {
        if bytes.len() > MAX_BUNDLE_IMPORT_INTENT_BYTES {
            return Err(intent_too_large());
        }
        let mut reader = SliceReader::new(bytes);
        if reader.take(INTENT_MAGIC.len())? != INTENT_MAGIC || reader.u16()? != INTENT_VERSION {
            return Err(intent_malformed());
        }
        let base_library_revision = reader.u64()?;
        let candidate_library_revision = reader.u64()?;
        let source_library_revision = reader.u64()?;
        let source_bytes = reader.u64()?;
        let base_sha256 = reader.array()?;
        let candidate_sha256 = reader.array()?;
        let source_sha256 = reader.array()?;
        let count = reader.u32()? as usize;
        if count > MAX_ATTACHMENTS {
            return Err(intent_malformed());
        }
        let mut attachments = Vec::with_capacity(count);
        let mut previous = None;
        for _ in 0..count {
            let destination_id = AttachmentId::new(reader.u64()?).ok_or_else(intent_malformed)?;
            let byte_len = reader.u64()?;
            let sha256 = reader.array()?;
            if byte_len == 0
                || sha256 == [0; 32]
                || previous.is_some_and(|previous| previous >= destination_id)
            {
                return Err(intent_malformed());
            }
            previous = Some(destination_id);
            attachments.push(IntentAttachment {
                destination_id,
                byte_len,
                sha256,
            });
        }
        let intent = Self {
            base_library_revision,
            candidate_library_revision,
            source_library_revision,
            source_bytes,
            base_sha256,
            candidate_sha256,
            source_sha256,
            attachments,
        };
        if !reader.finished()
            || intent.base_library_revision == 0
            || intent.candidate_library_revision
                != intent
                    .base_library_revision
                    .checked_add(1)
                    .ok_or_else(intent_malformed)?
            || intent.source_library_revision == 0
            || intent.source_bytes == 0
            || intent.base_sha256 == [0; 32]
            || intent.candidate_sha256 == [0; 32]
            || intent.source_sha256 == [0; 32]
        {
            return Err(intent_malformed());
        }
        Ok(intent)
    }

    pub(crate) fn authority(
        &self,
        snapshot: &LibrarySnapshot,
    ) -> Result<BundleIntentAuthority, BundleImportError> {
        let bytes = encode(snapshot).map_err(codec_plan_failure)?;
        let sha256: [u8; 32] = Sha256::digest(bytes).into();
        if snapshot.revision == self.base_library_revision && sha256 == self.base_sha256 {
            Ok(BundleIntentAuthority::RolledBack)
        } else if snapshot.revision == self.candidate_library_revision
            && sha256 == self.candidate_sha256
        {
            Ok(BundleIntentAuthority::Accepted)
        } else {
            Ok(BundleIntentAuthority::Ambiguous)
        }
    }

    pub(crate) fn stage<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
        plan: &BundleImportPlan,
        prepared: &PreparedBundleImport,
    ) -> Result<(), BundleImportError> {
        if self.source_bytes != plan.source_bytes
            || self.source_sha256 != plan.source_sha256()
            || self.attachments.len() != plan.attachments.len()
        {
            return Err(plan_invalid());
        }
        prepared.stage(root, backend, plan)
    }

    pub(crate) fn verify_staged<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
    ) -> Result<(), BundleImportError> {
        for attachment in &self.attachments {
            let path = managed_attachment_path(root, attachment.destination_id);
            let actual = backend
                .fingerprint_bounded_no_follow(&path, attachment.byte_len)
                .map_err(|error| io_failure(BundleImportOperation::VerifyAttachment, error))?;
            if actual.byte_len != attachment.byte_len || actual.sha256 != attachment.sha256 {
                return Err(attachment_mismatch());
            }
        }
        Ok(())
    }

    pub(crate) fn rollback_staging<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
    ) -> Result<(), BundleImportError> {
        for attachment in &self.attachments {
            let path = managed_attachment_path(root, attachment.destination_id);
            match backend.fingerprint_bounded_no_follow(&path, attachment.byte_len) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(io_failure(BundleImportOperation::VerifyAttachment, error))
                }
                Ok(actual)
                    if actual.byte_len == attachment.byte_len
                        && actual.sha256 == attachment.sha256 => {}
                Ok(_) => return Err(attachment_mismatch()),
            }
            match backend.remove_file_durable(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(io_failure(BundleImportOperation::StageAttachment, error))
                }
            }
        }
        Ok(())
    }
}

struct BundleReader {
    file: File,
    position: u64,
    whole: Sha256,
}

impl BundleReader {
    fn new(file: File) -> Self {
        Self {
            file,
            position: 0,
            whole: Sha256::new(),
        }
    }

    fn position(&self) -> u64 {
        self.position
    }

    fn read_exact(
        &mut self,
        operation: BundleImportOperation,
        output: &mut [u8],
    ) -> Result<(), BundleImportError> {
        let mut cursor = 0;
        while cursor < output.len() {
            let count = self
                .file
                .read(&mut output[cursor..])
                .map_err(|error| io_failure(operation, error))?;
            if count == 0 {
                return Err(malformed(operation));
            }
            self.whole.update(&output[cursor..cursor + count]);
            self.position = self
                .position
                .checked_add(count as u64)
                .ok_or_else(|| malformed(operation))?;
            if self.position > MAX_EXPORT_BUNDLE_BYTES {
                return Err(BundleImportError::new(
                    operation,
                    BundleImportErrorKind::TooLarge,
                ));
            }
            cursor += count;
        }
        Ok(())
    }

    fn u16(&mut self, operation: BundleImportOperation) -> Result<u16, BundleImportError> {
        Ok(u16::from_le_bytes(self.array(operation)?))
    }

    fn u64(&mut self, operation: BundleImportOperation) -> Result<u64, BundleImportError> {
        Ok(u64::from_le_bytes(self.array(operation)?))
    }

    fn array<const N: usize>(
        &mut self,
        operation: BundleImportOperation,
    ) -> Result<[u8; N], BundleImportError> {
        let mut value = [0; N];
        self.read_exact(operation, &mut value)?;
        Ok(value)
    }

    fn count(
        &mut self,
        operation: BundleImportOperation,
        maximum: usize,
    ) -> Result<usize, BundleImportError> {
        let value = usize::try_from(self.u64(operation)?).map_err(|_| malformed(operation))?;
        if value > maximum {
            return Err(malformed(operation));
        }
        Ok(value)
    }

    fn hashed_bytes(
        &mut self,
        operation: BundleImportOperation,
        maximum: u64,
    ) -> Result<Vec<u8>, BundleImportError> {
        let byte_len = self.u64(operation)?;
        let sha256 = self.array(operation)?;
        if byte_len > maximum {
            return Err(BundleImportError::new(
                operation,
                BundleImportErrorKind::TooLarge,
            ));
        }
        let length = usize::try_from(byte_len)
            .map_err(|_| BundleImportError::new(operation, BundleImportErrorKind::TooLarge))?;
        let mut bytes = vec![0; length];
        self.read_exact(operation, &mut bytes)?;
        if <[u8; 32]>::from(Sha256::digest(&bytes)) != sha256 {
            return Err(BundleImportError::new(
                operation,
                BundleImportErrorKind::HashMismatch,
            ));
        }
        Ok(bytes)
    }

    fn compare_hashed_bytes(
        &mut self,
        operation: BundleImportOperation,
        expected: &[u8],
    ) -> Result<(), BundleImportError> {
        let byte_len = self.u64(operation)?;
        let sha256 = self.array(operation)?;
        if byte_len != expected.len() as u64 || sha256 != <[u8; 32]>::from(Sha256::digest(expected))
        {
            return Err(BundleImportError::new(
                operation,
                BundleImportErrorKind::HashMismatch,
            ));
        }
        let mut offset = 0;
        let mut buffer = [0_u8; 64 * 1024];
        while offset < expected.len() {
            let count = buffer.len().min(expected.len() - offset);
            self.read_exact(operation, &mut buffer[..count])?;
            if buffer[..count] != expected[offset..offset + count] {
                return Err(BundleImportError::new(
                    operation,
                    BundleImportErrorKind::HashMismatch,
                ));
            }
            offset += count;
        }
        Ok(())
    }

    fn exact_hashed_bytes(
        &mut self,
        operation: BundleImportOperation,
        byte_len: u64,
        expected_sha256: [u8; 32],
        maximum: u64,
    ) -> Result<Vec<u8>, BundleImportError> {
        if byte_len > maximum {
            return Err(BundleImportError::new(
                operation,
                BundleImportErrorKind::AttachmentTooLarge,
            ));
        }
        let length = usize::try_from(byte_len).map_err(|_| {
            BundleImportError::new(operation, BundleImportErrorKind::AttachmentTooLarge)
        })?;
        let mut remaining = byte_len;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut bytes = Vec::with_capacity(length);
        while remaining > 0 {
            let count = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| malformed(operation))?;
            self.read_exact(operation, &mut buffer[..count])?;
            hasher.update(&buffer[..count]);
            bytes.extend_from_slice(&buffer[..count]);
            remaining -= count as u64;
        }
        if <[u8; 32]>::from(hasher.finalize()) != expected_sha256 {
            return Err(BundleImportError::new(
                operation,
                BundleImportErrorKind::HashMismatch,
            ));
        }
        Ok(bytes)
    }

    fn finish(mut self, metadata_len: u64) -> Result<FileFingerprint, BundleImportError> {
        let mut extra = [0_u8; 1];
        let count = self
            .file
            .read(&mut extra)
            .map_err(|error| io_failure(BundleImportOperation::ReadHeader, error))?;
        if count != 0 || self.position != metadata_len {
            return Err(malformed(BundleImportOperation::ReadHeader));
        }
        Ok(FileFingerprint {
            byte_len: self.position,
            sha256: self.whole.finalize().into(),
        })
    }
}

struct SliceReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> SliceReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], BundleImportError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or_else(intent_malformed)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(intent_malformed)?;
        self.cursor = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, BundleImportError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, BundleImportError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, BundleImportError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], BundleImportError> {
        self.take(N)?.try_into().map_err(|_| intent_malformed())
    }

    fn finished(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

fn canonical_selected_path(path: &Path) -> Result<PathBuf, BundleImportError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || path.file_name().is_none()
    {
        return Err(malformed(BundleImportOperation::ReviewSource));
    }
    let parent = path
        .parent()
        .ok_or_else(|| malformed(BundleImportOperation::ReviewSource))?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|error| io_failure(BundleImportOperation::ReviewSource, error))?;
    Ok(parent.join(path.file_name().expect("validated above")))
}

fn remove_created_mismatch<B: Backend>(backend: &B, path: &Path, written: FileFingerprint) {
    if backend
        .fingerprint_bounded_no_follow(path, written.byte_len)
        .is_ok_and(|actual| actual == written)
    {
        let _ = backend.remove_file_durable(path);
    }
}

fn malformed(operation: BundleImportOperation) -> BundleImportError {
    BundleImportError::new(operation, BundleImportErrorKind::Malformed)
}

fn io_failure(operation: BundleImportOperation, error: io::Error) -> BundleImportError {
    BundleImportError::new(operation, BundleImportErrorKind::Io(error.kind()))
}

fn attachment_mismatch() -> BundleImportError {
    BundleImportError::new(
        BundleImportOperation::StageAttachment,
        BundleImportErrorKind::AttachmentMismatch,
    )
}

fn plan_invalid() -> BundleImportError {
    BundleImportError::new(
        BundleImportOperation::StageAttachment,
        BundleImportErrorKind::Plan(BundlePlanError::InvalidPlan),
    )
}

fn codec_plan_failure(error: CodecError) -> BundleImportError {
    BundleImportError::new(
        BundleImportOperation::StageAttachment,
        BundleImportErrorKind::Codec(error),
    )
}

fn intent_malformed() -> BundleImportError {
    malformed(BundleImportOperation::StageAttachment)
}

fn intent_too_large() -> BundleImportError {
    BundleImportError::new(
        BundleImportOperation::StageAttachment,
        BundleImportErrorKind::TooLarge,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::write_bundle;
    use image::ImageEncoder as _;
    use rmac_notes_store::{
        AttachmentKind, AttachmentRecord, ExportScope, NoteId, NoteRecord, SortOrder,
    };
    use rmac_storage::FileSystem;

    fn snapshot() -> (LibrarySnapshot, Vec<u8>) {
        let note_id = NoteId::new(1).unwrap();
        let attachment_id = AttachmentId::new(1).unwrap();
        let mut attachment_bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut attachment_bytes)
            .write_image(&[12, 34, 56, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        (
            LibrarySnapshot {
                revision: 4,
                sort_order: SortOrder::Edited,
                next_note_id: 2,
                next_folder_id: 1,
                next_attachment_id: 2,
                folders: Vec::new(),
                notes: vec![NoteRecord {
                    id: note_id,
                    revision: 2,
                    created_unix_ms: 1,
                    modified_unix_ms: 2,
                    title: "Private bundle note".into(),
                    body: "Private body".into(),
                    tags: Vec::new(),
                    folder_id: None,
                    pinned: false,
                    deleted: false,
                    attachments: vec![attachment_id],
                }],
                attachments: vec![AttachmentRecord {
                    id: attachment_id,
                    revision: 1,
                    note_id,
                    display_name: "private.png".into(),
                    kind: AttachmentKind::Png,
                    byte_len: attachment_bytes.len() as u64,
                    sha256: Sha256::digest(&attachment_bytes).into(),
                    deleted: false,
                }],
            },
            attachment_bytes,
        )
    }

    fn bundle() -> (Vec<u8>, LibrarySnapshot, Vec<u8>) {
        let (snapshot, attachment_bytes) = snapshot();
        let plan = snapshot
            .plan_export(ExportScope::Library {
                expected_library_revision: snapshot.revision,
            })
            .unwrap();
        let manifest = encode(&plan.manifest(&snapshot).unwrap()).unwrap();
        let mut bundle = Vec::new();
        write_bundle(&snapshot, &plan, &manifest, &mut bundle, |_, output| {
            use std::io::Write as _;
            output.write_all(&attachment_bytes)
        })
        .unwrap();
        (bundle, snapshot, attachment_bytes)
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rmac-bundle-import-{label}-{}", std::process::id()))
    }

    #[test]
    fn preparation_streams_and_verifies_the_complete_canonical_bundle() {
        let root = temp_root("valid");
        std::fs::create_dir_all(&root).unwrap();
        let source_path = root.join("private.rmacnotes");
        let (bundle, snapshot, _) = bundle();
        std::fs::write(&source_path, &bundle).unwrap();

        let prepared = prepare_bundle_import(&source_path).unwrap();
        let review = prepared.review(&LibrarySnapshot::default()).unwrap();

        assert_eq!(prepared.manifest, snapshot);
        assert_eq!(review.note_count, 1);
        assert_eq!(review.attachment_count, 1);
        assert_eq!(review.source_bytes, bundle.len() as u64);
        assert!(!format!("{prepared:?}").contains("private.rmacnotes"));
        assert!(!format!("{prepared:?}").contains("Private bundle note"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_hash_trailing_data_and_source_change_fail_closed() {
        let root = temp_root("invalid");
        std::fs::create_dir_all(&root).unwrap();
        let source_path = root.join("bundle.rmacnotes");
        let (mut corrupted, _, _) = bundle();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xff;
        std::fs::write(&source_path, &corrupted).unwrap();
        assert!(matches!(
            prepare_bundle_import(&source_path),
            Err(BundleImportError {
                kind: BundleImportErrorKind::HashMismatch,
                ..
            })
        ));

        let (mut trailing, _, _) = bundle();
        trailing.push(0);
        std::fs::write(&source_path, &trailing).unwrap();
        assert!(matches!(
            prepare_bundle_import(&source_path),
            Err(BundleImportError {
                kind: BundleImportErrorKind::Malformed,
                ..
            })
        ));

        let (valid, _, _) = bundle();
        std::fs::write(&source_path, &valid).unwrap();
        let prepared = prepare_bundle_import(&source_path).unwrap();
        std::fs::write(&source_path, b"changed").unwrap();
        assert_eq!(
            prepared.recheck_source(),
            Err(BundleImportError::new(
                BundleImportOperation::RecheckSource,
                BundleImportErrorKind::SourceChanged
            ))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn intent_round_trips_and_classifies_exact_authority() {
        let root = temp_root("intent");
        std::fs::create_dir_all(&root).unwrap();
        let source_path = root.join("bundle.rmacnotes");
        let (bundle, _, _) = bundle();
        std::fs::write(&source_path, bundle).unwrap();
        let prepared = prepare_bundle_import(&source_path).unwrap();
        let base = LibrarySnapshot::default();
        let planned = prepared
            .plan(&base, BundleCollisionPolicy::KeepBoth)
            .unwrap();
        let intent =
            BundleImportIntent::prepare(&base, planned.candidate(), planned.plan(), &prepared)
                .unwrap();
        let decoded = BundleImportIntent::decode(&intent.encode().unwrap()).unwrap();

        assert_eq!(decoded, intent);
        assert_eq!(
            intent.authority(&base).unwrap(),
            BundleIntentAuthority::RolledBack
        );
        assert_eq!(
            intent.authority(planned.candidate()).unwrap(),
            BundleIntentAuthority::Accepted
        );
        assert!(!format!("{intent:?}").contains("["));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_store_stages_exact_attachments_then_publishes_the_complete_candidate() {
        let container = temp_root("store");
        let source_root = container.join("source");
        let library_root = container.join("library");
        std::fs::create_dir_all(&source_root).unwrap();
        let source_path = source_root.join("bundle.rmacnotes");
        let (bundle, _, attachment_bytes) = bundle();
        std::fs::write(&source_path, &bundle).unwrap();
        let store = crate::NotesLibraryStore::new(library_root.clone()).unwrap();
        let loaded = store.load().unwrap();
        let prepared = store.prepare_bundle_import(&source_path).unwrap();
        let planned = prepared
            .plan(loaded.snapshot(), BundleCollisionPolicy::KeepBoth)
            .unwrap();
        let destination_id = planned.plan().attachments[0].destination_id;
        let candidate = planned.candidate().clone();

        let outcome = store
            .save_bundle_import(&loaded, planned.candidate(), planned.plan(), &prepared)
            .unwrap();

        assert_eq!(outcome.library.snapshot(), &candidate);
        assert!(!outcome.maintenance_pending);
        assert!(!outcome.bundle_import_pending);
        assert_eq!(
            std::fs::read(crate::managed_attachment_path(
                &library_root,
                destination_id
            ))
            .unwrap(),
            attachment_bytes
        );
        assert!(!library_root.join("library.bundle-import.bin").exists());

        let inside = library_root.join("inside.rmacnotes");
        std::fs::write(&inside, bundle).unwrap();
        assert!(matches!(
            store.prepare_bundle_import(&inside),
            Err(BundleImportError {
                operation: BundleImportOperation::ReviewSource,
                kind: BundleImportErrorKind::Malformed,
            })
        ));
        drop(store);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn changed_reviewed_source_rolls_back_its_persisted_intent_without_metadata() {
        let container = temp_root("changed-source");
        let source_root = container.join("source");
        let library_root = container.join("library");
        std::fs::create_dir_all(&source_root).unwrap();
        let source_path = source_root.join("bundle.rmacnotes");
        let (bundle, _, _) = bundle();
        std::fs::write(&source_path, bundle).unwrap();
        let store = crate::NotesLibraryStore::new(library_root.clone()).unwrap();
        let loaded = store.load().unwrap();
        let prepared = store.prepare_bundle_import(&source_path).unwrap();
        let planned = prepared
            .plan(loaded.snapshot(), BundleCollisionPolicy::KeepBoth)
            .unwrap();
        std::fs::write(&source_path, b"changed after review").unwrap();

        let error = store
            .save_bundle_import(&loaded, planned.candidate(), planned.plan(), &prepared)
            .unwrap_err();

        assert_eq!(error.kind, crate::ErrorKind::InvalidBundleImport);
        assert_eq!(store.load().unwrap().snapshot(), loaded.snapshot());
        assert!(!library_root.join("library.bundle-import.bin").exists());
        drop(store);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn startup_rolls_back_exact_staging_when_bundle_metadata_was_not_published() {
        let container = temp_root("rollback-recovery");
        let source_root = container.join("source");
        let library_root = container.join("library");
        std::fs::create_dir_all(&source_root).unwrap();
        let source_path = source_root.join("bundle.rmacnotes");
        let (bundle, _, _) = bundle();
        std::fs::write(&source_path, bundle).unwrap();
        let store = crate::NotesLibraryStore::new(library_root.clone()).unwrap();
        let loaded = store.load().unwrap();
        let prepared = store.prepare_bundle_import(&source_path).unwrap();
        let planned = prepared
            .plan(loaded.snapshot(), BundleCollisionPolicy::KeepBoth)
            .unwrap();
        let intent = BundleImportIntent::prepare(
            loaded.snapshot(),
            planned.candidate(),
            planned.plan(),
            &prepared,
        )
        .unwrap();
        let attachment_path =
            managed_attachment_path(&library_root, planned.plan().attachments[0].destination_id);
        store.write_bundle_import_intent(&intent).unwrap();
        intent
            .stage(&library_root, &FileSystem, planned.plan(), &prepared)
            .unwrap();
        assert!(attachment_path.exists());

        let recovered = store.load().unwrap();

        assert_eq!(recovered.snapshot(), loaded.snapshot());
        assert!(!attachment_path.exists());
        assert!(!store.bundle_import_path().exists());
        assert!(recovered
            .notices()
            .contains(&crate::RecoveryNotice::RolledBackInterruptedBundleImport));
        drop(store);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn startup_finishes_exact_staging_when_bundle_metadata_was_published() {
        let container = temp_root("accepted-recovery");
        let source_root = container.join("source");
        let library_root = container.join("library");
        std::fs::create_dir_all(&source_root).unwrap();
        let source_path = source_root.join("bundle.rmacnotes");
        let (bundle, _, attachment_bytes) = bundle();
        std::fs::write(&source_path, bundle).unwrap();
        let store = crate::NotesLibraryStore::new(library_root.clone()).unwrap();
        let loaded = store.load().unwrap();
        let prepared = store.prepare_bundle_import(&source_path).unwrap();
        let planned = prepared
            .plan(loaded.snapshot(), BundleCollisionPolicy::KeepBoth)
            .unwrap();
        let intent = BundleImportIntent::prepare(
            loaded.snapshot(),
            planned.candidate(),
            planned.plan(),
            &prepared,
        )
        .unwrap();
        let attachment_path =
            managed_attachment_path(&library_root, planned.plan().attachments[0].destination_id);
        store.write_bundle_import_intent(&intent).unwrap();
        intent
            .stage(&library_root, &FileSystem, planned.plan(), &prepared)
            .unwrap();
        let committed = store.save_locked(&loaded, planned.candidate()).unwrap();
        assert_eq!(committed.library.snapshot(), planned.candidate());
        assert!(store.bundle_import_path().exists());

        let recovered = store.load().unwrap();

        assert_eq!(recovered.snapshot(), planned.candidate());
        assert_eq!(std::fs::read(attachment_path).unwrap(), attachment_bytes);
        assert!(!store.bundle_import_path().exists());
        assert!(recovered
            .notices()
            .contains(&crate::RecoveryNotice::FinishedInterruptedBundleImport));
        drop(store);
        std::fs::remove_dir_all(container).unwrap();
    }
}
