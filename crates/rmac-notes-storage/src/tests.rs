use super::*;
use crate::journal::{digest, Journal, JOURNAL_MAGIC, JOURNAL_VERSION};
use crate::model::Baseline;
use image::ImageEncoder as _;
use rmac_notes_store::{
    encode, AttachmentId, AttachmentImportPlan, AttachmentKind, AttachmentRecord, LibrarySnapshot,
    LibraryTransaction, NewNote, NoteId, NoteRecord, SortOrder,
};
use rmac_storage::Backend;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

fn orphan_fixture() -> (LibrarySnapshot, PathBuf, Vec<u8>) {
    let note_id = NoteId::new(1).unwrap();
    let attachment_id = AttachmentId::new(1).unwrap();
    let attachment_bytes = b"exact private orphan attachment".to_vec();
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
                title: "Private live note".into(),
                body: "Private body".into(),
                tags: Vec::new(),
                folder_id: None,
                pinned: false,
                deleted: false,
                attachments: Vec::new(),
            }],
            attachments: vec![AttachmentRecord {
                id: attachment_id,
                revision: 2,
                note_id,
                display_name: "private-orphan.png".into(),
                kind: AttachmentKind::Png,
                byte_len: attachment_bytes.len() as u64,
                sha256: digest(&attachment_bytes),
                deleted: true,
            }],
        },
        PathBuf::from("/virtual/library/attachments/00000000000000000001.bin"),
        attachment_bytes,
    )
}

fn install_orphan_base(
    store: &NotesLibraryStore<FakeBackend>,
    backend: &FakeBackend,
) -> (LoadedLibrary, LibrarySnapshot, PathBuf, Vec<u8>) {
    let initial = store.load().unwrap();
    let (base, attachment_path, attachment_bytes) = orphan_fixture();
    base.validate().unwrap();
    let loaded = store.save(&initial, &base).unwrap().library;
    backend.set(attachment_path.clone(), attachment_bytes.clone());
    (loaded, base, attachment_path, attachment_bytes)
}

fn orphan_candidate(
    base: &LibrarySnapshot,
) -> (LibrarySnapshot, rmac_notes_store::OrphanCollectionPlan) {
    let mut transaction = LibraryTransaction::begin(base).unwrap();
    let plan = transaction
        .collect_orphaned_attachment(AttachmentId::new(1).unwrap(), 2)
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
fn managed_preview_verifies_identity_and_returns_bounded_rgba() {
    let (store, backend) = store();
    let selected = PathBuf::from("/portal/private-plan.png");
    let bytes = png_bytes();
    backend.set(selected.clone(), bytes.clone());
    let prepared = store.prepare_image_attachment(&selected).unwrap();
    let (loaded, base, note_id) = install_attachment_base(&store);
    let (candidate, plan) = attachment_candidate(&base, note_id, &prepared);
    store
        .save_attachment_import(&loaded, &candidate, &plan, &prepared)
        .unwrap();
    let target = PreviewSize::new(512, 512).unwrap();

    let preview = attachment::load_managed_image_preview_with_backend(
        store.root(),
        &candidate.attachments[0],
        target,
        &backend,
    )
    .unwrap();

    assert_eq!(preview.attachment_id(), plan.attachment_id);
    assert_eq!((preview.width(), preview.height()), (1, 1));
    assert_eq!(preview.rgba().as_ref(), &[12, 34, 56, 255]);
    let debug = format!("{preview:?}");
    assert!(!debug.contains("private-plan"));
    assert!(!debug.contains("12, 34"));

    let managed = managed_attachment_path(store.root(), plan.attachment_id);
    backend.set(managed, b"changed managed image".to_vec());
    assert_eq!(
        attachment::load_managed_image_preview_with_backend(
            store.root(),
            &candidate.attachments[0],
            target,
            &backend,
        ),
        Err(PreviewError::Changed)
    );
    assert_eq!(
        PreviewSize::new(MAX_PREVIEW_DIMENSION, MAX_PREVIEW_DIMENSION),
        Err(PreviewError::InvalidRequest)
    );
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
                bytes: png_bytes(),
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
    let (loaded, base, attachment_path, attachment_bytes) = install_purge_base(&store, &backend);
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
    let (loaded, base, attachment_path, attachment_bytes) = install_purge_base(&store, &backend);
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
    let (loaded, base, attachment_path, attachment_bytes) = install_purge_base(&store, &backend);
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
fn accepted_orphan_collection_removes_exact_bytes_after_metadata() {
    let (store, backend) = store();
    let (loaded, base, attachment_path, _) = install_orphan_base(&store, &backend);
    let (collected, plan) = orphan_candidate(&base);

    let outcome = store
        .save_orphan_collection(&loaded, &collected, &plan)
        .unwrap();

    assert_eq!(outcome.library.snapshot(), &collected);
    assert!(!outcome.maintenance_pending);
    assert!(!outcome.orphan_collection_pending);
    assert_eq!(backend.get(&attachment_path), None);
    assert_eq!(backend.get(&store.orphan_path()), None);
}

#[test]
fn rolled_back_orphan_metadata_never_deletes_managed_bytes() {
    let (store, backend) = store();
    let (loaded, base, attachment_path, attachment_bytes) = install_orphan_base(&store, &backend);
    let (collected, plan) = orphan_candidate(&base);
    backend.fail_next_write(store.primary_path());

    assert_eq!(
        store
            .save_orphan_collection(&loaded, &collected, &plan)
            .unwrap_err()
            .operation,
        Operation::WritePrimary
    );
    assert_eq!(
        backend.get(&attachment_path),
        Some(attachment_bytes.clone())
    );
    assert!(backend.get(&store.orphan_path()).is_some());

    let recovered = store.load().unwrap();
    assert_eq!(recovered.snapshot(), &base);
    assert!(recovered
        .notices()
        .contains(&RecoveryNotice::RolledBackInterruptedOrphanCollection));
    assert_eq!(backend.get(&attachment_path), Some(attachment_bytes));
    assert_eq!(backend.get(&store.orphan_path()), None);
}

#[test]
fn orphan_collection_waits_for_metadata_maintenance_then_recovers() {
    let (store, backend) = store();
    let (loaded, base, attachment_path, attachment_bytes) = install_orphan_base(&store, &backend);
    let (collected, plan) = orphan_candidate(&base);
    backend.fail_next_write(store.last_good_path());

    let outcome = store
        .save_orphan_collection(&loaded, &collected, &plan)
        .unwrap();
    assert!(outcome.maintenance_pending);
    assert!(outcome.orphan_collection_pending);
    assert_eq!(backend.get(&attachment_path), Some(attachment_bytes));

    let recovered = store.load().unwrap();
    assert_eq!(recovered.snapshot(), &collected);
    assert!(recovered
        .notices()
        .contains(&RecoveryNotice::FinishedInterruptedOrphanCollection));
    assert_eq!(backend.get(&attachment_path), None);
    assert_eq!(backend.get(&store.orphan_path()), None);
}

#[test]
fn changed_orphan_bytes_are_preserved_and_block_writes() {
    let (store, backend) = store();
    let (loaded, base, attachment_path, _) = install_orphan_base(&store, &backend);
    let (collected, plan) = orphan_candidate(&base);
    let substituted = b"substituted private orphan".to_vec();
    backend.set(attachment_path.clone(), substituted.clone());

    let outcome = store
        .save_orphan_collection(&loaded, &collected, &plan)
        .unwrap();
    assert!(outcome.maintenance_pending);
    assert!(outcome.orphan_collection_pending);
    assert_eq!(backend.get(&attachment_path), Some(substituted.clone()));

    let reopened = store.load().unwrap();
    assert!(reopened
        .notices()
        .contains(&RecoveryNotice::OrphanCollectionPending));
    assert_eq!(backend.get(&attachment_path), Some(substituted));
    let later = candidate(&collected, "must remain blocked");
    assert_eq!(
        store.save(&reopened, &later).unwrap_err().kind,
        ErrorKind::InvalidOrphanCollection
    );
}

#[test]
fn orphan_remove_failure_and_missing_after_acceptance_resume_idempotently() {
    let (store, backend) = store();
    let (loaded, base, attachment_path, _) = install_orphan_base(&store, &backend);
    let (collected, plan) = orphan_candidate(&base);
    backend.fail_next_remove(attachment_path.clone());

    let outcome = store
        .save_orphan_collection(&loaded, &collected, &plan)
        .unwrap();
    assert!(outcome.orphan_collection_pending);
    backend.0.lock().unwrap().files.remove(&attachment_path);

    let recovered = store.load().unwrap();
    assert_eq!(recovered.snapshot(), &collected);
    assert!(recovered
        .notices()
        .contains(&RecoveryNotice::FinishedInterruptedOrphanCollection));
    assert_eq!(backend.get(&store.orphan_path()), None);
}

#[test]
fn malformed_or_multiple_cleanup_intents_are_preserved_and_blocked() {
    let (store, backend) = store();
    backend.set(store.orphan_path(), b"malformed orphan intent".to_vec());

    let orphan_only = store.load().unwrap();
    assert!(orphan_only
        .notices()
        .contains(&RecoveryNotice::CorruptOrphanCollectionPreserved));
    let next = candidate(orphan_only.snapshot(), "blocked");
    assert_eq!(
        store.save(&orphan_only, &next).unwrap_err().kind,
        ErrorKind::InvalidOrphanCollection
    );

    backend.set(store.purge_path(), b"malformed purge intent".to_vec());
    let multiple = store.load().unwrap();
    assert!(multiple
        .notices()
        .contains(&RecoveryNotice::CorruptOrphanCollectionPreserved));
    assert!(multiple
        .notices()
        .contains(&RecoveryNotice::CorruptPurgePreserved));
    assert!(backend.get(&store.orphan_path()).is_some());
    assert!(backend.get(&store.purge_path()).is_some());
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
    let (loaded, _base, attachment_path, attachment_bytes) = install_purge_base(&store, &backend);
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
fn repository_retry_retains_orphan_plan_and_collects_only_after_acceptance() {
    let (store, backend) = store();
    let (loaded, _base, attachment_path, attachment_bytes) = install_orphan_base(&store, &backend);
    let primary = store.primary_path();
    let mut repository = AcceptedLibrary::from_loaded(store, loaded);
    let mut transaction = repository.begin().unwrap();
    let plan = transaction
        .collect_orphaned_attachment(AttachmentId::new(1).unwrap(), 2)
        .unwrap();
    backend.fail_next_write(primary);

    let CommitError::Pending(pending) = repository
        .commit_orphan_collection(transaction, plan.clone())
        .unwrap_err()
    else {
        panic!("expected retained orphan-collection candidate")
    };
    assert_eq!(pending.orphan_collection_plan(), Some(&plan));
    assert_eq!(backend.get(&attachment_path), Some(attachment_bytes));
    assert!(!format!("{pending:?}").contains("["));

    let accepted = repository.retry(pending).unwrap();
    assert!(accepted.recovered_after_error);
    assert!(!accepted.orphan_collection_pending);
    assert_eq!(backend.get(&attachment_path), None);
    assert!(repository.snapshot().attachments.is_empty());
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
