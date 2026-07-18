use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use rmac_notes_store::{LibrarySnapshot, SortOrder};

use crate::{
    plan_legacy_library, scan_legacy_library, AcceptedLibrary, LegacyScanError,
    MigrationCommitError, MigrationError, MigrationPlan, NotesLibraryStore, RecoveryNotice,
    StoreError, WriterLeaseError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotesPathError {
    MissingHome,
    InvalidHome,
    InvalidDataHome,
    OverlappingRoots,
}

impl fmt::Display for NotesPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingHome => "Notes could not find the user data directory",
            Self::InvalidHome | Self::InvalidDataHome => {
                "Notes requires absolute normalized user data directories"
            }
            Self::OverlappingRoots => {
                "The Notes data library cannot overlap the legacy import directory"
            }
        })
    }
}

impl std::error::Error for NotesPathError {}

#[derive(Clone, PartialEq, Eq)]
pub struct NotesPaths {
    data_root: PathBuf,
    legacy_root: PathBuf,
}

impl fmt::Debug for NotesPaths {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("NotesPaths").finish_non_exhaustive()
    }
}

impl NotesPaths {
    pub fn new(data_root: PathBuf, legacy_root: PathBuf) -> Result<Self, NotesPathError> {
        if !is_normal_absolute(&data_root) {
            return Err(NotesPathError::InvalidDataHome);
        }
        if !is_normal_absolute(&legacy_root) {
            return Err(NotesPathError::InvalidHome);
        }
        if data_root.starts_with(&legacy_root) || legacy_root.starts_with(&data_root) {
            return Err(NotesPathError::OverlappingRoots);
        }
        Ok(Self {
            data_root,
            legacy_root,
        })
    }

    pub fn from_environment(
        xdg_data_home: Option<OsString>,
        home: Option<OsString>,
    ) -> Result<Self, NotesPathError> {
        let home = home
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(NotesPathError::MissingHome)?;
        if !is_normal_absolute(&home) {
            return Err(NotesPathError::InvalidHome);
        }
        let data_home = match xdg_data_home.filter(|value| !value.is_empty()) {
            Some(value) => {
                let value = PathBuf::from(value);
                if !is_normal_absolute(&value) {
                    return Err(NotesPathError::InvalidDataHome);
                }
                value
            }
            None => home.join(".local").join("share"),
        };
        Self::new(
            data_home.join("rmac").join("notes"),
            home.join("Documents").join("rmac-notes"),
        )
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn legacy_root(&self) -> &Path {
        &self.legacy_root
    }
}

pub fn resolve_notes_paths() -> Result<NotesPaths, NotesPathError> {
    NotesPaths::from_environment(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn is_normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::CurDir | Component::ParentDir))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupError {
    Writer(WriterLeaseError),
    Store(StoreError),
    Scan(LegacyScanError),
    Plan(MigrationError),
    Commit(MigrationCommitError),
}

impl fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Writer(error) => error.fmt(formatter),
            Self::Store(error) => error.fmt(formatter),
            Self::Scan(error) => error.fmt(formatter),
            Self::Plan(error) => error.fmt(formatter),
            Self::Commit(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StartupError {}

pub enum NotesStartup {
    Ready(Box<AcceptedLibrary>),
    MigrationReview(Box<MigrationReview>),
}

pub struct MigrationReview {
    store: NotesLibraryStore,
    loaded: crate::LoadedLibrary,
    legacy_root: PathBuf,
    plan: MigrationPlan,
}

impl MigrationReview {
    pub fn plan(&self) -> &MigrationPlan {
        &self.plan
    }

    /// Reread the entire source and commit only if it still exactly matches the
    /// reviewed plan. The legacy directory remains untouched on every outcome.
    pub fn accept(self: Box<Self>) -> Result<AcceptedLibrary, StartupError> {
        let reread = scan_legacy_library(&self.legacy_root).map_err(StartupError::Scan)?;
        let outcome = self
            .store
            .commit_legacy_migration(&self.loaded, &reread, &self.plan)
            .map_err(StartupError::Commit)?;
        Ok(AcceptedLibrary::from_loaded(self.store, outcome.library))
    }

    /// Start an empty library for this session without writing a suppression
    /// marker. If it remains empty, the migration review is offered again on
    /// the next launch.
    pub fn start_empty(self: Box<Self>) -> AcceptedLibrary {
        AcceptedLibrary::from_loaded(self.store, self.loaded)
    }
}

pub fn inspect_notes_startup(paths: &NotesPaths) -> Result<NotesStartup, StartupError> {
    let store =
        NotesLibraryStore::new(paths.data_root().to_path_buf()).map_err(StartupError::Writer)?;
    let loaded = store.load().map_err(StartupError::Store)?;
    let storage_needs_attention = loaded.notices().iter().any(|notice| {
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
    });
    if loaded.snapshot() != &LibrarySnapshot::default() || storage_needs_attention {
        return Ok(NotesStartup::Ready(Box::new(AcceptedLibrary::from_loaded(
            store, loaded,
        ))));
    }

    match std::fs::symlink_metadata(paths.legacy_root()) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(NotesStartup::Ready(Box::new(AcceptedLibrary::from_loaded(
                store, loaded,
            ))));
        }
        _ => {}
    }
    let input = scan_legacy_library(paths.legacy_root()).map_err(StartupError::Scan)?;
    let source_is_empty = input.folder_names.is_empty()
        && input.notes.is_empty()
        && input.attachments.is_empty()
        && input.pinned_note_paths.is_empty()
        && input.sort_order == SortOrder::Edited;
    if source_is_empty {
        return Ok(NotesStartup::Ready(Box::new(AcceptedLibrary::from_loaded(
            store, loaded,
        ))));
    }
    let plan = plan_legacy_library(input).map_err(StartupError::Plan)?;
    Ok(NotesStartup::MigrationReview(Box::new(MigrationReview {
        store,
        loaded,
        legacy_root: paths.legacy_root().to_path_buf(),
        plan,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MigrationCommitErrorKind, WriterLeaseErrorKind};
    use rmac_notes_store::{LibraryTransaction, NewNote};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn roots(label: &str) -> (PathBuf, NotesPaths) {
        let container = std::env::temp_dir().join(format!(
            "rmac-notes-startup-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let paths = NotesPaths::new(container.join("data"), container.join("legacy")).unwrap();
        (container, paths)
    }

    fn write_legacy(paths: &NotesPaths) {
        std::fs::create_dir_all(paths.legacy_root().join("Projects")).unwrap();
        std::fs::create_dir(paths.legacy_root().join("Empty")).unwrap();
        std::fs::write(
            paths.legacy_root().join("Projects/roadmap.md"),
            b"Roadmap\nMigration",
        )
        .unwrap();
        std::fs::write(paths.legacy_root().join(".sort"), b"title").unwrap();
    }

    #[test]
    fn environment_paths_never_fall_back_to_the_working_directory() {
        let paths = NotesPaths::from_environment(None, Some(OsString::from("/home/alex"))).unwrap();
        assert_eq!(
            paths.data_root(),
            Path::new("/home/alex/.local/share/rmac/notes")
        );
        assert_eq!(
            paths.legacy_root(),
            Path::new("/home/alex/Documents/rmac-notes")
        );

        let xdg = NotesPaths::from_environment(
            Some(OsString::from("/data/alex")),
            Some(OsString::from("/home/alex")),
        )
        .unwrap();
        assert_eq!(xdg.data_root(), Path::new("/data/alex/rmac/notes"));
        assert_eq!(
            NotesPaths::from_environment(None, None),
            Err(NotesPathError::MissingHome)
        );
        assert_eq!(
            NotesPaths::from_environment(
                Some(OsString::from("relative")),
                Some(OsString::from("/home/alex"))
            ),
            Err(NotesPathError::InvalidDataHome)
        );
    }

    #[test]
    fn overlapping_or_non_normal_roots_are_rejected() {
        assert_eq!(
            NotesPaths::new(PathBuf::from("/tmp/a/data"), PathBuf::from("/tmp/a")),
            Err(NotesPathError::OverlappingRoots)
        );
        assert_eq!(
            NotesPaths::new(
                PathBuf::from("/tmp/a/../data"),
                PathBuf::from("/tmp/legacy")
            ),
            Err(NotesPathError::InvalidDataHome)
        );
    }

    #[test]
    fn first_run_reviews_then_commits_an_exact_fresh_reread() {
        let (container, paths) = roots("accept");
        write_legacy(&paths);
        let NotesStartup::MigrationReview(review) = inspect_notes_startup(&paths).unwrap() else {
            panic!("legacy content must require review");
        };
        assert_eq!(review.plan().snapshot.notes.len(), 1);
        assert!(review
            .plan()
            .snapshot
            .folders
            .iter()
            .any(|folder| folder.name == "Empty"));

        let accepted = review.accept().unwrap();

        assert_eq!(accepted.snapshot().notes[0].title, "Roadmap");
        assert!(paths.legacy_root().join("Projects/roadmap.md").exists());
        drop(accepted);
        let NotesStartup::Ready(reopened) = inspect_notes_startup(&paths).unwrap() else {
            panic!("a committed migration must not be offered again");
        };
        assert_eq!(reopened.snapshot().notes.len(), 1);
        drop(reopened);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn source_change_after_review_cannot_publish_metadata() {
        let (container, paths) = roots("changed");
        write_legacy(&paths);
        let NotesStartup::MigrationReview(review) = inspect_notes_startup(&paths).unwrap() else {
            panic!("legacy content must require review");
        };
        std::fs::write(
            paths.legacy_root().join("Projects/roadmap.md"),
            b"Changed after review",
        )
        .unwrap();

        let error = match review.accept() {
            Ok(_) => panic!("changed source must not be committed"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            StartupError::Commit(MigrationCommitError {
                kind: MigrationCommitErrorKind::PlanMismatch,
                ..
            })
        ));
        assert!(!paths.data_root().join("library.bin").exists());
        assert_eq!(
            std::fs::read(paths.legacy_root().join("Projects/roadmap.md")).unwrap(),
            b"Changed after review"
        );
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn decline_is_session_only_and_writer_contention_fails_immediately() {
        let (container, paths) = roots("decline");
        write_legacy(&paths);
        let NotesStartup::MigrationReview(review) = inspect_notes_startup(&paths).unwrap() else {
            panic!("legacy content must require review");
        };
        let empty = review.start_empty();
        assert_eq!(empty.snapshot(), &LibrarySnapshot::default());

        let error = match inspect_notes_startup(&paths) {
            Ok(_) => panic!("the first session still owns the writer lease"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StartupError::Writer(WriterLeaseError {
                kind: WriterLeaseErrorKind::Contended,
                ..
            })
        ));
        drop(empty);

        let NotesStartup::MigrationReview(review) = inspect_notes_startup(&paths).unwrap() else {
            panic!("decline must not silently suppress a later review");
        };
        let mut empty = review.start_empty();
        let mut transaction = LibraryTransaction::begin(empty.snapshot()).unwrap();
        transaction
            .create_note(NewNote {
                created_unix_ms: 1,
                title: "New local library".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id: None,
            })
            .unwrap();
        empty.commit(transaction).unwrap();
        drop(empty);

        let NotesStartup::Ready(ready) = inspect_notes_startup(&paths).unwrap() else {
            panic!("a nonempty accepted library must remain authoritative");
        };
        assert_eq!(ready.snapshot().notes[0].title, "New local library");
        drop(ready);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn missing_or_empty_legacy_root_starts_ready() {
        let (container, paths) = roots("empty");
        let NotesStartup::Ready(missing) = inspect_notes_startup(&paths).unwrap() else {
            panic!("missing legacy root is a normal first run");
        };
        drop(missing);
        std::fs::create_dir(paths.legacy_root()).unwrap();
        let NotesStartup::Ready(empty) = inspect_notes_startup(&paths).unwrap() else {
            panic!("empty legacy root needs no migration review");
        };
        drop(empty);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn blocking_storage_recovery_notice_takes_priority_over_migration() {
        let (container, paths) = roots("storage-attention");
        write_legacy(&paths);
        std::fs::create_dir_all(paths.data_root()).unwrap();
        std::fs::write(paths.data_root().join("library.journal.bin"), b"malformed").unwrap();

        let NotesStartup::Ready(ready) = inspect_notes_startup(&paths).unwrap() else {
            panic!("a blocked store must not offer a migration it cannot commit");
        };

        assert!(ready
            .recovery_notices()
            .contains(&RecoveryNotice::CorruptJournalPreserved));
        assert_eq!(ready.snapshot(), &LibrarySnapshot::default());
        drop(ready);
        std::fs::remove_dir_all(container).unwrap();
    }

    #[test]
    fn blocked_bundle_import_recovery_takes_priority_over_legacy_migration() {
        let (container, paths) = roots("bundle-import-attention");
        write_legacy(&paths);
        std::fs::create_dir_all(paths.data_root()).unwrap();
        std::fs::write(
            paths.data_root().join("library.bundle-import.bin"),
            b"malformed",
        )
        .unwrap();

        let NotesStartup::Ready(ready) = inspect_notes_startup(&paths).unwrap() else {
            panic!("blocked bundle recovery must not offer an uncommittable migration");
        };

        assert!(ready
            .recovery_notices()
            .contains(&RecoveryNotice::CorruptBundleImportPreserved));
        assert_eq!(ready.snapshot(), &LibrarySnapshot::default());
        drop(ready);
        std::fs::remove_dir_all(container).unwrap();
    }
}
