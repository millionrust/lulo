use std::fmt;
use std::io;

use rmac_notes_store::{CodecError, LibrarySnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryNotice {
    RolledBackInterruptedSave,
    FinishedInterruptedSave,
    RecoveredLastKnownGood,
    CorruptJournalPreserved,
    MaintenancePending,
    RolledBackInterruptedPurge,
    FinishedInterruptedPurge,
    CorruptPurgePreserved,
    PurgeCleanupPending,
    RolledBackInterruptedAttachmentImport,
    FinishedInterruptedAttachmentImport,
    CorruptAttachmentImportPreserved,
    AttachmentImportPending,
    RolledBackInterruptedOrphanCollection,
    FinishedInterruptedOrphanCollection,
    CorruptOrphanCollectionPreserved,
    OrphanCollectionPending,
    RolledBackInterruptedBundleImport,
    FinishedInterruptedBundleImport,
    CorruptBundleImportPreserved,
    BundleImportPending,
}

#[derive(Clone, Debug)]
pub(crate) enum Baseline {
    Missing,
    Exact(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct LoadedLibrary {
    pub(crate) snapshot: LibrarySnapshot,
    pub(crate) baseline: Baseline,
    pub(crate) notices: Vec<RecoveryNotice>,
}

impl LoadedLibrary {
    pub fn snapshot(&self) -> &LibrarySnapshot {
        &self.snapshot
    }

    pub fn notices(&self) -> &[RecoveryNotice] {
        &self.notices
    }

    pub fn into_snapshot(self) -> LibrarySnapshot {
        self.snapshot
    }
}

#[derive(Clone, Debug)]
pub struct SaveOutcome {
    pub library: LoadedLibrary,
    pub maintenance_pending: bool,
    pub purge_cleanup_pending: bool,
    pub attachment_import_pending: bool,
    pub orphan_collection_pending: bool,
    pub bundle_import_pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    CreateDirectory,
    ReadPrimary,
    ReadLastKnownGood,
    ReadJournal,
    ParsePrimary,
    ParseLastKnownGood,
    ParseJournal,
    ValidateCandidate,
    PreflightPrimary,
    WriteJournal,
    VerifyJournal,
    WritePrimary,
    VerifyPrimary,
    WriteLastKnownGood,
    VerifyLastKnownGood,
    RemoveJournal,
    RecoverPrimary,
    ReadPurgeIntent,
    WritePurgeIntent,
    VerifyPurgeIntent,
    RemovePurgeIntent,
    VerifyPurgeAttachment,
    RemovePurgeAttachment,
    ReadAttachmentSource,
    DecodeAttachmentSource,
    ReadAttachmentImportIntent,
    WriteAttachmentImportIntent,
    VerifyAttachmentImportIntent,
    RemoveAttachmentImportIntent,
    StageManagedAttachment,
    VerifyManagedAttachment,
    RemoveManagedAttachment,
    ReadOrphanCollectionIntent,
    WriteOrphanCollectionIntent,
    VerifyOrphanCollectionIntent,
    RemoveOrphanCollectionIntent,
    VerifyOrphanAttachment,
    RemoveOrphanAttachment,
    ReadBundleImportIntent,
    WriteBundleImportIntent,
    VerifyBundleImportIntent,
    RemoveBundleImportIntent,
    StageBundleAttachment,
    VerifyBundleAttachment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Io(io::ErrorKind),
    Codec(CodecError),
    Conflict,
    InvalidRevision,
    ReadbackMismatch,
    AmbiguousJournal,
    InvalidPurge,
    InvalidAttachmentImport,
    InvalidOrphanCollection,
    InvalidBundleImport,
    UnsupportedAttachment,
    AttachmentTooLarge,
    AttachmentMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreError {
    pub operation: Operation,
    pub kind: ErrorKind,
}

impl StoreError {
    pub(crate) fn io(operation: Operation, error: io::Error) -> Self {
        Self {
            operation,
            kind: ErrorKind::Io(error.kind()),
        }
    }

    pub(crate) fn codec(operation: Operation, error: CodecError) -> Self {
        Self {
            operation,
            kind: ErrorKind::Codec(error),
        }
    }

    pub(crate) fn new(operation: Operation, kind: ErrorKind) -> Self {
        Self { operation, kind }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ErrorKind::Io(_) => "Notes could not complete a private storage operation",
            ErrorKind::Codec(_) => "Notes found invalid library data",
            ErrorKind::Conflict => "The Notes library changed before it could be saved",
            ErrorKind::InvalidRevision => "The Notes transaction revision is invalid",
            ErrorKind::ReadbackMismatch => "Notes could not verify the saved library data",
            ErrorKind::AmbiguousJournal => {
                "Notes found an interrupted transaction that needs recovery"
            }
            ErrorKind::InvalidPurge => "Notes found invalid permanent-deletion state",
            ErrorKind::InvalidAttachmentImport => {
                "Notes found invalid image-attachment import state"
            }
            ErrorKind::InvalidOrphanCollection => {
                "Notes found invalid orphan-attachment collection state"
            }
            ErrorKind::InvalidBundleImport => "Notes found invalid bundle-import recovery state",
            ErrorKind::UnsupportedAttachment => {
                "Notes supports PNG, JPEG, and WebP image attachments"
            }
            ErrorKind::AttachmentTooLarge => "The selected image exceeds a Notes safety limit",
            ErrorKind::AttachmentMismatch => {
                "Notes found managed attachment bytes that changed unexpectedly"
            }
        })
    }
}

impl std::error::Error for StoreError {}
