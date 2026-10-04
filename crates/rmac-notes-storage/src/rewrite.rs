//! Crash-safe replacement of managed attachment bytes.
//!
//! Locking a note seals its attachments; removing the lock or changing the
//! password rewrites them again. The new bytes are first staged next to the
//! managed file under a durable private intent, then the metadata candidate
//! naming their exact length and digest is committed, and only then are the
//! managed files replaced. Startup finishes or rolls back an interrupted
//! rewrite by comparing the durable library with the intent, so a sealed
//! record never outlives its plaintext file for longer than one recovery.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_notes_store::{encode, AttachmentId, LibrarySnapshot, MAX_ATTACHMENTS};
use rmac_storage::{Backend, FileFingerprint};
use sha2::{Digest as _, Sha256};

use crate::managed_attachment_path;

const REWRITE_MAGIC: &[u8; 8] = b"RMNRWRT\0";
const REWRITE_VERSION: u16 = 1;
const ENTRY_BYTES: usize = 8 + 8 + 32 + 8 + 32;
pub(crate) const MAX_REWRITE_INTENT_BYTES: usize =
    8 + 2 + 16 + 64 + 8 + MAX_ATTACHMENTS_PER_REWRITE * ENTRY_BYTES;
/// One rewrite covers one note's attachments, or every attachment sealed
/// under one password when it changes.
pub(crate) const MAX_ATTACHMENTS_PER_REWRITE: usize = MAX_ATTACHMENTS;

/// New bytes for one managed attachment, proven against the candidate record.
#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentRewrite {
    pub attachment_id: AttachmentId,
    pub bytes: Vec<u8>,
}

impl fmt::Debug for AttachmentRewrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttachmentRewrite")
            .field("attachment_id", &self.attachment_id)
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RewriteError {
    InvalidPlan,
    TooLarge,
    Malformed,
    Io(io::ErrorKind),
    ReadbackMismatch,
    AttachmentMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RewriteAuthority {
    RolledBack,
    Accepted,
    Ambiguous,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RewriteEntry {
    id: AttachmentId,
    old: FileFingerprint,
    new: FileFingerprint,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct RewriteIntent {
    base_library_revision: u64,
    candidate_library_revision: u64,
    base_sha256: [u8; 32],
    candidate_sha256: [u8; 32],
    entries: Vec<RewriteEntry>,
}

impl fmt::Debug for RewriteIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RewriteIntent")
            .field("base_library_revision", &self.base_library_revision)
            .field(
                "candidate_library_revision",
                &self.candidate_library_revision,
            )
            .field("attachment_count", &self.entries.len())
            .finish()
    }
}

pub(crate) fn staged_rewrite_path(root: &Path, id: AttachmentId) -> PathBuf {
    root.join("attachments")
        .join(format!("{:020}.rewrite", id.get()))
}

impl RewriteIntent {
    /// Prove that `candidate` differs from `base` in attachment bytes exactly
    /// where `rewrites` supplies new bytes with the candidate's length and
    /// digest.
    pub(crate) fn prepare(
        base: &LibrarySnapshot,
        candidate: &LibrarySnapshot,
        rewrites: &[AttachmentRewrite],
    ) -> Result<Self, RewriteError> {
        base.validate().map_err(|_| RewriteError::InvalidPlan)?;
        candidate
            .validate()
            .map_err(|_| RewriteError::InvalidPlan)?;
        if rewrites.is_empty()
            || rewrites.len() > MAX_ATTACHMENTS_PER_REWRITE
            || candidate.revision != base.revision.checked_add(1).unwrap_or(0)
            || !rewrites
                .windows(2)
                .all(|pair| pair[0].attachment_id < pair[1].attachment_id)
        {
            return Err(RewriteError::InvalidPlan);
        }
        let base_records = base
            .attachments
            .iter()
            .map(|record| (record.id, record))
            .collect::<BTreeMap<_, _>>();
        let mut entries = Vec::with_capacity(rewrites.len());
        for rewrite in rewrites {
            let old = base_records
                .get(&rewrite.attachment_id)
                .ok_or(RewriteError::InvalidPlan)?;
            let new = candidate
                .attachments
                .iter()
                .find(|record| record.id == rewrite.attachment_id)
                .ok_or(RewriteError::InvalidPlan)?;
            let fingerprint = FileFingerprint {
                byte_len: rewrite.bytes.len() as u64,
                sha256: digest(&rewrite.bytes),
            };
            if new.byte_len != fingerprint.byte_len || new.sha256 != fingerprint.sha256 {
                return Err(RewriteError::InvalidPlan);
            }
            entries.push(RewriteEntry {
                id: rewrite.attachment_id,
                old: FileFingerprint {
                    byte_len: old.byte_len,
                    sha256: old.sha256,
                },
                new: fingerprint,
            });
        }
        // No other attachment may change its bytes description silently.
        for record in &candidate.attachments {
            let Some(old) = base_records.get(&record.id) else {
                return Err(RewriteError::InvalidPlan);
            };
            let rewritten = rewrites
                .binary_search_by_key(&record.id, |rewrite| rewrite.attachment_id)
                .is_ok();
            if !rewritten && (old.byte_len != record.byte_len || old.sha256 != record.sha256) {
                return Err(RewriteError::InvalidPlan);
            }
        }
        let base_bytes = encode(base).map_err(|_| RewriteError::InvalidPlan)?;
        let candidate_bytes = encode(candidate).map_err(|_| RewriteError::InvalidPlan)?;
        Ok(Self {
            base_library_revision: base.revision,
            candidate_library_revision: candidate.revision,
            base_sha256: digest(&base_bytes),
            candidate_sha256: digest(&candidate_bytes),
            entries,
        })
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, RewriteError> {
        if self.entries.is_empty() || self.entries.len() > MAX_ATTACHMENTS_PER_REWRITE {
            return Err(RewriteError::InvalidPlan);
        }
        let mut bytes = Vec::with_capacity(8 + 2 + 16 + 64 + 8 + self.entries.len() * ENTRY_BYTES);
        bytes.extend_from_slice(REWRITE_MAGIC);
        bytes.extend_from_slice(&REWRITE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.base_library_revision.to_le_bytes());
        bytes.extend_from_slice(&self.candidate_library_revision.to_le_bytes());
        bytes.extend_from_slice(&self.base_sha256);
        bytes.extend_from_slice(&self.candidate_sha256);
        bytes.extend_from_slice(&(self.entries.len() as u64).to_le_bytes());
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.id.get().to_le_bytes());
            bytes.extend_from_slice(&entry.old.byte_len.to_le_bytes());
            bytes.extend_from_slice(&entry.old.sha256);
            bytes.extend_from_slice(&entry.new.byte_len.to_le_bytes());
            bytes.extend_from_slice(&entry.new.sha256);
        }
        if bytes.len() > MAX_REWRITE_INTENT_BYTES {
            return Err(RewriteError::TooLarge);
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, RewriteError> {
        if bytes.len() > MAX_REWRITE_INTENT_BYTES {
            return Err(RewriteError::TooLarge);
        }
        let mut reader = Reader { bytes, cursor: 0 };
        if reader.take(8)? != REWRITE_MAGIC || reader.u16()? != REWRITE_VERSION {
            return Err(RewriteError::Malformed);
        }
        let base_library_revision = reader.u64()?;
        let candidate_library_revision = reader.u64()?;
        let base_sha256 = reader.array()?;
        let candidate_sha256 = reader.array()?;
        let count = usize::try_from(reader.u64()?).map_err(|_| RewriteError::Malformed)?;
        if count == 0 || count > MAX_ATTACHMENTS_PER_REWRITE {
            return Err(RewriteError::Malformed);
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let id = AttachmentId::new(reader.u64()?).ok_or(RewriteError::Malformed)?;
            let old = FileFingerprint {
                byte_len: reader.u64()?,
                sha256: reader.array()?,
            };
            let new = FileFingerprint {
                byte_len: reader.u64()?,
                sha256: reader.array()?,
            };
            entries.push(RewriteEntry { id, old, new });
        }
        if reader.cursor != bytes.len()
            || base_library_revision == 0
            || candidate_library_revision != base_library_revision.checked_add(1).unwrap_or(0)
            || !entries.windows(2).all(|pair| pair[0].id < pair[1].id)
        {
            return Err(RewriteError::Malformed);
        }
        Ok(Self {
            base_library_revision,
            candidate_library_revision,
            base_sha256,
            candidate_sha256,
            entries,
        })
    }

    pub(crate) fn authority(&self, snapshot: &LibrarySnapshot) -> RewriteAuthority {
        let Ok(encoded) = encode(snapshot) else {
            return RewriteAuthority::Ambiguous;
        };
        let hash = digest(&encoded);
        if snapshot.revision == self.base_library_revision && hash == self.base_sha256 {
            RewriteAuthority::RolledBack
        } else if snapshot.revision == self.candidate_library_revision
            && hash == self.candidate_sha256
        {
            RewriteAuthority::Accepted
        } else {
            RewriteAuthority::Ambiguous
        }
    }

    /// Write every staged file, verifying each by readback.
    pub(crate) fn stage<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
        rewrites: &[AttachmentRewrite],
    ) -> Result<(), RewriteError> {
        if rewrites.len() != self.entries.len() {
            return Err(RewriteError::InvalidPlan);
        }
        let attachments = root.join("attachments");
        backend
            .create_dir_all_private(&attachments)
            .map_err(|error| RewriteError::Io(error.kind()))?;
        for (entry, rewrite) in self.entries.iter().zip(rewrites) {
            if entry.id != rewrite.attachment_id {
                return Err(RewriteError::InvalidPlan);
            }
            let path = staged_rewrite_path(root, entry.id);
            backend
                .write_atomic_private(&path, &rewrite.bytes)
                .map_err(|error| RewriteError::Io(error.kind()))?;
            if fingerprint(backend, &path, entry.new.byte_len)? != entry.new {
                return Err(RewriteError::ReadbackMismatch);
            }
        }
        Ok(())
    }

    /// After the candidate is durable: replace each managed file with its
    /// staged bytes, then remove the staged copy.
    pub(crate) fn finish<B: Backend>(&self, root: &Path, backend: &B) -> Result<(), RewriteError> {
        for entry in &self.entries {
            let managed = managed_attachment_path(root, entry.id);
            let staged = staged_rewrite_path(root, entry.id);
            let already = match fingerprint(
                backend,
                &managed,
                entry.new.byte_len.max(entry.old.byte_len),
            ) {
                Ok(current) => current == entry.new,
                Err(RewriteError::Io(io::ErrorKind::NotFound)) => false,
                Err(error) => return Err(error),
            };
            if !already {
                let maximum =
                    usize::try_from(entry.new.byte_len).map_err(|_| RewriteError::TooLarge)?;
                let bytes = backend
                    .read_bounded_no_follow(&staged, maximum)
                    .map_err(|error| RewriteError::Io(error.kind()))?;
                if bytes.len() as u64 != entry.new.byte_len || digest(&bytes) != entry.new.sha256 {
                    return Err(RewriteError::AttachmentMismatch);
                }
                backend
                    .write_atomic_private(&managed, &bytes)
                    .map_err(|error| RewriteError::Io(error.kind()))?;
                if fingerprint(backend, &managed, entry.new.byte_len)? != entry.new {
                    return Err(RewriteError::ReadbackMismatch);
                }
            }
            remove_if_present(backend, &staged)?;
        }
        Ok(())
    }

    /// The candidate never became durable: discard every staged copy.
    pub(crate) fn rollback<B: Backend>(
        &self,
        root: &Path,
        backend: &B,
    ) -> Result<(), RewriteError> {
        for entry in &self.entries {
            remove_if_present(backend, &staged_rewrite_path(root, entry.id))?;
        }
        Ok(())
    }
}

fn remove_if_present<B: Backend>(backend: &B, path: &Path) -> Result<(), RewriteError> {
    match backend.remove_file_durable(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RewriteError::Io(error.kind())),
    }
}

fn fingerprint<B: Backend>(
    backend: &B,
    path: &Path,
    maximum: u64,
) -> Result<FileFingerprint, RewriteError> {
    backend
        .fingerprint_bounded_no_follow(path, maximum)
        .map_err(|error| RewriteError::Io(error.kind()))
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], RewriteError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or(RewriteError::Malformed)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(RewriteError::Malformed)?;
        self.cursor = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, RewriteError> {
        self.take(2)?
            .try_into()
            .map(u16::from_le_bytes)
            .map_err(|_| RewriteError::Malformed)
    }

    fn u64(&mut self) -> Result<u64, RewriteError> {
        self.take(8)?
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| RewriteError::Malformed)
    }

    fn array(&mut self) -> Result<[u8; 32], RewriteError> {
        self.take(32)?
            .try_into()
            .map_err(|_| RewriteError::Malformed)
    }
}
