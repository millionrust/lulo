use std::fmt;
use std::path::Path;

use rmac_notes_store::{LibrarySnapshot, LibraryTransaction, MutationError};
use rmac_storage::{Backend, FileSystem};

use crate::{LoadedLibrary, NotesLibraryStore, RecoveryNotice, StoreError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingReason {
    Store(StoreError),
    AcceptedStateChanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCommit {
    candidate: LibrarySnapshot,
    pub reason: PendingReason,
}

impl PendingCommit {
    pub fn candidate(&self) -> &LibrarySnapshot {
        &self.candidate
    }

    pub fn into_candidate(self) -> LibrarySnapshot {
        self.candidate
    }

    fn store(candidate: LibrarySnapshot, error: StoreError) -> Self {
        Self {
            candidate,
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

    pub fn commit(
        &mut self,
        transaction: LibraryTransaction,
    ) -> Result<AcceptedCommit, CommitError> {
        let candidate = transaction.finish().map_err(CommitError::Mutation)?;
        self.commit_candidate(candidate, false)
            .map_err(CommitError::Pending)
    }

    pub fn retry(&mut self, pending: PendingCommit) -> Result<AcceptedCommit, PendingCommit> {
        let candidate = pending.candidate;
        let reloaded = match self.store.load() {
            Ok(reloaded) => reloaded,
            Err(error) => return Err(PendingCommit::store(candidate, error)),
        };
        if reloaded.snapshot() == &candidate {
            let maintenance_pending = reloaded.notices().iter().any(|notice| {
                matches!(
                    notice,
                    RecoveryNotice::MaintenancePending | RecoveryNotice::CorruptJournalPreserved
                )
            });
            self.loaded = reloaded;
            return Ok(AcceptedCommit {
                revision: candidate.revision,
                maintenance_pending,
                recovered_after_error: true,
            });
        }
        if reloaded.snapshot() != self.loaded.snapshot() {
            self.loaded = reloaded;
            return Err(PendingCommit {
                candidate,
                reason: PendingReason::AcceptedStateChanged,
            });
        }
        self.loaded = reloaded;
        self.commit_candidate(candidate, true)
    }

    fn commit_candidate(
        &mut self,
        candidate: LibrarySnapshot,
        recovered_after_error: bool,
    ) -> Result<AcceptedCommit, PendingCommit> {
        match self.store.save(&self.loaded, &candidate) {
            Ok(outcome) => {
                let accepted = AcceptedCommit {
                    revision: candidate.revision,
                    maintenance_pending: outcome.maintenance_pending,
                    recovered_after_error,
                };
                self.loaded = outcome.library;
                Ok(accepted)
            }
            Err(error) => Err(PendingCommit::store(candidate, error)),
        }
    }
}
