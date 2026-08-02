//! Durable single-writer transaction adapter for `rmac-notes-store`.
//!
//! Opening the real store first acquires one canonical, kernel-backed writer
//! lease for the library. A caller then retains the [`LoadedLibrary`] token
//! returned by `load`/`save`; every save performs an exact primary-file
//! preflight, writes and verifies a private journal, writes and verifies the
//! primary, refreshes last-known-good, then removes the journal. Startup
//! deterministically rolls back an uncommitted prepared journal or finishes
//! maintenance for a primary that already matches it.

mod attachment;
mod bundle_import;
mod drafts;
mod export;
mod journal;
mod legacy_scan;
mod markdown_preview;
mod migration;
mod model;
mod note_import;
mod orphan;
mod purge;
mod repository;
mod startup;
mod store;
mod writer;

pub use attachment::{
    load_managed_image_preview, DecodedImagePreview, PreparedImageAttachment, PreviewError,
    PreviewSize, MAX_IMPORTED_IMAGE_BYTES, MAX_IMPORTED_IMAGE_DIMENSION, MAX_IMPORTED_IMAGE_PIXELS,
    MAX_PREVIEW_DIMENSION, MAX_PREVIEW_PIXELS,
};
pub use bundle_import::{
    prepare_bundle_import, BundleImportError, BundleImportErrorKind, BundleImportOperation,
    PreparedBundleImport,
};

pub use drafts::{
    decode_draft, encode_draft, DraftCodecError, DraftDiscovery, DraftError, DraftErrorKind,
    DraftOperation, DraftRecord, DraftStore, MAX_DISCOVERED_DRAFTS, MAX_DRAFT_DISCOVERY_BYTES,
    MAX_DRAFT_RECORD_BYTES, MAX_SCANNED_DRAFT_ENTRIES,
};
pub use export::{
    ExportFailure, ExportFailureKind, ExportFormat, ExportOperation, ExportOutcome,
    PreparedExportDestination, MAX_EXPORT_BUNDLE_BYTES,
};

pub use legacy_scan::{
    scan_legacy_library, LegacyScanError, LegacyScanErrorKind, LegacyScanOperation,
};
pub use markdown_preview::{
    parse_inert_markdown_preview, MarkdownPreviewBlock, MarkdownPreviewBlockKind,
    MarkdownPreviewDocument, MarkdownPreviewError, MarkdownPreviewRun, MarkdownPreviewTextStyle,
    MAX_MARKDOWN_PREVIEW_BLOCKS, MAX_MARKDOWN_PREVIEW_DEPTH, MAX_MARKDOWN_PREVIEW_OUTPUT_BYTES,
    MAX_MARKDOWN_PREVIEW_RUNS,
};
pub use migration::{
    plan_legacy_library, LegacyAttachmentInput, LegacyLibraryInput, LegacyNoteInput,
    MigrationCommitError, MigrationCommitErrorKind, MigrationCommitOperation,
    MigrationCommitOutcome, MigrationError, MigrationPlan, MigrationWarning, PlannedAttachment,
    PlannedNoteSource, RecoveryFile,
};
pub use model::{ErrorKind, LoadedLibrary, Operation, RecoveryNotice, SaveOutcome, StoreError};
pub use note_import::{
    prepare_text_note, ImportedTextEncoding, MarkdownImportReview, PreparedTextNote,
    TextImportError, MAX_IMPORTED_TEXT_SOURCE_BYTES,
};
pub use repository::{AcceptedCommit, AcceptedLibrary, CommitError, PendingCommit, PendingReason};
pub use startup::{
    inspect_notes_startup, resolve_notes_paths, MigrationReview, NotesPathError, NotesPaths,
    NotesStartup, StartupError,
};
pub use writer::{WriterLease, WriterLeaseError, WriterLeaseErrorKind, WriterLeaseOperation};

pub use store::NotesLibraryStore;
pub(crate) use store::{has_blocking_notice, managed_attachment_path};

#[cfg(test)]
mod tests;
