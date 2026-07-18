use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Cursor};
use std::path::Path;

use image::ImageReader;
use rmac_notes_store::{
    encode, AttachmentId, AttachmentImportPlan, AttachmentKind, LibrarySnapshot, NewAttachment,
    NoteId, MAX_NAME_BYTES,
};
use rmac_storage::{Backend, FileFingerprint};
use sha2::{Digest as _, Sha256};

use crate::managed_attachment_path;

pub const MAX_IMPORTED_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_IMPORTED_IMAGE_DIMENSION: u32 = 16_384;
pub const MAX_IMPORTED_IMAGE_PIXELS: u64 = 40_000_000;
const MAX_DECODE_ALLOCATION_BYTES: u64 = MAX_IMPORTED_IMAGE_PIXELS * 4;
const IMPORT_MAGIC: &[u8; 8] = b"RMNIMPT\0";
const IMPORT_VERSION: u16 = 1;
pub(crate) const MAX_IMPORT_INTENT_BYTES: usize = 192;

/// A selected source that has been completely bounded, content-recognized,
/// decoded, and fingerprinted. Paths never survive this boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct PreparedImageAttachment {
    display_name: String,
    kind: AttachmentKind,
    width: u32,
    height: u32,
    sha256: [u8; 32],
    bytes: Vec<u8>,
}

impl fmt::Debug for PreparedImageAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedImageAttachment")
            .field("display_name", &"<redacted>")
            .field("kind", &self.kind)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("byte_len", &self.bytes.len())
            .field("sha256", &"<redacted>")
            .finish()
    }
}

impl PreparedImageAttachment {
    pub fn metadata(&self) -> NewAttachment {
        NewAttachment {
            display_name: self.display_name.clone(),
            kind: self.kind,
            byte_len: self.bytes.len() as u64,
            sha256: self.sha256,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn byte_len(&self) -> u64 {
        self.bytes.len() as u64
    }

    pub fn kind(&self) -> AttachmentKind {
        self.kind
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    fn matches(&self, plan: &AttachmentImportPlan) -> bool {
        self.kind == plan.kind && self.byte_len() == plan.byte_len && self.sha256 == plan.sha256
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportError {
    InvalidPlan,
    Malformed,
    Unsupported,
    TooLarge,
    Io(io::ErrorKind),
    ReadbackMismatch,
    AttachmentMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportAuthority {
    RolledBack,
    Accepted,
    AcceptedDescendant,
    Ambiguous,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ImportIntent {
    base_library_revision: u64,
    candidate_library_revision: u64,
    base_note_revision: u64,
    candidate_note_revision: u64,
    note_id: NoteId,
    attachment_id: AttachmentId,
    kind: AttachmentKind,
    byte_len: u64,
    base_sha256: [u8; 32],
    candidate_sha256: [u8; 32],
    attachment_sha256: [u8; 32],
}

impl fmt::Debug for ImportIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportIntent")
            .field("base_library_revision", &self.base_library_revision)
            .field(
                "candidate_library_revision",
                &self.candidate_library_revision,
            )
            .field("base_note_revision", &self.base_note_revision)
            .field("candidate_note_revision", &self.candidate_note_revision)
            .field("note_id", &self.note_id)
            .field("attachment_id", &self.attachment_id)
            .field("kind", &self.kind)
            .field("byte_len", &self.byte_len)
            .field("hashes", &"<redacted>")
            .finish()
    }
}

impl ImportIntent {
    pub(crate) fn prepare(
        base: &LibrarySnapshot,
        candidate: &LibrarySnapshot,
        plan: &AttachmentImportPlan,
        prepared: &PreparedImageAttachment,
    ) -> Result<Self, ImportError> {
        plan.validate_candidate(base, candidate)
            .map_err(|_| ImportError::InvalidPlan)?;
        if !prepared.matches(plan) {
            return Err(ImportError::InvalidPlan);
        }
        let base_bytes = encode(base).map_err(|_| ImportError::InvalidPlan)?;
        let candidate_bytes = encode(candidate).map_err(|_| ImportError::InvalidPlan)?;
        Ok(Self {
            base_library_revision: plan.base_library_revision,
            candidate_library_revision: plan.candidate_library_revision,
            base_note_revision: plan.base_note_revision,
            candidate_note_revision: plan.candidate_note_revision,
            note_id: plan.note_id,
            attachment_id: plan.attachment_id,
            kind: plan.kind,
            byte_len: plan.byte_len,
            base_sha256: digest(&base_bytes),
            candidate_sha256: digest(&candidate_bytes),
            attachment_sha256: plan.sha256,
        })
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, ImportError> {
        let mut bytes = Vec::with_capacity(MAX_IMPORT_INTENT_BYTES);
        extend(&mut bytes, IMPORT_MAGIC)?;
        extend(&mut bytes, &IMPORT_VERSION.to_le_bytes())?;
        for value in [
            self.base_library_revision,
            self.candidate_library_revision,
            self.base_note_revision,
            self.candidate_note_revision,
            self.note_id.get(),
            self.attachment_id.get(),
        ] {
            extend(&mut bytes, &value.to_le_bytes())?;
        }
        extend(&mut bytes, &[encode_kind(self.kind)])?;
        extend(&mut bytes, &self.byte_len.to_le_bytes())?;
        extend(&mut bytes, &self.base_sha256)?;
        extend(&mut bytes, &self.candidate_sha256)?;
        extend(&mut bytes, &self.attachment_sha256)?;
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ImportError> {
        if bytes.len() > MAX_IMPORT_INTENT_BYTES {
            return Err(ImportError::TooLarge);
        }
        let mut reader = Reader::new(bytes);
        if reader.take(IMPORT_MAGIC.len())? != IMPORT_MAGIC || reader.u16()? != IMPORT_VERSION {
            return Err(ImportError::Malformed);
        }
        let intent = Self {
            base_library_revision: reader.u64()?,
            candidate_library_revision: reader.u64()?,
            base_note_revision: reader.u64()?,
            candidate_note_revision: reader.u64()?,
            note_id: NoteId::new(reader.u64()?).ok_or(ImportError::Malformed)?,
            attachment_id: AttachmentId::new(reader.u64()?).ok_or(ImportError::Malformed)?,
            kind: decode_kind(reader.byte()?)?,
            byte_len: reader.u64()?,
            base_sha256: reader.array()?,
            candidate_sha256: reader.array()?,
            attachment_sha256: reader.array()?,
        };
        if !reader.finished()
            || intent.base_library_revision == 0
            || intent.candidate_library_revision
                != intent
                    .base_library_revision
                    .checked_add(1)
                    .ok_or(ImportError::Malformed)?
            || intent.base_note_revision == 0
            || intent.candidate_note_revision
                != intent
                    .base_note_revision
                    .checked_add(1)
                    .ok_or(ImportError::Malformed)?
            || intent.byte_len == 0
            || intent.byte_len > MAX_IMPORTED_IMAGE_BYTES as u64
            || intent.base_sha256 == [0; 32]
            || intent.candidate_sha256 == [0; 32]
            || intent.attachment_sha256 == [0; 32]
        {
            return Err(ImportError::Malformed);
        }
        Ok(intent)
    }

    pub(crate) fn authority(
        &self,
        snapshot: &LibrarySnapshot,
    ) -> Result<ImportAuthority, ImportError> {
        let encoded = encode(snapshot).map_err(|_| ImportError::Malformed)?;
        let sha256 = digest(&encoded);
        if snapshot.revision == self.base_library_revision && sha256 == self.base_sha256 {
            return Ok(ImportAuthority::RolledBack);
        }
        if snapshot.revision == self.candidate_library_revision && sha256 == self.candidate_sha256 {
            return Ok(ImportAuthority::Accepted);
        }
        if snapshot.revision > self.candidate_library_revision
            && snapshot.notes.iter().any(|note| {
                note.id == self.note_id
                    && note.revision >= self.candidate_note_revision
                    && note.attachments.contains(&self.attachment_id)
            })
            && snapshot.attachments.iter().any(|attachment| {
                attachment.id == self.attachment_id
                    && attachment.note_id == self.note_id
                    && attachment.kind == self.kind
                    && attachment.byte_len == self.byte_len
                    && attachment.sha256 == self.attachment_sha256
                    && !attachment.deleted
            })
        {
            return Ok(ImportAuthority::AcceptedDescendant);
        }
        Ok(ImportAuthority::Ambiguous)
    }

    pub(crate) fn stage<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
        prepared: &PreparedImageAttachment,
    ) -> Result<(), ImportError> {
        if !prepared.matches_plan_fields(self) {
            return Err(ImportError::InvalidPlan);
        }
        let path = managed_attachment_path(root, self.attachment_id);
        match fingerprint(backend, &path, self.byte_len) {
            Ok(existing) if self.matches_fingerprint(existing) => return Ok(()),
            Ok(_) => return Err(ImportError::AttachmentMismatch),
            Err(ImportError::Io(io::ErrorKind::NotFound)) => {}
            Err(error) => return Err(error),
        }
        let parent = path
            .parent()
            .ok_or(ImportError::Io(io::ErrorKind::InvalidInput))?;
        backend
            .create_dir_all_private(parent)
            .map_err(|error| ImportError::Io(error.kind()))?;
        match backend.write_new_private(&path, prepared.bytes()) {
            Ok(()) => self.verify_staged(root, backend),
            Err(write_error) => match fingerprint(backend, &path, self.byte_len) {
                Ok(existing) if self.matches_fingerprint(existing) => Ok(()),
                Ok(_) => Err(ImportError::AttachmentMismatch),
                Err(ImportError::Io(io::ErrorKind::NotFound)) => {
                    Err(ImportError::Io(write_error.kind()))
                }
                Err(_) => Err(ImportError::Io(write_error.kind())),
            },
        }
    }

    pub(crate) fn verify_staged<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
    ) -> Result<(), ImportError> {
        let path = managed_attachment_path(root, self.attachment_id);
        let fingerprint = fingerprint(backend, &path, self.byte_len)?;
        if self.matches_fingerprint(fingerprint) {
            Ok(())
        } else {
            Err(ImportError::ReadbackMismatch)
        }
    }

    pub(crate) fn rollback_staging<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
    ) -> Result<(), ImportError> {
        let path = managed_attachment_path(root, self.attachment_id);
        match fingerprint(backend, &path, self.byte_len) {
            Err(ImportError::Io(io::ErrorKind::NotFound)) => return Ok(()),
            Err(error) => return Err(error),
            Ok(fingerprint) if self.matches_fingerprint(fingerprint) => {}
            Ok(_) => return Err(ImportError::AttachmentMismatch),
        }
        match backend.remove_file_durable(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ImportError::Io(error.kind())),
        }
    }

    fn matches_fingerprint(&self, fingerprint: FileFingerprint) -> bool {
        fingerprint.byte_len == self.byte_len && fingerprint.sha256 == self.attachment_sha256
    }
}

impl PreparedImageAttachment {
    fn matches_plan_fields(&self, intent: &ImportIntent) -> bool {
        self.kind == intent.kind
            && self.byte_len() == intent.byte_len
            && self.sha256 == intent.attachment_sha256
    }
}

pub(crate) fn prepare_image<B: Backend>(
    backend: &B,
    selected_path: &Path,
) -> Result<PreparedImageAttachment, ImportError> {
    let bytes = backend
        .read_bounded(selected_path, MAX_IMPORTED_IMAGE_BYTES)
        .map_err(|error| match error.kind() {
            io::ErrorKind::InvalidData => ImportError::TooLarge,
            kind => ImportError::Io(kind),
        })?;
    if bytes.is_empty() {
        return Err(ImportError::Malformed);
    }
    let image_format = image::guess_format(&bytes).map_err(|_| ImportError::Unsupported)?;
    let (kind, image_format) = match image_format {
        image::ImageFormat::Png => (AttachmentKind::Png, image::ImageFormat::Png),
        image::ImageFormat::Jpeg => (AttachmentKind::Jpeg, image::ImageFormat::Jpeg),
        image::ImageFormat::WebP => (AttachmentKind::WebP, image::ImageFormat::WebP),
        _ => return Err(ImportError::Unsupported),
    };
    let mut reader = ImageReader::with_format(Cursor::new(bytes.as_slice()), image_format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMPORTED_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMPORTED_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOCATION_BYTES);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| ImportError::Malformed)?;
    let (width, height) = (decoded.width(), decoded.height());
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ImportError::TooLarge)?;
    if width == 0
        || height == 0
        || width > MAX_IMPORTED_IMAGE_DIMENSION
        || height > MAX_IMPORTED_IMAGE_DIMENSION
        || pixels > MAX_IMPORTED_IMAGE_PIXELS
    {
        return Err(ImportError::TooLarge);
    }
    drop(decoded);
    let display_name = canonical_display_name(selected_path.file_stem(), kind);
    let sha256 = digest(&bytes);
    Ok(PreparedImageAttachment {
        display_name,
        kind,
        width,
        height,
        sha256,
        bytes,
    })
}

fn canonical_display_name(stem: Option<&OsStr>, kind: AttachmentKind) -> String {
    let extension = match kind {
        AttachmentKind::Png => "png",
        AttachmentKind::Jpeg => "jpg",
        AttachmentKind::Gif => "gif",
        AttachmentKind::WebP => "webp",
    };
    let suffix = format!(".{extension}");
    let maximum_stem_bytes = MAX_NAME_BYTES.saturating_sub(suffix.len());
    let valid_stem = stem
        .and_then(OsStr::to_str)
        .map(str::trim)
        .filter(|stem| {
            !stem.is_empty()
                && !matches!(*stem, "." | "..")
                && !stem.chars().any(char::is_control)
                && !stem.contains('/')
                && !stem.contains('\\')
        })
        .unwrap_or("Image");
    let end = valid_stem
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(valid_stem.len()))
        .take_while(|index| *index <= maximum_stem_bytes)
        .last()
        .unwrap_or_default();
    let stem = &valid_stem[..end];
    let stem = if stem.is_empty() { "Image" } else { stem };
    format!("{stem}{suffix}")
}

fn encode_kind(kind: AttachmentKind) -> u8 {
    match kind {
        AttachmentKind::Png => 1,
        AttachmentKind::Jpeg => 2,
        AttachmentKind::Gif => 3,
        AttachmentKind::WebP => 4,
    }
}

fn decode_kind(value: u8) -> Result<AttachmentKind, ImportError> {
    match value {
        1 => Ok(AttachmentKind::Png),
        2 => Ok(AttachmentKind::Jpeg),
        4 => Ok(AttachmentKind::WebP),
        _ => Err(ImportError::Malformed),
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn fingerprint<B: Backend>(
    backend: &B,
    path: &Path,
    maximum: u64,
) -> Result<FileFingerprint, ImportError> {
    backend
        .fingerprint_bounded_no_follow(path, maximum)
        .map_err(|error| ImportError::Io(error.kind()))
}

fn extend(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), ImportError> {
    if bytes.len().saturating_add(value.len()) > MAX_IMPORT_INTENT_BYTES {
        return Err(ImportError::TooLarge);
    }
    bytes.extend_from_slice(value);
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ImportError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ImportError::Malformed)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(ImportError::Malformed)?;
        self.offset = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, ImportError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ImportError> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| ImportError::Malformed)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, ImportError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| ImportError::Malformed)?,
        ))
    }

    fn array(&mut self) -> Result<[u8; 32], ImportError> {
        self.take(32)?
            .try_into()
            .map_err(|_| ImportError::Malformed)
    }

    fn finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}
