//! Versioned, bounded domain records for the local rmac Notes library.
//!
//! This crate contains no UI, filesystem, portal, or async runtime. A storage
//! adapter can validate and encode a complete transaction candidate before it
//! changes durable state, and can decode records without lossy fallbacks or
//! unbounded allocation.

mod bundle_import;
mod codec;
mod export;
mod model;
mod mutation;
mod validation;

pub use bundle_import::{
    BundleAttachmentImport, BundleCollisionPolicy, BundleImportPlan, BundleImportReview,
    BundlePlanError, PlannedBundleImport,
};
pub use codec::{decode, encode, CodecError, MAX_LIBRARY_BYTES, SCHEMA_VERSION};
pub use export::{render_export_markdown, ExportAttachment, ExportError, ExportPlan, ExportScope};
pub use model::{
    AttachmentId, AttachmentKind, AttachmentRecord, FolderId, FolderRecord, LibrarySnapshot,
    NoteId, NoteRecord, SortOrder, MAX_ATTACHMENTS, MAX_ATTACHMENTS_PER_NOTE, MAX_ATTACHMENT_BYTES,
    MAX_BODY_BYTES, MAX_FOLDERS, MAX_NAME_BYTES, MAX_NOTES, MAX_TAGS_PER_NOTE, MAX_TAG_BYTES,
    MAX_TITLE_BYTES,
};
pub use mutation::{
    AttachmentImportPlan, LibraryTransaction, MutationError, NewAttachment, NewNote, NoteChanges,
    OrphanCollectionPlan, PurgePlan,
};
pub use validation::ValidationError;

#[cfg(test)]
mod tests;
