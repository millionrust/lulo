use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rmac_notes_store::{
    decode, encode, AttachmentImportPlan, BundleImportPlan, LibrarySnapshot, OrphanCollectionPlan,
    PurgePlan, MAX_LIBRARY_BYTES,
};
use rmac_storage::{Backend, FileSystem};

use crate::attachment::{
    self, ImportAuthority, ImportError, ImportIntent, MAX_IMPORT_INTENT_BYTES,
};
use crate::bundle_import::{
    self, BundleImportIntent, BundleIntentAuthority, MAX_BUNDLE_IMPORT_INTENT_BYTES,
};
use crate::journal::{Journal, MAX_JOURNAL_BYTES};
use crate::migration;
use crate::model::Baseline;
use crate::note_import;
use crate::orphan::{OrphanAuthority, OrphanError, OrphanIntent, MAX_ORPHAN_INTENT_BYTES};
use crate::purge::{PurgeAuthority, PurgeError, PurgeIntent, MAX_PURGE_INTENT_BYTES};
use crate::{
    BundleImportError, BundleImportErrorKind, BundleImportOperation, ErrorKind, LegacyLibraryInput,
    LoadedLibrary, MigrationCommitError, MigrationCommitOutcome, MigrationPlan, Operation,
    PreparedBundleImport, PreparedImageAttachment, PreparedTextNote, RecoveryNotice, SaveOutcome,
    StoreError, TextImportError, WriterLease, WriterLeaseError,
};

pub struct NotesLibraryStore<B = FileSystem> {
    pub(crate) root: PathBuf,
    pub(crate) backend: B,
    _writer_lease: WriterLease,
    pub(crate) transaction_lock: Mutex<()>,
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

    /// Strictly decode one portal-selected Markdown/plain-text source into a
    /// path-free note candidate. This never mutates or retains the source.
    pub fn prepare_text_note(
        &self,
        selected_path: &Path,
    ) -> Result<PreparedTextNote, TextImportError> {
        note_import::prepare_text_note_with_backend(selected_path, &self.backend)
    }

    /// Stream and verify one portal-selected rmac Notes bundle while retaining
    /// at most one bounded attachment payload during full image decoding.
    pub fn prepare_bundle_import(
        &self,
        selected_path: &Path,
    ) -> Result<PreparedBundleImport, BundleImportError> {
        let prepared = bundle_import::prepare_bundle_import(selected_path)?;
        if prepared.source_is_within(&self.root) {
            return Err(BundleImportError {
                operation: BundleImportOperation::ReviewSource,
                kind: BundleImportErrorKind::Malformed,
            });
        }
        Ok(prepared)
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
        self.require_no_orphan_intent()?;
        self.require_no_bundle_import_intent()?;
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
        self.require_no_orphan_intent()?;
        self.require_no_bundle_import_intent()?;
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

    /// Stage every exact bundle attachment under its collision-reviewed
    /// destination identity, then publish the complete metadata candidate.
    pub fn save_bundle_import(
        &self,
        loaded: &LoadedLibrary,
        candidate: &LibrarySnapshot,
        plan: &BundleImportPlan,
        prepared: &PreparedBundleImport,
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
        self.require_no_orphan_intent()?;
        self.require_no_bundle_import_intent()?;
        let intent = BundleImportIntent::prepare(loaded.snapshot(), candidate, plan, prepared)
            .map_err(|error| map_bundle_error(Operation::WriteBundleImportIntent, error))?;
        self.write_bundle_import_intent(&intent)?;
        intent
            .stage(&self.root, &self.backend, plan, prepared)
            .map_err(|error| map_bundle_error(Operation::StageBundleAttachment, error))?;
        let mut outcome = self.save_locked(loaded, candidate)?;
        if outcome.maintenance_pending {
            outcome.bundle_import_pending = true;
            push_notice(
                &mut outcome.library.notices,
                RecoveryNotice::BundleImportPending,
            );
            return Ok(outcome);
        }
        if self.remove_bundle_import_intent().is_err() {
            outcome.maintenance_pending = true;
            outcome.bundle_import_pending = true;
            push_notice(
                &mut outcome.library.notices,
                RecoveryNotice::BundleImportPending,
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
        self.require_no_orphan_intent()?;
        self.require_no_bundle_import_intent()?;
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

    /// Publish the exact metadata removal for one reviewed tombstone, then
    /// durably collect only its identity-bound managed bytes.
    pub fn save_orphan_collection(
        &self,
        loaded: &LoadedLibrary,
        candidate: &LibrarySnapshot,
        plan: &OrphanCollectionPlan,
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
        self.require_no_orphan_intent()?;
        self.require_no_bundle_import_intent()?;
        let intent = OrphanIntent::prepare(loaded.snapshot(), candidate, plan)
            .map_err(|error| map_orphan_error(Operation::WriteOrphanCollectionIntent, error))?;
        self.write_orphan_intent(&intent)?;
        let mut outcome = self.save_locked(loaded, candidate)?;
        if outcome.maintenance_pending {
            outcome.orphan_collection_pending = true;
            push_notice(
                &mut outcome.library.notices,
                RecoveryNotice::OrphanCollectionPending,
            );
            return Ok(outcome);
        }
        if self.finish_orphan_collection(&intent).is_err() {
            outcome.maintenance_pending = true;
            outcome.orphan_collection_pending = true;
            push_notice(
                &mut outcome.library.notices,
                RecoveryNotice::OrphanCollectionPending,
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

    pub(crate) fn load_locked(&self) -> Result<LoadedLibrary, StoreError> {
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
        let orphan_present = self.intent_present(&self.orphan_path(), MAX_ORPHAN_INTENT_BYTES);
        let bundle_import_present =
            self.intent_present(&self.bundle_import_path(), MAX_BUNDLE_IMPORT_INTENT_BYTES);
        if [
            purge_present,
            import_present,
            orphan_present,
            bundle_import_present,
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            > 1
        {
            let mut loaded = loaded;
            if purge_present {
                push_notice(&mut loaded.notices, RecoveryNotice::CorruptPurgePreserved);
            }
            if import_present {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptAttachmentImportPreserved,
                );
            }
            if orphan_present {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptOrphanCollectionPreserved,
                );
            }
            if bundle_import_present {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptBundleImportPreserved,
                );
            }
            return Ok(loaded);
        }
        let loaded = self.recover_purge(loaded)?;
        let loaded = self.recover_attachment_import(loaded)?;
        let loaded = self.recover_orphan_collection(loaded)?;
        self.recover_bundle_import(loaded)
    }

    pub(crate) fn save_locked(
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
            orphan_collection_pending: false,
            bundle_import_pending: false,
        })
    }

    pub(crate) fn recover_purge(
        &self,
        mut loaded: LoadedLibrary,
    ) -> Result<LoadedLibrary, StoreError> {
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

    pub(crate) fn recover_attachment_import(
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

    pub(crate) fn recover_orphan_collection(
        &self,
        mut loaded: LoadedLibrary,
    ) -> Result<LoadedLibrary, StoreError> {
        let bytes = match self
            .backend
            .read_bounded_no_follow(&self.orphan_path(), MAX_ORPHAN_INTENT_BYTES)
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(loaded),
            Err(_) => {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptOrphanCollectionPreserved,
                );
                return Ok(loaded);
            }
        };
        let intent = match OrphanIntent::decode(&bytes) {
            Ok(intent) => intent,
            Err(_) => {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptOrphanCollectionPreserved,
                );
                return Ok(loaded);
            }
        };
        match intent
            .authority(&loaded.snapshot)
            .map_err(|error| map_orphan_error(Operation::ReadOrphanCollectionIntent, error))?
        {
            OrphanAuthority::RolledBack => {
                if self.remove_orphan_intent().is_ok() {
                    push_notice(
                        &mut loaded.notices,
                        RecoveryNotice::RolledBackInterruptedOrphanCollection,
                    );
                } else {
                    push_notice(&mut loaded.notices, RecoveryNotice::OrphanCollectionPending);
                }
            }
            OrphanAuthority::Accepted | OrphanAuthority::AcceptedDescendant => {
                if self.finish_orphan_collection(&intent).is_ok() {
                    push_notice(
                        &mut loaded.notices,
                        RecoveryNotice::FinishedInterruptedOrphanCollection,
                    );
                } else {
                    push_notice(&mut loaded.notices, RecoveryNotice::OrphanCollectionPending);
                }
            }
            OrphanAuthority::Ambiguous => push_notice(
                &mut loaded.notices,
                RecoveryNotice::CorruptOrphanCollectionPreserved,
            ),
        }
        Ok(loaded)
    }

    pub(crate) fn recover_bundle_import(
        &self,
        mut loaded: LoadedLibrary,
    ) -> Result<LoadedLibrary, StoreError> {
        let bytes = match self
            .backend
            .read_bounded_no_follow(&self.bundle_import_path(), MAX_BUNDLE_IMPORT_INTENT_BYTES)
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(loaded),
            Err(_) => {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptBundleImportPreserved,
                );
                return Ok(loaded);
            }
        };
        let intent = match BundleImportIntent::decode(&bytes) {
            Ok(intent) => intent,
            Err(_) => {
                push_notice(
                    &mut loaded.notices,
                    RecoveryNotice::CorruptBundleImportPreserved,
                );
                return Ok(loaded);
            }
        };
        match intent
            .authority(&loaded.snapshot)
            .map_err(|error| map_bundle_error(Operation::ReadBundleImportIntent, error))?
        {
            BundleIntentAuthority::RolledBack => {
                if intent
                    .rollback_staging(&self.root, &self.backend)
                    .and_then(|()| {
                        self.remove_bundle_import_intent()
                            .map_err(bundle_error_from_store)
                    })
                    .is_ok()
                {
                    push_notice(
                        &mut loaded.notices,
                        RecoveryNotice::RolledBackInterruptedBundleImport,
                    );
                } else {
                    push_notice(&mut loaded.notices, RecoveryNotice::BundleImportPending);
                }
            }
            BundleIntentAuthority::Accepted => {
                if intent
                    .verify_staged(&self.root, &self.backend)
                    .and_then(|()| {
                        self.remove_bundle_import_intent()
                            .map_err(bundle_error_from_store)
                    })
                    .is_ok()
                {
                    push_notice(
                        &mut loaded.notices,
                        RecoveryNotice::FinishedInterruptedBundleImport,
                    );
                } else {
                    push_notice(&mut loaded.notices, RecoveryNotice::BundleImportPending);
                }
            }
            BundleIntentAuthority::Ambiguous => push_notice(
                &mut loaded.notices,
                RecoveryNotice::CorruptBundleImportPreserved,
            ),
        }
        Ok(loaded)
    }

    pub(crate) fn write_bundle_import_intent(
        &self,
        intent: &BundleImportIntent,
    ) -> Result<(), StoreError> {
        let bytes = intent
            .encode()
            .map_err(|error| map_bundle_error(Operation::WriteBundleImportIntent, error))?;
        self.backend
            .create_dir_all_private(&self.root)
            .map_err(|error| StoreError::io(Operation::CreateDirectory, error))?;
        self.backend
            .write_atomic_private(&self.bundle_import_path(), &bytes)
            .map_err(|error| StoreError::io(Operation::WriteBundleImportIntent, error))?;
        let readback = self
            .backend
            .read_bounded_no_follow(&self.bundle_import_path(), MAX_BUNDLE_IMPORT_INTENT_BYTES)
            .map_err(|error| StoreError::io(Operation::VerifyBundleImportIntent, error))?;
        if readback != bytes || BundleImportIntent::decode(&readback).ok().as_ref() != Some(intent)
        {
            return Err(StoreError::new(
                Operation::VerifyBundleImportIntent,
                ErrorKind::ReadbackMismatch,
            ));
        }
        Ok(())
    }

    pub(crate) fn require_no_bundle_import_intent(&self) -> Result<(), StoreError> {
        match self
            .backend
            .read_bounded_no_follow(&self.bundle_import_path(), MAX_BUNDLE_IMPORT_INTENT_BYTES)
        {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(StoreError::new(
                Operation::ReadBundleImportIntent,
                ErrorKind::InvalidBundleImport,
            )),
            Err(error) => Err(StoreError::io(Operation::ReadBundleImportIntent, error)),
        }
    }

    pub(crate) fn remove_bundle_import_intent(&self) -> Result<(), StoreError> {
        match self.backend.remove_file_durable(&self.bundle_import_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::io(Operation::RemoveBundleImportIntent, error)),
        }
    }

    pub(crate) fn write_orphan_intent(&self, intent: &OrphanIntent) -> Result<(), StoreError> {
        let bytes = intent.encode();
        self.backend
            .create_dir_all_private(&self.root)
            .map_err(|error| StoreError::io(Operation::CreateDirectory, error))?;
        self.backend
            .write_atomic_private(&self.orphan_path(), &bytes)
            .map_err(|error| StoreError::io(Operation::WriteOrphanCollectionIntent, error))?;
        let readback = self
            .backend
            .read_bounded_no_follow(&self.orphan_path(), MAX_ORPHAN_INTENT_BYTES)
            .map_err(|error| StoreError::io(Operation::VerifyOrphanCollectionIntent, error))?;
        if readback != bytes || OrphanIntent::decode(&readback).ok().as_ref() != Some(intent) {
            return Err(StoreError::new(
                Operation::VerifyOrphanCollectionIntent,
                ErrorKind::ReadbackMismatch,
            ));
        }
        Ok(())
    }

    pub(crate) fn require_no_orphan_intent(&self) -> Result<(), StoreError> {
        match self
            .backend
            .read_bounded_no_follow(&self.orphan_path(), MAX_ORPHAN_INTENT_BYTES)
        {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(StoreError::new(
                Operation::ReadOrphanCollectionIntent,
                ErrorKind::InvalidOrphanCollection,
            )),
            Err(error) => Err(StoreError::io(Operation::ReadOrphanCollectionIntent, error)),
        }
    }

    pub(crate) fn remove_orphan_intent(&self) -> Result<(), StoreError> {
        match self.backend.remove_file_durable(&self.orphan_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::io(
                Operation::RemoveOrphanCollectionIntent,
                error,
            )),
        }
    }

    pub(crate) fn finish_orphan_collection(&self, intent: &OrphanIntent) -> Result<(), StoreError> {
        intent
            .cleanup(&self.root, &self.backend)
            .map_err(|error| map_orphan_error(Operation::VerifyOrphanAttachment, error))?;
        self.remove_orphan_intent()
    }

    pub(crate) fn write_import_intent(&self, intent: &ImportIntent) -> Result<(), StoreError> {
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

    pub(crate) fn require_no_import_intent(&self) -> Result<(), StoreError> {
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

    pub(crate) fn remove_import_intent(&self) -> Result<(), StoreError> {
        match self.backend.remove_file_durable(&self.import_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::io(
                Operation::RemoveAttachmentImportIntent,
                error,
            )),
        }
    }

    pub(crate) fn intent_present(&self, path: &Path, maximum: usize) -> bool {
        match self.backend.read_bounded_no_follow(path, maximum) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Ok(_) | Err(_) => true,
        }
    }

    pub(crate) fn write_purge_intent(&self, intent: &PurgeIntent) -> Result<(), StoreError> {
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

    pub(crate) fn finish_purge(&self, intent: &PurgeIntent) -> Result<(), StoreError> {
        intent
            .cleanup(&self.root, &self.backend)
            .map_err(|error| map_purge_error(Operation::VerifyPurgeAttachment, error))?;
        self.remove_purge_intent()
    }

    pub(crate) fn require_no_purge_intent(&self) -> Result<(), StoreError> {
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

    pub(crate) fn remove_purge_intent(&self) -> Result<(), StoreError> {
        match self.backend.remove_file_durable(&self.purge_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::io(Operation::RemovePurgeIntent, error)),
        }
    }

    pub(crate) fn recover_journal(&self) -> Result<Vec<RecoveryNotice>, StoreError> {
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

    pub(crate) fn preflight(&self, baseline: &Baseline) -> Result<(), StoreError> {
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

    pub(crate) fn require_no_journal(&self) -> Result<(), StoreError> {
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

    pub(crate) fn read_snapshot(
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

    pub(crate) fn restore_primary(&self, bytes: &[u8]) -> Result<(), StoreError> {
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

    pub(crate) fn write_and_verify_snapshot(
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

    pub(crate) fn write_and_verify_raw(
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

    pub(crate) fn remove_journal(&self) -> Result<(), StoreError> {
        match self.backend.remove_file(&self.journal_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::io(Operation::RemoveJournal, error)),
        }
    }

    pub(crate) fn primary_path(&self) -> PathBuf {
        self.root.join("library.bin")
    }

    pub(crate) fn last_good_path(&self) -> PathBuf {
        self.root.join("library.last-good.bin")
    }

    pub(crate) fn journal_path(&self) -> PathBuf {
        self.root.join("library.journal.bin")
    }

    pub(crate) fn purge_path(&self) -> PathBuf {
        self.root.join("library.purge.bin")
    }

    pub(crate) fn import_path(&self) -> PathBuf {
        self.root.join("library.attachment-import.bin")
    }

    pub(crate) fn orphan_path(&self) -> PathBuf {
        self.root.join("library.orphan-collection.bin")
    }

    pub(crate) fn bundle_import_path(&self) -> PathBuf {
        self.root.join("library.bundle-import.bin")
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

pub(crate) fn has_blocking_notice(notices: &[RecoveryNotice]) -> bool {
    notices.iter().any(|notice| {
        matches!(
            notice,
            RecoveryNotice::CorruptJournalPreserved
                | RecoveryNotice::MaintenancePending
                | RecoveryNotice::CorruptPurgePreserved
                | RecoveryNotice::PurgeCleanupPending
                | RecoveryNotice::CorruptAttachmentImportPreserved
                | RecoveryNotice::AttachmentImportPending
                | RecoveryNotice::CorruptOrphanCollectionPreserved
                | RecoveryNotice::OrphanCollectionPending
                | RecoveryNotice::CorruptBundleImportPreserved
                | RecoveryNotice::BundleImportPending
        )
    })
}

fn map_bundle_error(operation: Operation, error: BundleImportError) -> StoreError {
    match error.kind {
        BundleImportErrorKind::Io(kind) => StoreError::new(operation, ErrorKind::Io(kind)),
        BundleImportErrorKind::AttachmentMismatch => {
            StoreError::new(operation, ErrorKind::AttachmentMismatch)
        }
        BundleImportErrorKind::HashMismatch => {
            StoreError::new(operation, ErrorKind::ReadbackMismatch)
        }
        _ => StoreError::new(operation, ErrorKind::InvalidBundleImport),
    }
}

fn bundle_error_from_store(error: StoreError) -> BundleImportError {
    let kind = match error.kind {
        ErrorKind::Io(kind) => BundleImportErrorKind::Io(kind),
        ErrorKind::AttachmentMismatch => BundleImportErrorKind::AttachmentMismatch,
        ErrorKind::ReadbackMismatch => BundleImportErrorKind::HashMismatch,
        _ => BundleImportErrorKind::Malformed,
    };
    BundleImportError {
        operation: BundleImportOperation::StageAttachment,
        kind,
    }
}

fn map_orphan_error(operation: Operation, error: OrphanError) -> StoreError {
    match error {
        OrphanError::InvalidPlan | OrphanError::Malformed => {
            StoreError::new(operation, ErrorKind::InvalidOrphanCollection)
        }
        OrphanError::Io(kind) => StoreError::new(operation, ErrorKind::Io(kind)),
        OrphanError::Remove(kind) => {
            StoreError::new(Operation::RemoveOrphanAttachment, ErrorKind::Io(kind))
        }
        OrphanError::ReadbackMismatch => StoreError::new(operation, ErrorKind::ReadbackMismatch),
        OrphanError::AttachmentMismatch => {
            StoreError::new(operation, ErrorKind::AttachmentMismatch)
        }
    }
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
