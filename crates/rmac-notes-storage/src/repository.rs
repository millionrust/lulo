use std::fmt;
use std::path::Path;

use rmac_notes_store::{
    AttachmentImportPlan, BundleImportPlan, LibrarySnapshot, LibraryTransaction, MutationError,
    OrphanCollectionPlan, PlannedBundleImport, PurgePlan,
};
use rmac_storage::{Backend, FileSystem};

use crate::{
    ExportFailure, ExportFormat, ExportOutcome, LoadedLibrary, NotesLibraryStore,
    PreparedBundleImport, PreparedExportDestination, PreparedImageAttachment, PreparedTextNote,
    RecoveryNotice, StoreError, TextImportError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingReason {
    Store(StoreError),
    AcceptedStateChanged,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PendingCommit {
    candidate: Box<LibrarySnapshot>,
    operation: PendingOperation,
    pub reason: PendingReason,
}

impl fmt::Debug for PendingCommit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingCommit")
            .field("candidate_revision", &self.candidate.revision)
            .field(
                "operation",
                &match self.operation {
                    PendingOperation::Ordinary => "ordinary",
                    PendingOperation::Purge(_) => "purge",
                    PendingOperation::AttachmentImport { .. } => "attachment import",
                    PendingOperation::OrphanCollection(_) => "orphan collection",
                    PendingOperation::BundleImport { .. } => "bundle import",
                },
            )
            .field("reason", &self.reason)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingOperation {
    Ordinary,
    Purge(Box<PurgePlan>),
    OrphanCollection(Box<OrphanCollectionPlan>),
    AttachmentImport {
        plan: Box<AttachmentImportPlan>,
        prepared: Box<PreparedImageAttachment>,
    },
    BundleImport {
        plan: Box<BundleImportPlan>,
        prepared: Box<PreparedBundleImport>,
    },
}

impl PendingCommit {
    pub fn candidate(&self) -> &LibrarySnapshot {
        &self.candidate
    }

    pub fn into_candidate(self) -> LibrarySnapshot {
        *self.candidate
    }

    pub fn purge_plan(&self) -> Option<&PurgePlan> {
        match &self.operation {
            PendingOperation::Purge(plan) => Some(plan),
            PendingOperation::Ordinary
            | PendingOperation::AttachmentImport { .. }
            | PendingOperation::OrphanCollection(_)
            | PendingOperation::BundleImport { .. } => None,
        }
    }

    pub fn attachment_import_plan(&self) -> Option<&AttachmentImportPlan> {
        match &self.operation {
            PendingOperation::AttachmentImport { plan, .. } => Some(plan),
            PendingOperation::Ordinary
            | PendingOperation::Purge(_)
            | PendingOperation::OrphanCollection(_)
            | PendingOperation::BundleImport { .. } => None,
        }
    }

    pub fn orphan_collection_plan(&self) -> Option<&OrphanCollectionPlan> {
        match &self.operation {
            PendingOperation::OrphanCollection(plan) => Some(plan),
            PendingOperation::Ordinary
            | PendingOperation::Purge(_)
            | PendingOperation::AttachmentImport { .. }
            | PendingOperation::BundleImport { .. } => None,
        }
    }

    pub fn bundle_import_plan(&self) -> Option<&BundleImportPlan> {
        match &self.operation {
            PendingOperation::BundleImport { plan, .. } => Some(plan),
            PendingOperation::Ordinary
            | PendingOperation::Purge(_)
            | PendingOperation::OrphanCollection(_)
            | PendingOperation::AttachmentImport { .. } => None,
        }
    }

    fn store(
        candidate: Box<LibrarySnapshot>,
        operation: PendingOperation,
        error: StoreError,
    ) -> Self {
        Self {
            candidate,
            operation,
            reason: PendingReason::Store(error),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitError {
    Mutation(MutationError),
    Pending(PendingCommit),
}

impl fmt::Display for CommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mutation(error) => error.fmt(formatter),
            Self::Pending(pending) => formatter.write_str(match pending.reason {
                PendingReason::Store(_) => {
                    "Notes retained the edit but could not commit it to private storage"
                }
                PendingReason::AcceptedStateChanged => {
                    "The durable Notes library changed while the local edit was pending"
                }
            }),
        }
    }
}

impl std::error::Error for CommitError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptedCommit {
    pub revision: u64,
    pub maintenance_pending: bool,
    pub purge_cleanup_pending: bool,
    pub attachment_import_pending: bool,
    pub orphan_collection_pending: bool,
    pub bundle_import_pending: bool,
    pub recovered_after_error: bool,
}

/// The only application-facing owner of an accepted durable library snapshot.
///
/// Candidates remain provisional until the storage adapter returns verified
/// success. A failed candidate is returned intact as [`PendingCommit`]; retry
/// first runs journal recovery and detects whether the candidate was already
/// committed, rolled back, or displaced by an unrelated durable snapshot.
pub struct AcceptedLibrary<B = FileSystem> {
    store: NotesLibraryStore<B>,
    loaded: LoadedLibrary,
}

impl<B: Backend> AcceptedLibrary<B> {
    pub fn open(store: NotesLibraryStore<B>) -> Result<Self, StoreError> {
        let loaded = store.load()?;
        Ok(Self { store, loaded })
    }

    pub(super) fn from_loaded(store: NotesLibraryStore<B>, loaded: LoadedLibrary) -> Self {
        Self { store, loaded }
    }

    pub fn snapshot(&self) -> &LibrarySnapshot {
        self.loaded.snapshot()
    }

    pub fn recovery_notices(&self) -> &[RecoveryNotice] {
        self.loaded.notices()
    }

    pub fn root(&self) -> &Path {
        self.store.root()
    }

    pub fn begin(&self) -> Result<LibraryTransaction, MutationError> {
        LibraryTransaction::begin(self.snapshot())
    }

    pub fn prepare_image_attachment(
        &self,
        selected_path: &Path,
    ) -> Result<PreparedImageAttachment, StoreError> {
        self.store.prepare_image_attachment(selected_path)
    }

    pub fn prepare_text_note(
        &self,
        selected_path: &Path,
    ) -> Result<PreparedTextNote, TextImportError> {
        self.store.prepare_text_note(selected_path)
    }

    pub fn prepare_bundle_import(
        &self,
        selected_path: &Path,
    ) -> Result<PreparedBundleImport, crate::BundleImportError> {
        self.store.prepare_bundle_import(selected_path)
    }

    pub fn commit(
        &mut self,
        transaction: LibraryTransaction,
    ) -> Result<AcceptedCommit, CommitError> {
        let candidate = transaction.finish().map_err(CommitError::Mutation)?;
        self.commit_candidate(candidate, PendingOperation::Ordinary, false)
            .map_err(CommitError::Pending)
    }

    pub fn commit_purge(
        &mut self,
        transaction: LibraryTransaction,
        plan: PurgePlan,
    ) -> Result<AcceptedCommit, CommitError> {
        let candidate = transaction.finish().map_err(CommitError::Mutation)?;
        self.commit_candidate(candidate, PendingOperation::Purge(Box::new(plan)), false)
            .map_err(CommitError::Pending)
    }

    pub fn commit_attachment_import(
        &mut self,
        transaction: LibraryTransaction,
        plan: AttachmentImportPlan,
        prepared: PreparedImageAttachment,
    ) -> Result<AcceptedCommit, CommitError> {
        let candidate = transaction.finish().map_err(CommitError::Mutation)?;
        self.commit_candidate(
            candidate,
            PendingOperation::AttachmentImport {
                plan: Box::new(plan),
                prepared: Box::new(prepared),
            },
            false,
        )
        .map_err(CommitError::Pending)
    }

    pub fn commit_orphan_collection(
        &mut self,
        transaction: LibraryTransaction,
        plan: OrphanCollectionPlan,
    ) -> Result<AcceptedCommit, CommitError> {
        let candidate = transaction.finish().map_err(CommitError::Mutation)?;
        self.commit_candidate(
            candidate,
            PendingOperation::OrphanCollection(Box::new(plan)),
            false,
        )
        .map_err(CommitError::Pending)
    }

    pub fn commit_bundle_import(
        &mut self,
        planned: PlannedBundleImport,
        prepared: PreparedBundleImport,
    ) -> Result<AcceptedCommit, CommitError> {
        let (candidate, plan) = planned.into_parts();
        self.commit_candidate(
            candidate,
            PendingOperation::BundleImport {
                plan: Box::new(plan),
                prepared: Box::new(prepared),
            },
            false,
        )
        .map_err(CommitError::Pending)
    }

    pub fn retry(&mut self, pending: PendingCommit) -> Result<AcceptedCommit, PendingCommit> {
        let PendingCommit {
            candidate,
            operation,
            ..
        } = pending;
        let reloaded = match self.store.load() {
            Ok(reloaded) => reloaded,
            Err(error) => return Err(PendingCommit::store(candidate, operation, error)),
        };
        if reloaded.snapshot() == candidate.as_ref() {
            let maintenance_pending = has_blocking_maintenance(reloaded.notices());
            let purge_cleanup_pending = has_purge_maintenance(reloaded.notices());
            let attachment_import_pending = has_attachment_maintenance(reloaded.notices());
            let orphan_collection_pending = has_orphan_maintenance(reloaded.notices());
            let bundle_import_pending = has_bundle_maintenance(reloaded.notices());
            self.loaded = reloaded;
            return Ok(AcceptedCommit {
                revision: candidate.revision,
                maintenance_pending,
                purge_cleanup_pending,
                attachment_import_pending,
                orphan_collection_pending,
                bundle_import_pending,
                recovered_after_error: true,
            });
        }
        if reloaded.snapshot() != self.loaded.snapshot() {
            self.loaded = reloaded;
            return Err(PendingCommit {
                candidate,
                operation,
                reason: PendingReason::AcceptedStateChanged,
            });
        }
        self.loaded = reloaded;
        self.commit_candidate(*candidate, operation, true)
    }

    fn commit_candidate(
        &mut self,
        candidate: LibrarySnapshot,
        operation: PendingOperation,
        recovered_after_error: bool,
    ) -> Result<AcceptedCommit, PendingCommit> {
        let outcome = match &operation {
            PendingOperation::Ordinary => self.store.save(&self.loaded, &candidate),
            PendingOperation::Purge(plan) => self.store.save_purge(&self.loaded, &candidate, plan),
            PendingOperation::AttachmentImport { plan, prepared } => self
                .store
                .save_attachment_import(&self.loaded, &candidate, plan, prepared),
            PendingOperation::OrphanCollection(plan) => {
                self.store
                    .save_orphan_collection(&self.loaded, &candidate, plan)
            }
            PendingOperation::BundleImport { plan, prepared } => {
                self.store
                    .save_bundle_import(&self.loaded, &candidate, plan, prepared)
            }
        };
        match outcome {
            Ok(outcome) => {
                let accepted = AcceptedCommit {
                    revision: candidate.revision,
                    maintenance_pending: outcome.maintenance_pending,
                    purge_cleanup_pending: outcome.purge_cleanup_pending,
                    attachment_import_pending: outcome.attachment_import_pending,
                    orphan_collection_pending: outcome.orphan_collection_pending,
                    bundle_import_pending: outcome.bundle_import_pending,
                    recovered_after_error,
                };
                self.loaded = outcome.library;
                Ok(accepted)
            }
            Err(error) => Err(PendingCommit::store(Box::new(candidate), operation, error)),
        }
    }
}

impl AcceptedLibrary<FileSystem> {
    pub fn export(
        &self,
        plan: &rmac_notes_store::ExportPlan,
        format: ExportFormat,
        destination: PreparedExportDestination,
    ) -> Result<ExportOutcome, ExportFailure> {
        self.store.export(&self.loaded, plan, format, destination)
    }
}

fn has_blocking_maintenance(notices: &[RecoveryNotice]) -> bool {
    notices.iter().any(|notice| {
        matches!(
            notice,
            RecoveryNotice::MaintenancePending
                | RecoveryNotice::CorruptJournalPreserved
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

fn has_attachment_maintenance(notices: &[RecoveryNotice]) -> bool {
    notices.iter().any(|notice| {
        matches!(
            notice,
            RecoveryNotice::CorruptAttachmentImportPreserved
                | RecoveryNotice::AttachmentImportPending
        )
    })
}

fn has_purge_maintenance(notices: &[RecoveryNotice]) -> bool {
    notices.iter().any(|notice| {
        matches!(
            notice,
            RecoveryNotice::CorruptPurgePreserved | RecoveryNotice::PurgeCleanupPending
        )
    })
}

fn has_orphan_maintenance(notices: &[RecoveryNotice]) -> bool {
    notices.iter().any(|notice| {
        matches!(
            notice,
            RecoveryNotice::CorruptOrphanCollectionPreserved
                | RecoveryNotice::OrphanCollectionPending
        )
    })
}

fn has_bundle_maintenance(notices: &[RecoveryNotice]) -> bool {
    notices.iter().any(|notice| {
        matches!(
            notice,
            RecoveryNotice::CorruptBundleImportPreserved | RecoveryNotice::BundleImportPending
        )
    })
}
