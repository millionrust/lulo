//! Versioned, bounded domain records for the local rmac Notes library.
//!
//! This crate contains no UI, filesystem, portal, or async runtime. A storage
//! adapter can validate and encode a complete transaction candidate before it
//! changes durable state, and can decode records without lossy fallbacks or
//! unbounded allocation. Locked notes are sealed here too (`lock`), so the
//! durable encoding can refuse to carry a locked note's plaintext.

mod bundle_import;
mod codec;
mod export;
mod lock;
mod model;
mod mutation;
mod validation;

pub use bundle_import::{
    BundleAttachmentImport, BundleCollisionPolicy, BundleImportPlan, BundleImportReview,
    BundlePlanError, PlannedBundleImport,
};
pub use codec::{decode, encode, CodecError, MAX_LIBRARY_BYTES, SCHEMA_VERSION};
pub use export::{render_export_markdown, ExportAttachment, ExportError, ExportPlan, ExportScope};
pub use lock::{
    create_lock_key, open_attachment, open_note, seal_attachment, seal_note, unlock_lock_key,
    LockError, LockKey, LockedNoteContent, DEFAULT_LOCK_KDF, LOCK_KEY_BYTES, LOCK_NONCE_BYTES,
    LOCK_SALT_BYTES, LOCK_TAG_BYTES, MAX_LOCK_KDF_ITERATIONS, MAX_LOCK_KDF_MEMORY_KIB,
    MAX_LOCK_KDF_PARALLELISM, MIN_LOCK_KDF_MEMORY_KIB, SEALED_ATTACHMENT_OVERHEAD,
};
pub use model::{
    AttachmentId, AttachmentKind, AttachmentRecord, FolderId, FolderRecord, LibrarySnapshot,
    LockKdfParams, LockKeyRecord, NoteId, NoteLock, NoteRecord, SealedBlob, SmartFolderId,
    SmartFolderRecord, SortOrder, MAX_ATTACHMENTS, MAX_ATTACHMENTS_PER_NOTE, MAX_ATTACHMENT_BYTES,
    MAX_BODY_BYTES, MAX_FOLDERS, MAX_LOCK_HINT_BYTES, MAX_LOCK_KEYS, MAX_NAME_BYTES, MAX_NOTES,
    MAX_SEALED_NOTE_BYTES, MAX_SMART_FOLDERS, MAX_TAGS_PER_NOTE, MAX_TAG_BYTES, MAX_TITLE_BYTES,
    SEALED_ATTACHMENT_NAME,
};
pub use mutation::{
    AttachmentImportPlan, LibraryTransaction, MutationError, NewAttachment, NewNote, NoteChanges,
    OrphanCollectionPlan, PurgePlan,
};
pub use validation::{is_valid_name, ValidationError};

#[cfg(test)]
pub(crate) use lock::TEST_LOCK_KDF;

#[cfg(test)]
mod tests;
