//! Durable single-writer transaction adapter for `rmac-notes-store`.
//!
//! Opening the real store first acquires one canonical, kernel-backed writer
//! lease for the library. A caller then retains the [`LoadedLibrary`] token
//! returned by `load`/`save`; every save performs an exact primary-file
//! preflight, writes and verifies a private journal, writes and verifies the
//! primary, refreshes last-known-good, then removes the journal. Startup
//! deterministically rolls back an uncommitted prepared journal or finishes
//! maintenance for a primary that already matches it.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rmac_notes_store::{
    decode, encode, AttachmentImportPlan, CodecError, LibrarySnapshot, PurgePlan, MAX_LIBRARY_BYTES,
};
use rmac_storage::{Backend, FileSystem};
use sha2::{Digest as _, Sha256};

mod attachment;
mod drafts;
mod legacy_scan;
mod migration;
mod purge;
mod repository;
mod startup;
mod writer;

use attachment::{ImportAuthority, ImportError, ImportIntent, MAX_IMPORT_INTENT_BYTES};
use purge::{PurgeAuthority, PurgeError, PurgeIntent, MAX_PURGE_INTENT_BYTES};

pub use attachment::{
    PreparedImageAttachment, MAX_IMPORTED_IMAGE_BYTES, MAX_IMPORTED_IMAGE_DIMENSION,
    MAX_IMPORTED_IMAGE_PIXELS,
};

pub use drafts::{
    decode_draft, encode_draft, DraftCodecError, DraftDiscovery, DraftError, DraftErrorKind,
    DraftOperation, DraftRecord, DraftStore, MAX_DISCOVERED_DRAFTS, MAX_DRAFT_DISCOVERY_BYTES,
    MAX_DRAFT_RECORD_BYTES, MAX_SCANNED_DRAFT_ENTRIES,
};

pub use legacy_scan::{
    scan_legacy_library, LegacyScanError, LegacyScanErrorKind, LegacyScanOperation,
};
pub use migration::{
    plan_legacy_library, LegacyAttachmentInput, LegacyLibraryInput, LegacyNoteInput,
    MigrationCommitError, MigrationCommitErrorKind, MigrationCommitOperation,
    MigrationCommitOutcome, MigrationError, MigrationPlan, MigrationWarning, PlannedAttachment,
    PlannedNoteSource, RecoveryFile,
};
pub use repository::{AcceptedCommit, AcceptedLibrary, CommitError, PendingCommit, PendingReason};
pub use startup::{
    inspect_notes_startup, resolve_notes_paths, MigrationReview, NotesPathError, NotesPaths,
    NotesStartup, StartupError,
};
pub use writer::{WriterLease, WriterLeaseError, WriterLeaseErrorKind, WriterLeaseOperation};

const JOURNAL_MAGIC: &[u8; 8] = b"RMNJRN\0\0";
const JOURNAL_VERSION: u16 = 1;
const MAX_JOURNAL_BYTES: usize = MAX_LIBRARY_BYTES + 128;

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
}

#[derive(Clone, Debug)]
enum Baseline {
    Missing,
    Exact(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct LoadedLibrary {
    snapshot: LibrarySnapshot,
    baseline: Baseline,
    notices: Vec<RecoveryNotice>,
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
    fn io(operation: Operation, error: io::Error) -> Self {
        Self {
            operation,
            kind: ErrorKind::Io(error.kind()),
        }
    }

    fn codec(operation: Operation, error: CodecError) -> Self {
        Self {
            operation,
            kind: ErrorKind::Codec(error),
        }
    }

    fn new(operation: Operation, kind: ErrorKind) -> Self {
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

pub struct NotesLibraryStore<B = FileSystem> {
    root: PathBuf,
    backend: B,
    _writer_lease: WriterLease,
    transaction_lock: Mutex<()>,
}

impl NotesLibraryStore<FileSystem> {
    pub fn new(root: PathBuf) -> Result<Self, WriterLeaseError> {
        let writer_lease = WriterLease::acquire(&root)?;
        Ok(Self::with_backend(FileSystem, writer_lease))
    }
}

impl<B: Backend> NotesLibraryStore<B> {
    pub fn with_backend(backend: B, writer_lease: WriterLease) -> Self {
        let root = writer_lease.root().to_path_buf();
        Self {
            root,
            backend,
            _writer_lease: writer_lease,
            transaction_lock: Mutex::new(()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Completely read, content-recognize, and decode one portal-selected
    /// image. The returned value retains bytes, not the untrusted source path.
    pub fn prepare_image_attachment(
        &self,
        selected_path: &Path,
    ) -> Result<PreparedImageAttachment, StoreError> {
        attachment::prepare_image(&self.backend, selected_path).map_err(|error| {
            map_import_error(
                match error {
                    ImportError::Io(_) | ImportError::TooLarge => Operation::ReadAttachmentSource,
                    _ => Operation::DecodeAttachmentSource,
                },
                error,
            )
        })
    }

    pub fn load(&self) -> Result<LoadedLibrary, StoreError> {
        let _guard = self
            .transaction_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.load_locked()
    }

    pub fn save(
        &self,
        loaded: &LoadedLibrary,
        candidate: &LibrarySnapshot,
    ) -> Result<SaveOutcome, StoreError> {
        let _guard = self
            .transaction_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.require_no_purge_intent()?;
        self.require_no_import_intent()?;
        self.save_locked(loaded, candidate)
    }

    /// Stage one exact decoded image under its fresh managed identity, then
    /// publish only the metadata candidate bound by `plan`.
    pub fn save_attachment_import(
        &self,
        loaded: &LoadedLibrary,
        candidate: &LibrarySnapshot,
        plan: &AttachmentImportPlan,
        prepared: &PreparedImageAttachment,
    ) -> Result<SaveOutcome, StoreError> {
        let _guard = self
            .transaction_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if has_blocking_notice(loaded.notices()) {
            return Err(StoreError::new(
                Operation::PreflightPrimary,
                ErrorKind::AmbiguousJournal,
            ));
        }
        self.require_no_purge_intent()?;
        self.require_no_import_intent()?;
        let intent = ImportIntent::prepare(loaded.snapshot(), candidate, plan, prepared)
            .map_err(|error| map_import_error(Operation::WriteAttachmentImportIntent, error))?;
        self.write_import_intent(&intent)?;
        intent
            .stage(&self.root, &self.backend, prepared)
            .map_err(|error| map_import_error(Operation::StageManagedAttachment, error))?;
        let mut outcome = self.save_locked(loaded, candidate)?;
        if outcome.maintenance_pending {
            outcome.attachment_import_pending = true;
            push_notice(
                &mut outcome.library.notices,
                RecoveryNotice::AttachmentImportPending,
            );
            return Ok(outcome);
        }
        if self.remove_import_intent().is_err() {
            outcome.maintenance_pending = true;
            outcome.attachment_import_pending = true;
            push_notice(
                &mut outcome.library.notices,
                RecoveryNotice::AttachmentImportPending,
            );
        }
        Ok(outcome)
    }

    /// Publish an exact purge candidate, then collect only the managed
    /// attachment bytes named by its durable private intent.
    pub fn save_purge(
        &self,
        loaded: &LoadedLibrary,
        candidate: &LibrarySnapshot,
        plan: &PurgePlan,
    ) -> Result<SaveOutcome, StoreError> {
        let _guard = self
            .transaction_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if has_blocking_notice(loaded.notices()) {
            return Err(StoreError::new(
                Operation::PreflightPrimary,
                ErrorKind::AmbiguousJournal,
            ));
        }
        self.require_no_purge_intent()?;
        self.require_no_import_intent()?;
        let intent = PurgeIntent::prepare(loaded.snapshot(), candidate, plan)
            .map_err(|error| map_purge_error(Operation::WritePurgeIntent, error))?;
        self.write_purge_intent(&intent)?;
        let mut outcome = self.save_locked(loaded, candidate)?;
        if outcome.maintenance_pending {
            outcome.purge_cleanup_pending = true;
            push_notice(
                &mut outcome.library.notices,
                RecoveryNotice::PurgeCleanupPending,
            );
            return Ok(outcome);
        }
        if self.finish_purge(&intent).is_err() {
            outcome.maintenance_pending = true;
            outcome.purge_cleanup_pending = true;
            push_notice(
                &mut outcome.library.notices,
                RecoveryNotice::PurgeCleanupPending,
            );
        }
        Ok(outcome)
    }

    /// Preserve a freshly reread legacy library and publish its reviewed plan.
    /// Raw recovery data and managed attachments are exact-readback verified
    /// before the metadata transaction becomes authoritative.
    pub fn commit_legacy_migration(
        &self,
        loaded: &LoadedLibrary,
        reread: &LegacyLibraryInput,
        reviewed_plan: &MigrationPlan,
    ) -> Result<MigrationCommitOutcome, MigrationCommitError> {
        let _guard = self
            .transaction_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        migration::commit_legacy_migration_locked(self, loaded, reread, reviewed_plan)
    }

    fn load_locked(&self) -> Result<LoadedLibrary, StoreError> {
        let notices = self.recover_journal()?;
        let loaded = match self.read_snapshot(
            &self.primary_path(),
            Operation::ReadPrimary,
            Operation::ParsePrimary,
        ) {
            Ok(Some((snapshot, bytes))) => LoadedLibrary {
                snapshot,
                baseline: Baseline::Exact(bytes),
                notices,
            },
            Ok(None) => match self.read_snapshot(
                &self.last_good_path(),
                Operation::ReadLastKnownGood,
                Operation::ParseLastKnownGood,
            )? {
                Some((snapshot, bytes)) => {
                    self.restore_primary(&bytes)?;
                    let mut notices = notices;
                    notices.push(RecoveryNotice::RecoveredLastKnownGood);
                    LoadedLibrary {
                        snapshot,
                        baseline: Baseline::Exact(bytes),
                        notices,
                    }
                }
                None => LoadedLibrary {
                    snapshot: LibrarySnapshot::default(),
                    baseline: Baseline::Missing,
                    notices,
                },
            },
            Err(primary_error) => match self.read_snapshot(
                &self.last_good_path(),
                Operation::ReadLastKnownGood,
                Operation::ParseLastKnownGood,
            ) {
                Ok(Some((snapshot, bytes))) => {
                    self.restore_primary(&bytes)?;
                    let mut notices = notices;
                    notices.push(RecoveryNotice::RecoveredLastKnownGood);
                    LoadedLibrary {
                        snapshot,
                        baseline: Baseline::Exact(bytes),
                        notices,
                    }
                }
                _ => return Err(primary_error),
            },
        };
        let purge_present = self.intent_present(&self.purge_path(), MAX_PURGE_INTENT_BYTES);
        let import_present = self.intent_present(&self.import_path(), MAX_IMPORT_INTENT_BYTES);
        if purge_present && import_present {
            let mut loaded = loaded;
            push_notice(&mut loaded.notices, RecoveryNotice::CorruptPurgePreserved);
            push_notice(
                &mut loaded.notices,
                RecoveryNotice::CorruptAttachmentImportPreserved,
            );
            return Ok(loaded);
        }
        let loaded = self.recover_purge(loaded)?;
        self.recover_attachment_import(loaded)
    }

    fn save_locked(
        &self,
        loaded: &LoadedLibrary,
        candidate: &LibrarySnapshot,
    ) -> Result<SaveOutcome, StoreError> {
        if has_blocking_notice(&loaded.notices) {
            return Err(StoreError::new(
                Operation::PreflightPrimary,
                ErrorKind::AmbiguousJournal,
            ));
        }
        let expected_revision = loaded.snapshot.revision.checked_add(1).ok_or_else(|| {
            StoreError::new(Operation::ValidateCandidate, ErrorKind::InvalidRevision)
        })?;
        if candidate.revision != expected_revision {
            return Err(StoreError::new(
                Operation::ValidateCandidate,
                ErrorKind::InvalidRevision,
            ));
        }
        let candidate_bytes = encode(candidate)
            .map_err(|error| StoreError::codec(Operation::ValidateCandidate, error))?;
        self.preflight(&loaded.baseline)?;
        self.require_no_journal()?;
        self.backend
            .create_dir_all(&self.root)
            .map_err(|error| StoreError::io(Operation::CreateDirectory, error))?;

        let journal = Journal::new(&loaded.baseline, candidate_bytes.clone());
        let journal_bytes = journal.encode()?;
        if let Err(error) = self.write_and_verify_raw(
            &self.journal_path(),
            &journal_bytes,
            MAX_JOURNAL_BYTES,
            Operation::WriteJournal,
            Operation::VerifyJournal,
        ) {
            let _ = self.remove_journal();
            return Err(error);
        }
        Journal::decode(&journal_bytes)?;

        // Replacement can succeed before a later directory-sync error is
        // reported. Preserve the journal on every error so startup can compare
        // the exact previous/candidate primary and recover deterministically.
        self.write_and_verify_snapshot(
            &self.primary_path(),
            &candidate_bytes,
            Operation::WritePrimary,
            Operation::VerifyPrimary,
        )?;

        let mut maintenance_pending = false;
        if self
            .write_and_verify_snapshot(
                &self.last_good_path(),
                &candidate_bytes,
                Operation::WriteLastKnownGood,
                Operation::VerifyLastKnownGood,
            )
            .is_err()
        {
            maintenance_pending = true;
        }
        if !maintenance_pending && self.remove_journal().is_err() {
            maintenance_pending = true;
        }
        let notices = maintenance_pending
            .then_some(RecoveryNotice::MaintenancePending)
            .into_iter()
            .collect();
        Ok(SaveOutcome {
            library: LoadedLibrary {
                snapshot: candidate.clone(),
                baseline: Baseline::Exact(candidate_bytes),
                notices,
            },
            maintenance_pending,
            purge_cleanup_pending: false,
            attachment_import_pending: false,
        })
    }

    fn recover_purge(&self, mut loaded: LoadedLibrary) -> Result<LoadedLibrary, StoreError> {
        let bytes = match self
            .backend
            .read_bounded_no_follow(&self.purge_path(), MAX_PURGE_INTENT_BYTES)
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(loaded),
            Err(_) => {
                push_notice(&mut loaded.notices, RecoveryNotice::CorruptPurgePreserved);
                return Ok(loaded);
            }
        };
        let intent = match PurgeIntent::decode(&bytes) {
            Ok(intent) => intent,
            Err(_) => {
                push_notice(&mut loaded.notices, RecoveryNotice::CorruptPurgePreserved);
                return Ok(loaded);
            }
        };
        match intent
            .authority(&loaded.snapshot)
            .map_err(|error| map_purge_error(Operation::ReadPurgeIntent, error))?
        {
            PurgeAuthority::RolledBack => {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::RolledBackInterruptedPurge,
                );
                if self.remove_purge_intent().is_err() {
                    push_notice(&mut loaded.notices, RecoveryNotice::PurgeCleanupPending);
                }
            }
            PurgeAuthority::Accepted | PurgeAuthority::AcceptedDescendant => {
                if self.finish_purge(&intent).is_ok() {
                    push_notice(
                        &mut loaded.notices,
                        RecoveryNotice::FinishedInterruptedPurge,
                    );
                } else {
                    push_notice(&mut loaded.notices, RecoveryNotice::PurgeCleanupPending);
                }
            }
            PurgeAuthority::Ambiguous => {
                push_notice(&mut loaded.notices, RecoveryNotice::CorruptPurgePreserved)
            }
        }
        Ok(loaded)
    }

    fn recover_attachment_import(
        &self,
        mut loaded: LoadedLibrary,
    ) -> Result<LoadedLibrary, StoreError> {
        let bytes = match self
            .backend
            .read_bounded_no_follow(&self.import_path(), MAX_IMPORT_INTENT_BYTES)
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(loaded),
            Err(_) => {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptAttachmentImportPreserved,
                );
                return Ok(loaded);
            }
        };
        let intent = match ImportIntent::decode(&bytes) {
            Ok(intent) => intent,
            Err(_) => {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptAttachmentImportPreserved,
                );
                return Ok(loaded);
            }
        };
        match intent
            .authority(&loaded.snapshot)
            .map_err(|error| map_import_error(Operation::ReadAttachmentImportIntent, error))?
        {
            ImportAuthority::RolledBack => {
                if intent
                    .rollback_staging(&self.root, &self.backend)
                    .and_then(|()| self.remove_import_intent().map_err(import_error_from_store))
                    .is_ok()
                {
                    push_notice(
                        &mut loaded.notices,
                        RecoveryNotice::RolledBackInterruptedAttachmentImport,
                    );
                } else {
                    push_notice(&mut loaded.notices, RecoveryNotice::AttachmentImportPending);
                }
            }
            ImportAuthority::Accepted | ImportAuthority::AcceptedDescendant => {
                if intent
                    .verify_staged(&self.root, &self.backend)
                    .and_then(|()| self.remove_import_intent().map_err(import_error_from_store))
                    .is_ok()
                {
                    push_notice(
                        &mut loaded.notices,
                        RecoveryNotice::FinishedInterruptedAttachmentImport,
                    );
                } else {
                    push_notice(&mut loaded.notices, RecoveryNotice::AttachmentImportPending);
                }
            }
            ImportAuthority::Ambiguous => push_notice(
                &mut loaded.notices,
                RecoveryNotice::CorruptAttachmentImportPreserved,
            ),
        }
        Ok(loaded)
    }

    fn write_import_intent(&self, intent: &ImportIntent) -> Result<(), StoreError> {
        let bytes = intent
            .encode()
            .map_err(|error| map_import_error(Operation::WriteAttachmentImportIntent, error))?;
        self.backend
            .create_dir_all_private(&self.root)
            .map_err(|error| StoreError::io(Operation::CreateDirectory, error))?;
        self.backend
            .write_atomic_private(&self.import_path(), &bytes)
            .map_err(|error| StoreError::io(Operation::WriteAttachmentImportIntent, error))?;
        let readback = self
            .backend
            .read_bounded_no_follow(&self.import_path(), MAX_IMPORT_INTENT_BYTES)
            .map_err(|error| StoreError::io(Operation::VerifyAttachmentImportIntent, error))?;
        if readback != bytes || ImportIntent::decode(&readback).ok().as_ref() != Some(intent) {
            return Err(StoreError::new(
                Operation::VerifyAttachmentImportIntent,
                ErrorKind::ReadbackMismatch,
            ));
        }
        Ok(())
    }

    fn require_no_import_intent(&self) -> Result<(), StoreError> {
        match self
            .backend
            .read_bounded_no_follow(&self.import_path(), MAX_IMPORT_INTENT_BYTES)
        {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(StoreError::new(
                Operation::ReadAttachmentImportIntent,
                ErrorKind::InvalidAttachmentImport,
            )),
            Err(error) => Err(StoreError::io(Operation::ReadAttachmentImportIntent, error)),
        }
    }

    fn remove_import_intent(&self) -> Result<(), StoreError> {
        match self.backend.remove_file_durable(&self.import_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::io(
                Operation::RemoveAttachmentImportIntent,
                error,
            )),
        }
    }

    fn intent_present(&self, path: &Path, maximum: usize) -> bool {
        match self.backend.read_bounded_no_follow(path, maximum) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Ok(_) | Err(_) => true,
        }
    }

    fn write_purge_intent(&self, intent: &PurgeIntent) -> Result<(), StoreError> {
        let bytes = intent
            .encode()
            .map_err(|error| map_purge_error(Operation::WritePurgeIntent, error))?;
        self.backend
            .create_dir_all_private(&self.root)
            .map_err(|error| StoreError::io(Operation::CreateDirectory, error))?;
        self.backend
            .write_atomic_private(&self.purge_path(), &bytes)
            .map_err(|error| StoreError::io(Operation::WritePurgeIntent, error))?;
        let readback = self
            .backend
            .read_bounded_no_follow(&self.purge_path(), MAX_PURGE_INTENT_BYTES)
            .map_err(|error| StoreError::io(Operation::VerifyPurgeIntent, error))?;
        if readback != bytes || PurgeIntent::decode(&readback).ok().as_ref() != Some(intent) {
            return Err(StoreError::new(
                Operation::VerifyPurgeIntent,
                ErrorKind::ReadbackMismatch,
            ));
        }
        Ok(())
    }

    fn finish_purge(&self, intent: &PurgeIntent) -> Result<(), StoreError> {
        intent
            .cleanup(&self.root, &self.backend)
            .map_err(|error| map_purge_error(Operation::VerifyPurgeAttachment, error))?;
        self.remove_purge_intent()
    }

    fn require_no_purge_intent(&self) -> Result<(), StoreError> {
        match self
            .backend
            .read_bounded_no_follow(&self.purge_path(), MAX_PURGE_INTENT_BYTES)
        {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(StoreError::new(
                Operation::ReadPurgeIntent,
                ErrorKind::InvalidPurge,
            )),
            Err(error) => Err(StoreError::io(Operation::ReadPurgeIntent, error)),
        }
    }

    fn remove_purge_intent(&self) -> Result<(), StoreError> {
        match self.backend.remove_file_durable(&self.purge_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::io(Operation::RemovePurgeIntent, error)),
        }
    }

    fn recover_journal(&self) -> Result<Vec<RecoveryNotice>, StoreError> {
        let journal_bytes = match self
            .backend
            .read_bounded(&self.journal_path(), MAX_JOURNAL_BYTES)
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(StoreError::io(Operation::ReadJournal, error)),
        };
        let journal = match Journal::decode(&journal_bytes) {
            Ok(journal) => journal,
            Err(_) => return Ok(vec![RecoveryNotice::CorruptJournalPreserved]),
        };
        let current = match self
            .backend
            .read_bounded(&self.primary_path(), MAX_LIBRARY_BYTES)
        {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(StoreError::io(Operation::ReadPrimary, error)),
        };

        if current.as_deref() == Some(journal.candidate.as_slice()) {
            let mut notices = vec![RecoveryNotice::FinishedInterruptedSave];
            let backup_ok = self
                .write_and_verify_snapshot(
                    &self.last_good_path(),
                    &journal.candidate,
                    Operation::WriteLastKnownGood,
                    Operation::VerifyLastKnownGood,
                )
                .is_ok();
            let cleanup_ok = backup_ok && self.remove_journal().is_ok();
            if !cleanup_ok {
                notices.push(RecoveryNotice::MaintenancePending);
            }
            return Ok(notices);
        }

        if journal.matches_expected(current.as_deref()) {
            let mut notices = vec![RecoveryNotice::RolledBackInterruptedSave];
            if self.remove_journal().is_err() {
                notices.push(RecoveryNotice::MaintenancePending);
            }
            return Ok(notices);
        }
        Err(StoreError::new(
            Operation::ParseJournal,
            ErrorKind::AmbiguousJournal,
        ))
    }

    fn preflight(&self, baseline: &Baseline) -> Result<(), StoreError> {
        let current = match self
            .backend
            .read_bounded(&self.primary_path(), MAX_LIBRARY_BYTES)
        {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(StoreError::io(Operation::PreflightPrimary, error)),
        };
        let matches = match baseline {
            Baseline::Missing => current.is_none(),
            Baseline::Exact(expected) => current.as_deref() == Some(expected.as_slice()),
        };
        matches
            .then_some(())
            .ok_or_else(|| StoreError::new(Operation::PreflightPrimary, ErrorKind::Conflict))
    }

    fn require_no_journal(&self) -> Result<(), StoreError> {
        match self
            .backend
            .read_bounded(&self.journal_path(), MAX_JOURNAL_BYTES)
        {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(StoreError::new(
                Operation::PreflightPrimary,
                ErrorKind::AmbiguousJournal,
            )),
            Err(error) => Err(StoreError::io(Operation::ReadJournal, error)),
        }
    }

    fn read_snapshot(
        &self,
        path: &Path,
        read_operation: Operation,
        parse_operation: Operation,
    ) -> Result<Option<(LibrarySnapshot, Vec<u8>)>, StoreError> {
        let bytes = match self.backend.read_bounded(path, MAX_LIBRARY_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(StoreError::io(read_operation, error)),
        };
        let snapshot = decode(&bytes).map_err(|error| StoreError::codec(parse_operation, error))?;
        Ok(Some((snapshot, bytes)))
    }

    fn restore_primary(&self, bytes: &[u8]) -> Result<(), StoreError> {
        self.backend
            .create_dir_all(&self.root)
            .map_err(|error| StoreError::io(Operation::CreateDirectory, error))?;
        self.write_and_verify_snapshot(
            &self.primary_path(),
            bytes,
            Operation::RecoverPrimary,
            Operation::VerifyPrimary,
        )
    }

    fn write_and_verify_snapshot(
        &self,
        path: &Path,
        bytes: &[u8],
        write_operation: Operation,
        verify_operation: Operation,
    ) -> Result<(), StoreError> {
        self.write_and_verify_raw(
            path,
            bytes,
            MAX_LIBRARY_BYTES,
            write_operation,
            verify_operation,
        )?;
        let decoded = decode(bytes).map_err(|error| StoreError::codec(verify_operation, error))?;
        let readback = self
            .backend
            .read_bounded(path, MAX_LIBRARY_BYTES)
            .map_err(|error| StoreError::io(verify_operation, error))?;
        let readback_decoded =
            decode(&readback).map_err(|error| StoreError::codec(verify_operation, error))?;
        if decoded != readback_decoded {
            return Err(StoreError::new(
                verify_operation,
                ErrorKind::ReadbackMismatch,
            ));
        }
        Ok(())
    }

    fn write_and_verify_raw(
        &self,
        path: &Path,
        bytes: &[u8],
        maximum: usize,
        write_operation: Operation,
        verify_operation: Operation,
    ) -> Result<(), StoreError> {
        self.backend
            .write_atomic_private(path, bytes)
            .map_err(|error| StoreError::io(write_operation, error))?;
        let readback = self
            .backend
            .read_bounded(path, maximum)
            .map_err(|error| StoreError::io(verify_operation, error))?;
        if readback != bytes {
            return Err(StoreError::new(
                verify_operation,
                ErrorKind::ReadbackMismatch,
            ));
        }
        Ok(())
    }

    fn remove_journal(&self) -> Result<(), StoreError> {
        match self.backend.remove_file(&self.journal_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::io(Operation::RemoveJournal, error)),
        }
    }

    fn primary_path(&self) -> PathBuf {
        self.root.join("library.bin")
    }

    fn last_good_path(&self) -> PathBuf {
        self.root.join("library.last-good.bin")
    }

    fn journal_path(&self) -> PathBuf {
        self.root.join("library.journal.bin")
    }

    fn purge_path(&self) -> PathBuf {
        self.root.join("library.purge.bin")
    }

    fn import_path(&self) -> PathBuf {
        self.root.join("library.attachment-import.bin")
    }
}

fn push_notice(notices: &mut Vec<RecoveryNotice>, notice: RecoveryNotice) {
    if !notices.contains(&notice) {
        notices.push(notice);
    }
}

pub(crate) fn managed_attachment_path(root: &Path, id: rmac_notes_store::AttachmentId) -> PathBuf {
    root.join("attachments")
        .join(format!("{:020}.bin", id.get()))
}

fn has_blocking_notice(notices: &[RecoveryNotice]) -> bool {
    notices.iter().any(|notice| {
        matches!(
            notice,
            RecoveryNotice::CorruptJournalPreserved
                | RecoveryNotice::MaintenancePending
                | RecoveryNotice::CorruptPurgePreserved
                | RecoveryNotice::PurgeCleanupPending
                | RecoveryNotice::CorruptAttachmentImportPreserved
                | RecoveryNotice::AttachmentImportPending
        )
    })
}

fn map_purge_error(operation: Operation, error: PurgeError) -> StoreError {
    match error {
        PurgeError::InvalidPlan | PurgeError::TooLarge | PurgeError::Malformed => {
            StoreError::new(operation, ErrorKind::InvalidPurge)
        }
        PurgeError::Io(kind) => StoreError::new(operation, ErrorKind::Io(kind)),
        PurgeError::Remove(kind) => {
            StoreError::new(Operation::RemovePurgeAttachment, ErrorKind::Io(kind))
        }
        PurgeError::ReadbackMismatch => StoreError::new(operation, ErrorKind::ReadbackMismatch),
        PurgeError::AttachmentMismatch => StoreError::new(operation, ErrorKind::AttachmentMismatch),
    }
}

fn map_import_error(operation: Operation, error: ImportError) -> StoreError {
    match error {
        ImportError::InvalidPlan | ImportError::Malformed => {
            StoreError::new(operation, ErrorKind::InvalidAttachmentImport)
        }
        ImportError::Unsupported => StoreError::new(operation, ErrorKind::UnsupportedAttachment),
        ImportError::TooLarge => StoreError::new(operation, ErrorKind::AttachmentTooLarge),
        ImportError::Io(kind) => StoreError::new(operation, ErrorKind::Io(kind)),
        ImportError::ReadbackMismatch => StoreError::new(operation, ErrorKind::ReadbackMismatch),
        ImportError::AttachmentMismatch => {
            StoreError::new(operation, ErrorKind::AttachmentMismatch)
        }
    }
}

fn import_error_from_store(error: StoreError) -> ImportError {
    match error.kind {
        ErrorKind::Io(kind) => ImportError::Io(kind),
        ErrorKind::ReadbackMismatch => ImportError::ReadbackMismatch,
        ErrorKind::AttachmentMismatch => ImportError::AttachmentMismatch,
        _ => ImportError::Malformed,
    }
}

#[derive(Clone, Debug)]
struct Journal {
    expected: ExpectedPrimary,
    candidate: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
enum ExpectedPrimary {
    Missing,
    Exact { length: u64, sha256: [u8; 32] },
}

impl Journal {
    fn new(baseline: &Baseline, candidate: Vec<u8>) -> Self {
        let expected = match baseline {
            Baseline::Missing => ExpectedPrimary::Missing,
            Baseline::Exact(bytes) => ExpectedPrimary::Exact {
                length: bytes.len() as u64,
                sha256: digest(bytes),
            },
        };
        Self {
            expected,
            candidate,
        }
    }

    fn encode(&self) -> Result<Vec<u8>, StoreError> {
        if self.candidate.len() > MAX_LIBRARY_BYTES {
            return Err(StoreError::new(
                Operation::ValidateCandidate,
                ErrorKind::Codec(CodecError::TooLarge),
            ));
        }
        let mut bytes = Vec::with_capacity(self.candidate.len() + 64);
        bytes.extend_from_slice(JOURNAL_MAGIC);
        bytes.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
        match self.expected {
            ExpectedPrimary::Missing => {
                bytes.push(0);
                bytes.extend_from_slice(&0_u64.to_le_bytes());
                bytes.extend_from_slice(&[0; 32]);
            }
            ExpectedPrimary::Exact { length, sha256 } => {
                bytes.push(1);
                bytes.extend_from_slice(&length.to_le_bytes());
                bytes.extend_from_slice(&sha256);
            }
        }
        bytes.extend_from_slice(&(self.candidate.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&self.candidate);
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        if bytes.len() > MAX_JOURNAL_BYTES {
            return Err(StoreError::new(
                Operation::ParseJournal,
                ErrorKind::Codec(CodecError::TooLarge),
            ));
        }
        let mut reader = Reader::new(bytes);
        if reader.take(8)? != JOURNAL_MAGIC || reader.u16()? != JOURNAL_VERSION {
            return Err(StoreError::new(
                Operation::ParseJournal,
                ErrorKind::Codec(CodecError::Malformed),
            ));
        }
        let present = reader.byte()?;
        let length = reader.u64()?;
        let sha256: [u8; 32] = reader
            .take(32)?
            .try_into()
            .map_err(|_| malformed_journal())?;
        let expected = match present {
            0 if length == 0 && sha256 == [0; 32] => ExpectedPrimary::Missing,
            1 => ExpectedPrimary::Exact { length, sha256 },
            _ => return Err(malformed_journal()),
        };
        let candidate_length = usize::try_from(reader.u64()?).map_err(|_| malformed_journal())?;
        if candidate_length > MAX_LIBRARY_BYTES {
            return Err(malformed_journal());
        }
        let candidate = reader.take(candidate_length)?.to_vec();
        if !reader.is_empty() {
            return Err(malformed_journal());
        }
        decode(&candidate).map_err(|error| StoreError::codec(Operation::ParseJournal, error))?;
        Ok(Self {
            expected,
            candidate,
        })
    }

    fn matches_expected(&self, current: Option<&[u8]>) -> bool {
        match (self.expected, current) {
            (ExpectedPrimary::Missing, None) => true,
            (ExpectedPrimary::Exact { length, sha256 }, Some(bytes)) => {
                bytes.len() as u64 == length && digest(bytes) == sha256
            }
            _ => false,
        }
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn malformed_journal() -> StoreError {
    StoreError::new(
        Operation::ParseJournal,
        ErrorKind::Codec(CodecError::Malformed),
    )
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], StoreError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or_else(malformed_journal)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(malformed_journal)?;
        self.cursor = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, StoreError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, StoreError> {
        self.take(2)?
            .try_into()
            .map(u16::from_le_bytes)
            .map_err(|_| malformed_journal())
    }

    fn u64(&mut self) -> Result<u64, StoreError> {
        self.take(8)?
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| malformed_journal())
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder as _;
    use rmac_notes_store::{
        AttachmentId, AttachmentKind, AttachmentRecord, LibraryTransaction, NewNote, NoteId,
        NoteRecord, SortOrder,
    };
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[derive(Default)]
    struct FakeState {
        files: BTreeMap<PathBuf, Vec<u8>>,
        fail_write: Option<PathBuf>,
        fail_after_write: Option<PathBuf>,
        fail_remove: Option<PathBuf>,
    }

    #[derive(Clone, Default)]
    struct FakeBackend(Arc<Mutex<FakeState>>);

    impl FakeBackend {
        fn set(&self, path: PathBuf, bytes: Vec<u8>) {
            self.0.lock().unwrap().files.insert(path, bytes);
        }

        fn get(&self, path: &Path) -> Option<Vec<u8>> {
            self.0.lock().unwrap().files.get(path).cloned()
        }

        fn fail_next_write(&self, path: PathBuf) {
            self.0.lock().unwrap().fail_write = Some(path);
        }

        fn fail_after_next_write(&self, path: PathBuf) {
            self.0.lock().unwrap().fail_after_write = Some(path);
        }

        fn fail_next_remove(&self, path: PathBuf) {
            self.0.lock().unwrap().fail_remove = Some(path);
        }
    }

    impl Backend for FakeBackend {
        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.0
                .lock()
                .unwrap()
                .files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn write_atomic_private(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
            let mut state = self.0.lock().unwrap();
            if state.fail_write.as_deref() == Some(path) {
                state.fail_write = None;
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            state.files.insert(path.to_path_buf(), contents.to_vec());
            if state.fail_after_write.as_deref() == Some(path) {
                state.fail_after_write = None;
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            Ok(())
        }

        fn write_new_private(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
            let mut state = self.0.lock().unwrap();
            if state.fail_write.as_deref() == Some(path) {
                state.fail_write = None;
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            if state.files.contains_key(path) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            state.files.insert(path.to_path_buf(), contents.to_vec());
            Ok(())
        }

        fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            let mut state = self.0.lock().unwrap();
            if state.fail_remove.as_deref() == Some(path) {
                state.fail_remove = None;
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            state
                .files
                .remove(path)
                .map(|_| ())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }

    fn candidate(base: &LibrarySnapshot, title: &str) -> LibrarySnapshot {
        let mut candidate = base.clone();
        candidate.revision += 1;
        candidate.notes.push(NoteRecord {
            id: NoteId::new(candidate.next_note_id).unwrap(),
            revision: 1,
            created_unix_ms: 1,
            modified_unix_ms: 1,
            title: title.into(),
            body: "body".into(),
            tags: Vec::new(),
            folder_id: None,
            pinned: false,
            deleted: false,
            attachments: Vec::new(),
        });
        candidate.next_note_id += 1;
        candidate
    }

    fn purge_fixture() -> (LibrarySnapshot, PathBuf, Vec<u8>) {
        let note_id = NoteId::new(1).unwrap();
        let attachment_id = AttachmentId::new(1).unwrap();
        let attachment_bytes = b"exact private managed attachment".to_vec();
        (
            LibrarySnapshot {
                revision: 2,
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
                    title: "Private trashed note".into(),
                    body: "Private body".into(),
                    tags: vec!["private-tag".into()],
                    folder_id: None,
                    pinned: false,
                    deleted: true,
                    attachments: vec![attachment_id],
                }],
                attachments: vec![AttachmentRecord {
                    id: attachment_id,
                    revision: 1,
                    note_id,
                    display_name: "private-image.png".into(),
                    kind: AttachmentKind::Png,
                    byte_len: attachment_bytes.len() as u64,
                    sha256: digest(&attachment_bytes),
                    deleted: false,
                }],
            },
            PathBuf::from("/virtual/library/attachments/00000000000000000001.bin"),
            attachment_bytes,
        )
    }

    fn install_purge_base(
        store: &NotesLibraryStore<FakeBackend>,
        backend: &FakeBackend,
    ) -> (LoadedLibrary, LibrarySnapshot, PathBuf, Vec<u8>) {
        let initial = store.load().unwrap();
        let (base, attachment_path, attachment_bytes) = purge_fixture();
        base.validate().unwrap();
        let loaded = store.save(&initial, &base).unwrap().library;
        backend.set(attachment_path.clone(), attachment_bytes.clone());
        (loaded, base, attachment_path, attachment_bytes)
    }

    fn purge_candidate(base: &LibrarySnapshot) -> (LibrarySnapshot, rmac_notes_store::PurgePlan) {
        let mut transaction = LibraryTransaction::begin(base).unwrap();
        let plan = transaction
            .purge_trashed_note(NoteId::new(1).unwrap(), 2)
            .unwrap();
        (transaction.finish().unwrap(), plan)
    }

    fn store() -> (NotesLibraryStore<FakeBackend>, FakeBackend) {
        let backend = FakeBackend::default();
        (
            NotesLibraryStore::with_backend(
                backend.clone(),
                WriterLease::for_fake_backend(PathBuf::from("/virtual/library")),
            ),
            backend,
        )
    }

    fn png_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[12, 34, 56, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    fn install_attachment_base(
        store: &NotesLibraryStore<FakeBackend>,
    ) -> (LoadedLibrary, LibrarySnapshot, NoteId) {
        let initial = store.load().unwrap();
        let mut transaction = LibraryTransaction::begin(initial.snapshot()).unwrap();
        let note_id = transaction
            .create_note(NewNote {
                created_unix_ms: 10,
                title: "Private note".into(),
                body: "Private body".into(),
                tags: Vec::new(),
                folder_id: None,
            })
            .unwrap();
        let base = transaction.finish().unwrap();
        let loaded = store.save(&initial, &base).unwrap().library;
        (loaded, base, note_id)
    }

    fn attachment_candidate(
        base: &LibrarySnapshot,
        note_id: NoteId,
        prepared: &PreparedImageAttachment,
    ) -> (LibrarySnapshot, AttachmentImportPlan) {
        let mut transaction = LibraryTransaction::begin(base).unwrap();
        let plan = transaction
            .add_attachment(note_id, 1, 11, prepared.metadata())
            .unwrap();
        (transaction.finish().unwrap(), plan)
    }

    #[test]
    fn selected_image_is_content_recognized_fully_decoded_and_path_free() {
        let (store, backend) = store();
        let selected = PathBuf::from("/portal/private-plan.not-an-image-extension");
        let bytes = png_bytes();
        backend.set(selected.clone(), bytes.clone());

        let prepared = store.prepare_image_attachment(&selected).unwrap();

        assert_eq!(prepared.kind(), AttachmentKind::Png);
        assert_eq!((prepared.width(), prepared.height()), (1, 1));
        assert_eq!(prepared.byte_len(), bytes.len() as u64);
        assert_eq!(prepared.metadata().display_name, "private-plan.png");
        let debug = format!("{prepared:?}");
        assert!(!debug.contains("private-plan"));
        assert!(!debug.contains("/portal"));

        backend.set(
            selected.clone(),
            b"plain text pretending to be an image".to_vec(),
        );
        assert_eq!(
            store.prepare_image_attachment(&selected).unwrap_err().kind,
            ErrorKind::UnsupportedAttachment
        );
        backend.set(selected, b"\x89PNG\r\n\x1a\ntruncated".to_vec());
        assert_eq!(
            store
                .prepare_image_attachment(Path::new("/portal/private-plan.not-an-image-extension"))
                .unwrap_err()
                .kind,
            ErrorKind::InvalidAttachmentImport
        );
    }

    #[test]
    fn attachment_import_stages_exact_bytes_then_publishes_metadata() {
        let (store, backend) = store();
        let selected = PathBuf::from("/portal/private-plan.png");
        let bytes = png_bytes();
        backend.set(selected.clone(), bytes.clone());
        let prepared = store.prepare_image_attachment(&selected).unwrap();
        let (loaded, base, note_id) = install_attachment_base(&store);
        let (candidate, plan) = attachment_candidate(&base, note_id, &prepared);

        let outcome = store
            .save_attachment_import(&loaded, &candidate, &plan, &prepared)
            .unwrap();

        assert_eq!(outcome.library.snapshot(), &candidate);
        assert!(!outcome.maintenance_pending);
        assert_eq!(
            backend.get(&managed_attachment_path(store.root(), plan.attachment_id)),
            Some(bytes)
        );
        assert_eq!(backend.get(&store.import_path()), None);
        assert_eq!(candidate.notes[0].attachments, vec![plan.attachment_id]);
    }

    #[test]
    fn rolled_back_import_removes_only_its_exact_staged_orphan() {
        let (store, backend) = store();
        let selected = PathBuf::from("/portal/private-plan.png");
        let bytes = png_bytes();
        backend.set(selected.clone(), bytes);
        let prepared = store.prepare_image_attachment(&selected).unwrap();
        let (loaded, base, note_id) = install_attachment_base(&store);
        let (candidate, plan) = attachment_candidate(&base, note_id, &prepared);
        backend.fail_next_write(store.primary_path());

        assert_eq!(
            store
                .save_attachment_import(&loaded, &candidate, &plan, &prepared)
                .unwrap_err()
                .operation,
            Operation::WritePrimary
        );
        let attachment_path = managed_attachment_path(store.root(), plan.attachment_id);
        assert!(backend.get(&attachment_path).is_some());
        assert!(backend.get(&store.import_path()).is_some());

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &base);
        assert_eq!(backend.get(&attachment_path), None);
        assert_eq!(backend.get(&store.import_path()), None);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::RolledBackInterruptedAttachmentImport));
    }

    #[test]
    fn accepted_import_recovery_keeps_verified_bytes_and_finishes_intent() {
        let (store, backend) = store();
        let selected = PathBuf::from("/portal/private-plan.png");
        let bytes = png_bytes();
        backend.set(selected.clone(), bytes.clone());
        let prepared = store.prepare_image_attachment(&selected).unwrap();
        let (loaded, base, note_id) = install_attachment_base(&store);
        let (candidate, plan) = attachment_candidate(&base, note_id, &prepared);
        backend.fail_after_next_write(store.primary_path());

        assert_eq!(
            store
                .save_attachment_import(&loaded, &candidate, &plan, &prepared)
                .unwrap_err()
                .operation,
            Operation::WritePrimary
        );
        let recovered = store.load().unwrap();

        assert_eq!(recovered.snapshot(), &candidate);
        assert_eq!(
            backend.get(&managed_attachment_path(store.root(), plan.attachment_id)),
            Some(bytes)
        );
        assert_eq!(backend.get(&store.import_path()), None);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::FinishedInterruptedAttachmentImport));
    }

    #[test]
    fn substituted_or_missing_managed_bytes_are_preserved_as_blocking_maintenance() {
        let (store, backend) = store();
        let selected = PathBuf::from("/portal/private-plan.png");
        backend.set(selected.clone(), png_bytes());
        let prepared = store.prepare_image_attachment(&selected).unwrap();
        let (loaded, base, note_id) = install_attachment_base(&store);
        let (candidate, plan) = attachment_candidate(&base, note_id, &prepared);
        backend.fail_next_write(store.primary_path());
        store
            .save_attachment_import(&loaded, &candidate, &plan, &prepared)
            .unwrap_err();
        let attachment_path = managed_attachment_path(store.root(), plan.attachment_id);
        let substituted = b"substituted private bytes".to_vec();
        backend.set(attachment_path.clone(), substituted.clone());

        let blocked = store.load().unwrap();
        assert_eq!(blocked.snapshot(), &base);
        assert_eq!(backend.get(&attachment_path), Some(substituted));
        assert!(backend.get(&store.import_path()).is_some());
        assert!(blocked
            .notices()
            .contains(&RecoveryNotice::AttachmentImportPending));
        assert_eq!(
            store.save(&blocked, &candidate).unwrap_err().kind,
            ErrorKind::InvalidAttachmentImport
        );
    }

    #[test]
    fn import_plan_mismatch_fails_before_intent_or_managed_bytes() {
        let (store, backend) = store();
        let selected = PathBuf::from("/portal/private-plan.png");
        backend.set(selected.clone(), png_bytes());
        let prepared = store.prepare_image_attachment(&selected).unwrap();
        let (loaded, base, note_id) = install_attachment_base(&store);
        let (mut candidate, plan) = attachment_candidate(&base, note_id, &prepared);
        candidate.notes[0].title = "Unrelated metadata change".into();

        assert_eq!(
            store
                .save_attachment_import(&loaded, &candidate, &plan, &prepared)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidAttachmentImport
        );
        assert_eq!(backend.get(&store.import_path()), None);
        assert_eq!(
            backend.get(&managed_attachment_path(store.root(), plan.attachment_id)),
            None
        );
    }

    fn legacy_fixture() -> LegacyLibraryInput {
        LegacyLibraryInput {
            folder_names: vec!["Projects".into()],
            notes: vec![LegacyNoteInput {
                relative_path: "Projects/roadmap.md".into(),
                bytes: b"Roadmap\nKeep every byte\n![](diagram.png)".to_vec(),
                created_unix_ms: 10,
                modified_unix_ms: 20,
            }],
            attachments: vec![
                LegacyAttachmentInput {
                    relative_path: "diagram.png".into(),
                    bytes: b"\x89PNG\r\n\x1a\nfixture".to_vec(),
                },
                LegacyAttachmentInput {
                    relative_path: "unclaimed.bin".into(),
                    bytes: b"preserve unsupported bytes".to_vec(),
                },
            ],
            pinned_note_paths: vec!["Projects/roadmap.md".into()],
            sort_order: SortOrder::Title,
        }
    }

    #[test]
    fn new_library_commit_verifies_primary_backup_and_cleanup() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let next = candidate(loaded.snapshot(), "First");
        let outcome = store.save(&loaded, &next).unwrap();

        assert!(!outcome.maintenance_pending);
        let expected = encode(&next).unwrap();
        assert_eq!(backend.get(&store.primary_path()), Some(expected.clone()));
        assert_eq!(backend.get(&store.last_good_path()), Some(expected));
        assert_eq!(backend.get(&store.journal_path()), None);
        assert_eq!(store.load().unwrap().snapshot(), &next);
    }

    #[test]
    fn exact_preflight_rejects_an_external_replacement() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let external = candidate(loaded.snapshot(), "External");
        backend.set(store.primary_path(), encode(&external).unwrap());
        let local = candidate(loaded.snapshot(), "Local");

        assert_eq!(
            store.save(&loaded, &local).unwrap_err(),
            StoreError::new(Operation::PreflightPrimary, ErrorKind::Conflict)
        );
        assert_eq!(backend.get(&store.journal_path()), None);
    }

    #[test]
    fn primary_write_failure_preserves_previous_state_and_recoverable_journal() {
        let (store, backend) = store();
        let initial = store.load().unwrap();
        let first = candidate(initial.snapshot(), "First");
        let loaded = store.save(&initial, &first).unwrap().library;
        let second = candidate(loaded.snapshot(), "Second");
        backend.fail_next_write(store.primary_path());

        assert_eq!(
            store.save(&loaded, &second).unwrap_err().operation,
            Operation::WritePrimary
        );
        assert_eq!(
            backend.get(&store.primary_path()),
            Some(encode(&first).unwrap())
        );
        assert!(backend.get(&store.journal_path()).is_some());
        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &first);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::RolledBackInterruptedSave));
        assert_eq!(backend.get(&store.journal_path()), None);
    }

    #[test]
    fn interrupted_backup_is_finished_from_verified_primary_and_journal() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let next = candidate(loaded.snapshot(), "First");
        backend.fail_next_write(store.last_good_path());
        let outcome = store.save(&loaded, &next).unwrap();
        assert!(outcome.maintenance_pending);
        assert!(backend.get(&store.journal_path()).is_some());

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &next);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::FinishedInterruptedSave));
        assert_eq!(
            backend.get(&store.last_good_path()),
            Some(encode(&next).unwrap())
        );
        assert_eq!(backend.get(&store.journal_path()), None);
    }

    #[test]
    fn prepared_but_uncommitted_journal_rolls_back() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let next = candidate(loaded.snapshot(), "Uncommitted");
        let journal = Journal::new(&loaded.baseline, encode(&next).unwrap())
            .encode()
            .unwrap();
        backend.set(store.journal_path(), journal);

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), loaded.snapshot());
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::RolledBackInterruptedSave));
        assert_eq!(backend.get(&store.journal_path()), None);
    }

    #[test]
    fn corrupt_primary_recovers_exact_last_known_good() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let next = candidate(loaded.snapshot(), "Durable");
        store.save(&loaded, &next).unwrap();
        backend.set(store.primary_path(), b"corrupt".to_vec());

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &next);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::RecoveredLastKnownGood));
        assert_eq!(
            backend.get(&store.primary_path()),
            Some(encode(&next).unwrap())
        );
    }

    #[test]
    fn malformed_journal_never_hides_a_valid_primary() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let next = candidate(loaded.snapshot(), "Durable");
        store.save(&loaded, &next).unwrap();
        backend.set(store.journal_path(), b"malformed".to_vec());

        let reopened = store.load().unwrap();
        assert_eq!(reopened.snapshot(), &next);
        assert!(reopened
            .notices()
            .contains(&RecoveryNotice::CorruptJournalPreserved));
        assert_eq!(
            backend.get(&store.journal_path()),
            Some(b"malformed".to_vec())
        );
        let later = candidate(reopened.snapshot(), "Blocked");
        assert_eq!(
            store.save(&reopened, &later).unwrap_err().kind,
            ErrorKind::AmbiguousJournal
        );
    }

    #[test]
    fn cleanup_failure_reports_committed_state_and_retries_on_load() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let next = candidate(loaded.snapshot(), "Committed");
        backend.fail_next_remove(store.journal_path());

        let outcome = store.save(&loaded, &next).unwrap();
        assert!(outcome.maintenance_pending);
        assert_eq!(outcome.library.snapshot(), &next);
        assert!(backend.get(&store.journal_path()).is_some());

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &next);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::FinishedInterruptedSave));
        assert_eq!(backend.get(&store.journal_path()), None);
    }

    #[test]
    fn accepted_purge_removes_only_verified_attachment_then_intent() {
        let (store, backend) = store();
        let (loaded, base, attachment_path, _) = install_purge_base(&store, &backend);
        let (purged, plan) = purge_candidate(&base);

        let outcome = store.save_purge(&loaded, &purged, &plan).unwrap();

        assert_eq!(outcome.library.snapshot(), &purged);
        assert!(!outcome.maintenance_pending);
        assert!(!outcome.purge_cleanup_pending);
        assert_eq!(backend.get(&attachment_path), None);
        assert_eq!(backend.get(&store.purge_path()), None);
        assert_eq!(store.load().unwrap().snapshot(), &purged);
    }

    #[test]
    fn rolled_back_metadata_never_deletes_attachment_bytes() {
        let (store, backend) = store();
        let (loaded, base, attachment_path, attachment_bytes) =
            install_purge_base(&store, &backend);
        let (purged, plan) = purge_candidate(&base);
        backend.fail_next_write(store.primary_path());

        assert_eq!(
            store
                .save_purge(&loaded, &purged, &plan)
                .unwrap_err()
                .operation,
            Operation::WritePrimary
        );
        assert!(backend.get(&store.purge_path()).is_some());
        assert_eq!(
            backend.get(&attachment_path),
            Some(attachment_bytes.clone())
        );

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &base);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::RolledBackInterruptedPurge));
        assert_eq!(backend.get(&store.purge_path()), None);
        assert_eq!(backend.get(&attachment_path), Some(attachment_bytes));
    }

    #[test]
    fn purge_waits_for_metadata_maintenance_then_resumes_on_load() {
        let (store, backend) = store();
        let (loaded, base, attachment_path, attachment_bytes) =
            install_purge_base(&store, &backend);
        let (purged, plan) = purge_candidate(&base);
        backend.fail_next_write(store.last_good_path());

        let outcome = store.save_purge(&loaded, &purged, &plan).unwrap();
        assert!(outcome.maintenance_pending);
        assert!(outcome.purge_cleanup_pending);
        assert_eq!(backend.get(&attachment_path), Some(attachment_bytes));
        assert!(backend.get(&store.purge_path()).is_some());

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &purged);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::FinishedInterruptedSave));
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::FinishedInterruptedPurge));
        assert_eq!(backend.get(&attachment_path), None);
        assert_eq!(backend.get(&store.purge_path()), None);
    }

    #[test]
    fn changed_attachment_is_preserved_and_blocks_later_mutation() {
        let (store, backend) = store();
        let (loaded, base, attachment_path, _) = install_purge_base(&store, &backend);
        let (purged, plan) = purge_candidate(&base);
        let substituted = b"different private bytes".to_vec();
        backend.set(attachment_path.clone(), substituted.clone());

        let outcome = store.save_purge(&loaded, &purged, &plan).unwrap();
        assert!(outcome.maintenance_pending);
        assert!(outcome.purge_cleanup_pending);
        assert_eq!(backend.get(&attachment_path), Some(substituted.clone()));
        assert!(backend.get(&store.purge_path()).is_some());

        let reopened = store.load().unwrap();
        assert_eq!(reopened.snapshot(), &purged);
        assert!(reopened
            .notices()
            .contains(&RecoveryNotice::PurgeCleanupPending));
        assert_eq!(backend.get(&attachment_path), Some(substituted));
        let later = candidate(&purged, "must remain blocked");
        assert_eq!(
            store.save(&reopened, &later).unwrap_err().kind,
            ErrorKind::InvalidPurge
        );
    }

    #[test]
    fn attachment_remove_failure_is_resumed_without_republishing_metadata() {
        let (store, backend) = store();
        let (loaded, base, attachment_path, attachment_bytes) =
            install_purge_base(&store, &backend);
        let (purged, plan) = purge_candidate(&base);
        backend.fail_next_remove(attachment_path.clone());

        let outcome = store.save_purge(&loaded, &purged, &plan).unwrap();
        assert!(outcome.purge_cleanup_pending);
        assert_eq!(backend.get(&attachment_path), Some(attachment_bytes));
        assert_eq!(
            backend.get(&store.primary_path()),
            Some(encode(&purged).unwrap())
        );

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &purged);
        assert!(recovered
            .notices()
            .contains(&RecoveryNotice::FinishedInterruptedPurge));
        assert_eq!(backend.get(&attachment_path), None);
        assert_eq!(backend.get(&store.purge_path()), None);
    }

    #[test]
    fn malformed_purge_intent_is_preserved_and_blocks_writes() {
        let (store, backend) = store();
        backend.set(store.purge_path(), b"malformed private intent".to_vec());

        let loaded = store.load().unwrap();

        assert!(loaded
            .notices()
            .contains(&RecoveryNotice::CorruptPurgePreserved));
        assert_eq!(
            backend.get(&store.purge_path()),
            Some(b"malformed private intent".to_vec())
        );
        let next = candidate(loaded.snapshot(), "blocked");
        assert_eq!(
            store.save(&loaded, &next).unwrap_err().kind,
            ErrorKind::InvalidPurge
        );
    }

    #[test]
    fn journal_header_is_versioned_and_candidate_is_revalidated() {
        let loaded = LoadedLibrary {
            snapshot: LibrarySnapshot::default(),
            baseline: Baseline::Missing,
            notices: Vec::new(),
        };
        let next = candidate(loaded.snapshot(), "Candidate");
        let mut bytes = Journal::new(&loaded.baseline, encode(&next).unwrap())
            .encode()
            .unwrap();
        assert_eq!(&bytes[..8], JOURNAL_MAGIC);
        bytes[8..10].copy_from_slice(&(JOURNAL_VERSION + 1).to_le_bytes());
        assert!(Journal::decode(&bytes).is_err());
        assert_eq!(rmac_notes_store::SCHEMA_VERSION, 2);
    }

    #[test]
    fn migration_preserves_every_source_before_metadata_and_is_idempotent() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let input = legacy_fixture();
        let plan = plan_legacy_library(input.clone()).unwrap();

        let outcome = store
            .commit_legacy_migration(&loaded, &input, &plan)
            .unwrap();

        assert!(!outcome.already_committed);
        assert!(!outcome.maintenance_pending);
        assert_eq!(outcome.library.snapshot(), &plan.snapshot);
        assert_eq!(
            backend.get(&PathBuf::from(
                "/virtual/library/legacy-recovery/notes/00000000000000000001.md"
            )),
            Some(input.notes[0].bytes.clone())
        );
        assert_eq!(
            backend.get(&PathBuf::from(
                "/virtual/library/legacy-recovery/files/00000000000000000000.bin"
            )),
            Some(input.attachments[0].bytes.clone())
        );
        assert_eq!(
            backend.get(&PathBuf::from(
                "/virtual/library/legacy-recovery/files/00000000000000000001.bin"
            )),
            Some(input.attachments[1].bytes.clone())
        );
        assert_eq!(
            backend.get(&PathBuf::from(
                "/virtual/library/attachments/00000000000000000001.bin"
            )),
            Some(input.attachments[0].bytes.clone())
        );
        assert!(backend
            .get(&PathBuf::from(
                "/virtual/library/legacy-recovery/receipt.bin",
            ))
            .unwrap()
            .starts_with(b"RMNMIG\0\0"));

        let retry = store
            .commit_legacy_migration(&outcome.library, &input, &plan)
            .unwrap();
        assert!(retry.already_committed);
        assert_eq!(retry.library.snapshot(), &plan.snapshot);
    }

    #[test]
    fn changed_reread_or_conflicting_recovery_data_never_publishes_metadata() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let input = legacy_fixture();
        let plan = plan_legacy_library(input.clone()).unwrap();
        let mut changed = input.clone();
        changed.notes[0].bytes = b"Changed after review".to_vec();

        let changed_error = store
            .commit_legacy_migration(&loaded, &changed, &plan)
            .unwrap_err();
        assert_eq!(changed_error.kind, MigrationCommitErrorKind::PlanMismatch);
        assert_eq!(backend.get(&store.primary_path()), None);

        backend.set(
            PathBuf::from("/virtual/library/legacy-recovery/notes/00000000000000000001.md"),
            b"unrelated existing data".to_vec(),
        );
        let conflict = store
            .commit_legacy_migration(&loaded, &input, &plan)
            .unwrap_err();
        assert_eq!(conflict.kind, MigrationCommitErrorKind::DestinationConflict);
        assert_eq!(backend.get(&store.primary_path()), None);
        assert_eq!(backend.get(&store.journal_path()), None);
    }

    #[test]
    fn metadata_failure_leaves_verified_staging_for_a_safe_retry() {
        let (store, backend) = store();
        let loaded = store.load().unwrap();
        let input = legacy_fixture();
        let plan = plan_legacy_library(input.clone()).unwrap();
        backend.fail_next_write(store.primary_path());

        let error = store
            .commit_legacy_migration(&loaded, &input, &plan)
            .unwrap_err();
        assert_eq!(error.operation, MigrationCommitOperation::CommitMetadata);
        assert_eq!(
            error.kind,
            MigrationCommitErrorKind::Store(StoreError::new(
                Operation::WritePrimary,
                ErrorKind::Io(io::ErrorKind::PermissionDenied)
            ))
        );
        assert!(backend
            .get(&PathBuf::from(
                "/virtual/library/legacy-recovery/receipt.bin",
            ))
            .is_some());
        assert!(backend
            .get(&PathBuf::from(
                "/virtual/library/attachments/00000000000000000001.bin"
            ))
            .is_some());

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &LibrarySnapshot::default());
        let retry = store
            .commit_legacy_migration(&recovered, &input, &plan)
            .unwrap();
        assert_eq!(retry.library.snapshot(), &plan.snapshot);
        assert!(!retry.already_committed);
    }

    #[test]
    fn migration_refuses_to_stage_over_a_nonempty_library() {
        let (store, backend) = store();
        let empty = store.load().unwrap();
        let existing = candidate(empty.snapshot(), "Existing");
        let loaded = store.save(&empty, &existing).unwrap().library;
        let input = legacy_fixture();
        let plan = plan_legacy_library(input.clone()).unwrap();

        let error = store
            .commit_legacy_migration(&loaded, &input, &plan)
            .unwrap_err();

        assert_eq!(error.kind, MigrationCommitErrorKind::NonEmptyLibrary);
        assert_eq!(
            backend.get(&PathBuf::from(
                "/virtual/library/legacy-recovery/receipt.bin"
            )),
            None
        );
        assert_eq!(store.load().unwrap().snapshot(), &existing);
    }

    #[test]
    fn accepted_repository_publishes_only_verified_transactions() {
        let (store, _backend) = store();
        let mut repository = AcceptedLibrary::open(store).unwrap();
        let mut transaction = repository.begin().unwrap();
        let note_id = transaction
            .create_note(NewNote {
                created_unix_ms: 10,
                title: "Accepted".into(),
                body: "Durable body".into(),
                tags: vec!["rmac".into()],
                folder_id: None,
            })
            .unwrap();

        let accepted = repository.commit(transaction).unwrap();

        assert_eq!(accepted.revision, 2);
        assert!(!accepted.maintenance_pending);
        assert!(!accepted.recovered_after_error);
        assert_eq!(repository.snapshot().notes[0].id, note_id);
        assert_eq!(repository.snapshot().notes[0].title, "Accepted");
    }

    #[test]
    fn failed_repository_commit_retains_candidate_and_retries_after_recovery() {
        let (store, backend) = store();
        let primary = store.primary_path();
        let mut repository = AcceptedLibrary::open(store).unwrap();
        let accepted_before = repository.snapshot().clone();
        let mut transaction = repository.begin().unwrap();
        transaction
            .create_note(NewNote {
                created_unix_ms: 10,
                title: "Pending".into(),
                body: "Never discard this".into(),
                tags: Vec::new(),
                folder_id: None,
            })
            .unwrap();
        backend.fail_next_write(primary);

        let CommitError::Pending(pending) = repository.commit(transaction).unwrap_err() else {
            panic!("storage failure must return a pending candidate");
        };
        assert_eq!(repository.snapshot(), &accepted_before);
        assert_eq!(pending.candidate().notes[0].title, "Pending");

        let accepted = repository.retry(pending).unwrap();
        assert!(accepted.recovered_after_error);
        assert_eq!(repository.snapshot().notes[0].body, "Never discard this");
    }

    #[test]
    fn repository_retry_adopts_a_candidate_committed_before_reported_failure() {
        let (store, backend) = store();
        let primary = store.primary_path();
        let mut repository = AcceptedLibrary::open(store).unwrap();
        let mut transaction = repository.begin().unwrap();
        transaction
            .create_note(NewNote {
                created_unix_ms: 10,
                title: "Committed during error".into(),
                body: "Exact candidate".into(),
                tags: Vec::new(),
                folder_id: None,
            })
            .unwrap();
        backend.fail_after_next_write(primary);

        let CommitError::Pending(pending) = repository.commit(transaction).unwrap_err() else {
            panic!("the reported write error must retain the candidate");
        };
        assert_eq!(repository.snapshot(), &LibrarySnapshot::default());

        let accepted = repository.retry(pending).unwrap();

        assert!(accepted.recovered_after_error);
        assert_eq!(accepted.revision, 2);
        assert_eq!(repository.snapshot().revision, 2);
        assert_eq!(
            repository.snapshot().notes[0].title,
            "Committed during error"
        );
        assert!(repository
            .recovery_notices()
            .contains(&RecoveryNotice::FinishedInterruptedSave));
    }

    #[test]
    fn repository_retry_surfaces_unrelated_durable_change_without_overwrite() {
        let (store, backend) = store();
        let primary = store.primary_path();
        let mut repository = AcceptedLibrary::open(store).unwrap();
        let mut transaction = repository.begin().unwrap();
        transaction
            .create_note(NewNote {
                created_unix_ms: 10,
                title: "Local".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id: None,
            })
            .unwrap();
        let external = candidate(repository.snapshot(), "External");
        backend.set(primary, encode(&external).unwrap());

        let CommitError::Pending(pending) = repository.commit(transaction).unwrap_err() else {
            panic!("the exact preflight must retain the local candidate");
        };
        let pending = repository.retry(pending).unwrap_err();

        assert_eq!(pending.reason, PendingReason::AcceptedStateChanged);
        assert_eq!(pending.candidate().notes[0].title, "Local");
        assert_eq!(repository.snapshot(), &external);
        assert_eq!(repository.snapshot().notes[0].title, "External");
    }

    #[test]
    fn accepted_repository_reports_verified_purge_cleanup_separately() {
        let (store, backend) = store();
        let (loaded, _base, attachment_path, _) = install_purge_base(&store, &backend);
        let mut repository = AcceptedLibrary::from_loaded(store, loaded);
        let mut transaction = repository.begin().unwrap();
        let plan = transaction
            .purge_trashed_note(NoteId::new(1).unwrap(), 2)
            .unwrap();

        let accepted = repository.commit_purge(transaction, plan).unwrap();

        assert_eq!(accepted.revision, 3);
        assert!(!accepted.maintenance_pending);
        assert!(!accepted.purge_cleanup_pending);
        assert_eq!(backend.get(&attachment_path), None);
        assert!(repository.snapshot().notes.is_empty());
    }

    #[test]
    fn repository_retry_retains_purge_plan_and_rolled_back_bytes() {
        let (store, backend) = store();
        let (loaded, _base, attachment_path, attachment_bytes) =
            install_purge_base(&store, &backend);
        let primary = store.primary_path();
        let mut repository = AcceptedLibrary::from_loaded(store, loaded);
        let mut transaction = repository.begin().unwrap();
        let plan = transaction
            .purge_trashed_note(NoteId::new(1).unwrap(), 2)
            .unwrap();
        backend.fail_next_write(primary);

        let CommitError::Pending(pending) = repository
            .commit_purge(transaction, plan.clone())
            .unwrap_err()
        else {
            panic!("expected retained purge candidate")
        };
        assert_eq!(pending.purge_plan(), Some(&plan));
        assert_eq!(backend.get(&attachment_path), Some(attachment_bytes));

        let accepted = repository.retry(pending).unwrap();
        assert!(accepted.recovered_after_error);
        assert!(!accepted.purge_cleanup_pending);
        assert_eq!(backend.get(&attachment_path), None);
        assert!(repository.snapshot().notes.is_empty());
    }

    #[test]
    fn repository_retry_retains_prepared_image_without_exposing_private_data() {
        let (store, backend) = store();
        let primary = store.primary_path();
        let mut repository = AcceptedLibrary::open(store).unwrap();
        let mut create = repository.begin().unwrap();
        let note_id = create
            .create_note(NewNote {
                created_unix_ms: 10,
                title: "Private attachment note".into(),
                body: "Private attachment body".into(),
                tags: Vec::new(),
                folder_id: None,
            })
            .unwrap();
        repository.commit(create).unwrap();

        let selected = PathBuf::from("/portal/private-retry-source.dat");
        backend.set(selected.clone(), png_bytes());
        let prepared = repository.prepare_image_attachment(&selected).unwrap();
        let mut transaction = repository.begin().unwrap();
        let plan = transaction
            .add_attachment(note_id, 1, 11, prepared.metadata())
            .unwrap();
        backend.fail_next_write(primary);

        let CommitError::Pending(pending) = repository
            .commit_attachment_import(transaction, plan.clone(), prepared)
            .unwrap_err()
        else {
            panic!("expected retained attachment candidate")
        };
        assert_eq!(pending.attachment_import_plan(), Some(&plan));
        let debug = format!("{pending:?}");
        assert!(!debug.contains("private-retry-source"));
        assert!(!debug.contains("Private attachment"));

        let accepted = repository.retry(pending).unwrap();
        assert!(accepted.recovered_after_error);
        assert!(!accepted.attachment_import_pending);
        assert_eq!(repository.snapshot().attachments.len(), 1);
        assert_eq!(repository.snapshot().attachments[0].id, plan.attachment_id);
        assert!(backend
            .get(&managed_attachment_path(
                repository.root(),
                plan.attachment_id
            ))
            .is_some());
    }

    #[test]
    fn repository_refuses_noop_transactions_without_writing() {
        let (store, backend) = store();
        let mut repository = AcceptedLibrary::open(store).unwrap();
        let transaction = repository.begin().unwrap();

        assert_eq!(
            repository.commit(transaction).unwrap_err(),
            CommitError::Mutation(rmac_notes_store::MutationError::NoChanges)
        );
        assert!(backend.0.lock().unwrap().files.is_empty());
        assert_eq!(repository.snapshot(), &LibrarySnapshot::default());
    }
}
