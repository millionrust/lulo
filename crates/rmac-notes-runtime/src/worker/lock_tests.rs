//! End-to-end locked-note tests through the real worker and file store.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::ImageEncoder as _;
use rmac_notes_storage::{load_managed_image_preview_with_key, PreviewError, PreviewSize};
use rmac_notes_store::{LockKdfParams, NewNote, NoteChanges};

use super::*;
use crate::{NotesSearchIndex, SearchCancellation, SearchGeneration, SearchRequest};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Cheap Argon2id parameters so tests stay fast; the shipped cost is
/// covered by `rmac-notes-store`'s timing test.
const FAST_KDF: LockKdfParams = LockKdfParams {
    memory_kib: 64,
    iterations: 1,
    parallelism: 1,
};

const SECRET_TITLE: &str = "Quarterly bank codes";
const SECRET_BODY: &str = "vault combination 4417-zebra";
const SECRET_TAG: &str = "privatetagxyz";
const SECRET_IMAGE_STEM: &str = "secret-scan-qq";

fn roots(label: &str) -> (PathBuf, NotesPaths) {
    let container = std::env::temp_dir().join(format!(
        "rmac-notes-lock-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let paths = NotesPaths::new(container.join("data"), container.join("legacy")).unwrap();
    (container, paths)
}

fn start(paths: &NotesPaths, keyring: &NotesKeyring) -> NotesWorker {
    NotesWorker::start_with_options(
        paths.clone(),
        Duration::from_millis(10),
        keyring.clone(),
        FAST_KDF,
    )
    .unwrap()
}

fn next(worker: &NotesWorker) -> WorkerEvent {
    worker.recv_timeout(Duration::from_secs(10)).unwrap()
}

fn ready(worker: &NotesWorker) -> SnapshotEvent {
    match next(worker) {
        WorkerEvent::Ready(snapshot) => snapshot,
        event => panic!("expected ready, got {event:?}"),
    }
}

fn apply(worker: &NotesWorker, request_id: u64, action: LibraryAction) -> WorkerEvent {
    worker
        .try_send(WorkerCommand::Apply(
            ActionRequest::new(request_id, action).unwrap(),
        ))
        .unwrap();
    next(worker)
}

fn accepted(event: WorkerEvent) -> AcceptedEvent {
    match event {
        WorkerEvent::Accepted(event) => event,
        event => panic!("expected accepted, got {event:?}"),
    }
}

fn rejected(event: WorkerEvent) -> WorkerFailure {
    match event {
        WorkerEvent::Rejected(event) => event.failure,
        event => panic!("expected rejected, got {event:?}"),
    }
}

fn lock_state(event: WorkerEvent) -> LockStateEvent {
    match event {
        WorkerEvent::LockStateChanged(event) => event,
        event => panic!("expected lock state, got {event:?}"),
    }
}

fn secret(value: &str) -> LockSecret {
    LockSecret::new(value.to_owned())
}

fn create_secret_note(worker: &NotesWorker, request_id: u64) -> NoteId {
    let event = accepted(apply(
        worker,
        request_id,
        LibraryAction::CreateNote(NewNote {
            created_unix_ms: 10,
            title: SECRET_TITLE.into(),
            body: SECRET_BODY.into(),
            tags: vec![SECRET_TAG.into()],
            folder_id: None,
        }),
    ));
    match event.result {
        ActionResult::CreatedNote(note_id) => note_id,
        result => panic!("unexpected {result:?}"),
    }
}

fn tiny_png(pixel: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&pixel, 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}

fn note_revision(snapshot: &LibrarySnapshot, note_id: NoteId) -> u64 {
    snapshot
        .notes
        .iter()
        .find(|note| note.id == note_id)
        .unwrap()
        .revision
}

/// Every byte of every file the store owns, for plaintext scans.
fn store_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                files.push((path, bytes));
            }
        }
    }
    files
}

fn assert_absent_from_store(root: &Path, needle: &[u8], label: &str) {
    for (path, bytes) in store_files(root) {
        assert!(
            !bytes.windows(needle.len()).any(|window| window == needle),
            "{label} found in plaintext at {}",
            path.display()
        );
    }
}

fn assert_present_in_store(root: &Path, needle: &[u8]) -> bool {
    store_files(root)
        .iter()
        .any(|(_, bytes)| bytes.windows(needle.len()).any(|window| window == needle))
}

#[test]
fn locking_leaves_no_plaintext_and_round_trips_across_launches() {
    let (container, paths) = roots("round-trip");
    std::fs::create_dir_all(&container).unwrap();
    let image = tiny_png([12, 34, 56, 255]);
    let source = container.join(format!("{SECRET_IMAGE_STEM}.png"));
    std::fs::write(&source, &image).unwrap();
    let keyring = NotesKeyring::new();
    let worker = start(&paths, &keyring);
    ready(&worker);
    let note_id = create_secret_note(&worker, 1);
    let attached = accepted(apply(
        &worker,
        2,
        LibraryAction::AttachImage {
            note_id,
            expected_revision: 1,
            modified_unix_ms: 11,
            selected_path: source.clone(),
        },
    ));
    let revision = note_revision(&attached.accepted.snapshot, note_id);
    // Migration: an existing plaintext note stays plaintext until locked.
    assert!(assert_present_in_store(
        paths.data_root(),
        SECRET_BODY.as_bytes()
    ));

    let locked = accepted(apply(
        &worker,
        3,
        LibraryAction::LockNote {
            note_id,
            expected_revision: revision,
            credential: LockCredential::NewPassword {
                password: secret("correct horse"),
                hint: "the stable".into(),
            },
        },
    ));
    assert_eq!(locked.result, ActionResult::LockedNote(note_id));
    // The note stays open after locking, as on macOS.
    assert!(locked.accepted.open_notes.contains(&note_id));
    let view = &locked.accepted.snapshot;
    let note = view.notes.iter().find(|note| note.id == note_id).unwrap();
    assert!(note.lock.is_some());
    assert_eq!(note.body, SECRET_BODY);
    let attachment = view
        .attachments
        .iter()
        .find(|attachment| attachment.note_id == note_id)
        .unwrap()
        .clone();
    assert_eq!(attachment.display_name, format!("{SECRET_IMAGE_STEM}.png"));
    assert!(attachment.sealed_key.is_some());

    // Nothing of the note survives in plaintext anywhere in the store.
    let root = paths.data_root();
    assert_absent_from_store(root, SECRET_TITLE.as_bytes(), "title");
    assert_absent_from_store(root, SECRET_BODY.as_bytes(), "body");
    assert_absent_from_store(root, SECRET_TAG.as_bytes(), "tag");
    assert_absent_from_store(root, SECRET_IMAGE_STEM.as_bytes(), "attachment name");
    assert_absent_from_store(root, &image, "attachment bytes");
    assert_absent_from_store(root, b"correct horse", "password");

    // An open locked note's sealed image previews with its key, and never
    // without it.
    let key = keyring.get(attachment.sealed_key.unwrap()).unwrap();
    let preview = load_managed_image_preview_with_key(
        root,
        &attachment,
        PreviewSize::new(64, 64).unwrap(),
        Some(&key),
    )
    .unwrap();
    assert_eq!(preview.rgba().as_ref(), &[12, 34, 56, 255]);
    drop(key);
    assert_eq!(
        load_managed_image_preview_with_key(
            root,
            &attachment,
            PreviewSize::new(64, 64).unwrap(),
            None
        ),
        Err(PreviewError::Locked)
    );

    // Edits of an open locked note are sealed too and never drafted.
    let edited_body = "edited secret paragraph 9931";
    worker
        .try_send(WorkerCommand::ScheduleEdit(
            ScheduledEdit::new(
                4,
                EditGeneration::new(1).unwrap(),
                note_id,
                note_revision(view, note_id),
                NoteChanges {
                    modified_unix_ms: 20,
                    title: SECRET_TITLE.into(),
                    body: edited_body.into(),
                    tags: vec![SECRET_TAG.into()],
                },
            )
            .unwrap(),
        ))
        .unwrap();
    let edited = accepted(next(&worker));
    assert_eq!(edited.result, ActionResult::Edited(note_id));
    let note = edited
        .accepted
        .snapshot
        .notes
        .iter()
        .find(|note| note.id == note_id)
        .unwrap()
        .clone();
    assert_eq!(note.body, edited_body);
    assert_absent_from_store(root, edited_body.as_bytes(), "edited body");

    // A new image on an open locked note is sealed straight away.
    let second = tiny_png([200, 1, 2, 255]);
    let second_source = container.join("second-private-zz.png");
    std::fs::write(&second_source, &second).unwrap();
    let attached = accepted(apply(
        &worker,
        5,
        LibraryAction::AttachImage {
            note_id,
            expected_revision: note.revision,
            modified_unix_ms: 21,
            selected_path: second_source,
        },
    ));
    assert!(matches!(
        attached.result,
        ActionResult::AttachedImage { .. }
    ));
    let sealed_again = accepted(next(&worker));
    assert_eq!(sealed_again.result, ActionResult::Changed);
    assert!(sealed_again
        .accepted
        .snapshot
        .attachments
        .iter()
        .filter(|attachment| attachment.note_id == note_id)
        .all(|attachment| attachment.sealed_key.is_some()));
    assert_absent_from_store(root, b"second-private-zz", "second name");
    assert_absent_from_store(root, &second, "second bytes");

    // Close All Locked Notes: the view no longer carries the content.
    worker
        .try_send(WorkerCommand::CloseLockedNotes { request_id: 6 })
        .unwrap();
    let closed = lock_state(next(&worker));
    assert!(closed.accepted.open_notes.is_empty());
    assert!(keyring.is_empty());
    let note = closed
        .accepted
        .snapshot
        .notes
        .iter()
        .find(|note| note.id == note_id)
        .unwrap();
    assert!(note.title.is_empty() && note.body.is_empty() && note.tags.is_empty());

    // Relaunch: the lock, the verifier and the hint persist.
    drop(worker);
    let keyring = NotesKeyring::new();
    let worker = start(&paths, &keyring);
    let relaunched = ready(&worker);
    assert!(relaunched.open_notes.is_empty());
    let current = relaunched.snapshot.current_lock_key.unwrap();
    assert_eq!(
        relaunched.snapshot.lock_key(current).unwrap().hint,
        "the stable"
    );
    worker
        .try_send(WorkerCommand::UnlockNotes {
            request_id: 7,
            note_id,
            password: secret("wrong horse"),
        })
        .unwrap();
    assert_eq!(
        rejected(next(&worker)),
        WorkerFailure::Lock(LockError::WrongPassword)
    );
    assert!(keyring.is_empty());
    worker
        .try_send(WorkerCommand::UnlockNotes {
            request_id: 8,
            note_id,
            password: secret("correct horse"),
        })
        .unwrap();
    let unlocked = lock_state(next(&worker));
    assert_eq!(unlocked.unlocked_note, Some(note_id));
    let note = unlocked
        .accepted
        .snapshot
        .notes
        .iter()
        .find(|note| note.id == note_id)
        .unwrap()
        .clone();
    assert_eq!(note.body, edited_body);
    assert_eq!(note.tags, vec![SECRET_TAG.to_owned()]);

    // Remove Lock: plaintext again, attachments decrypted in place.
    let removed = accepted(apply(
        &worker,
        9,
        LibraryAction::RemoveNoteLock {
            note_id,
            expected_revision: note.revision,
            password: None,
        },
    ));
    assert_eq!(removed.result, ActionResult::RemovedLock(note_id));
    let view = &removed.accepted.snapshot;
    assert!(view.notes.iter().all(|note| note.lock.is_none()));
    assert!(view
        .attachments
        .iter()
        .all(|attachment| attachment.sealed_key.is_none()));
    assert!(view
        .attachments
        .iter()
        .any(|attachment| attachment.display_name == format!("{SECRET_IMAGE_STEM}.png")));
    assert!(assert_present_in_store(root, &image));
    assert!(assert_present_in_store(root, edited_body.as_bytes()));

    drop(worker);
    std::fs::remove_dir_all(container).unwrap();
}

#[test]
fn tampered_ciphertext_is_detected_and_never_opens() {
    let (container, paths) = roots("tamper");
    let keyring = NotesKeyring::new();
    let worker = start(&paths, &keyring);
    ready(&worker);
    let note_id = create_secret_note(&worker, 1);
    let locked = accepted(apply(
        &worker,
        2,
        LibraryAction::LockNote {
            note_id,
            expected_revision: 1,
            credential: LockCredential::NewPassword {
                password: secret("pw"),
                hint: String::new(),
            },
        },
    ));
    let ciphertext = locked
        .accepted
        .snapshot
        .notes
        .iter()
        .find(|note| note.id == note_id)
        .and_then(|note| note.lock.as_ref())
        .unwrap()
        .sealed
        .ciphertext
        .clone();
    drop(worker);

    // Flip one ciphertext bit in both durable copies.
    for name in ["library.bin", "library.last-good.bin"] {
        let path = paths.data_root().join(name);
        let mut bytes = std::fs::read(&path).unwrap();
        let offset = bytes
            .windows(ciphertext.len())
            .position(|window| window == ciphertext.as_slice())
            .unwrap();
        bytes[offset + ciphertext.len() / 2] ^= 0x01;
        std::fs::write(&path, bytes).unwrap();
    }

    let keyring = NotesKeyring::new();
    let worker = start(&paths, &keyring);
    let relaunched = ready(&worker);
    assert!(relaunched.open_notes.is_empty());
    worker
        .try_send(WorkerCommand::UnlockNotes {
            request_id: 3,
            note_id,
            password: secret("pw"),
        })
        .unwrap();
    assert_eq!(
        rejected(next(&worker)),
        WorkerFailure::Lock(LockError::Tampered)
    );
    assert!(keyring.is_empty());
    drop(worker);
    std::fs::remove_dir_all(container).unwrap();
}

#[test]
fn change_password_reencrypts_and_reset_keeps_older_notes_on_their_password() {
    let (container, paths) = roots("passwords");
    let keyring = NotesKeyring::new();
    let worker = start(&paths, &keyring);
    ready(&worker);
    let first = create_secret_note(&worker, 1);
    let locked = accepted(apply(
        &worker,
        2,
        LibraryAction::LockNote {
            note_id: first,
            expected_revision: 1,
            credential: LockCredential::NewPassword {
                password: secret("one"),
                hint: "first".into(),
            },
        },
    ));
    let before = locked.accepted.snapshot.notes[0].lock.clone().unwrap();

    // Change Password needs the current password.
    assert_eq!(
        rejected(apply(
            &worker,
            3,
            LibraryAction::ChangeLockPassword {
                old_password: secret("not one"),
                new_password: secret("two"),
                hint: "second".into(),
            },
        )),
        WorkerFailure::Lock(LockError::WrongPassword)
    );
    let changed = accepted(apply(
        &worker,
        4,
        LibraryAction::ChangeLockPassword {
            old_password: secret("one"),
            new_password: secret("two"),
            hint: "second".into(),
        },
    ));
    assert_eq!(changed.result, ActionResult::LockPasswordChanged);
    let after = changed.accepted.snapshot.notes[0].lock.clone().unwrap();
    assert_ne!(after.key_id, before.key_id);
    assert_ne!(after.sealed, before.sealed);
    // The old generation is gone; only the new password remains.
    assert_eq!(changed.accepted.snapshot.lock_keys.len(), 1);

    // Reset Password: future locks use the new password, the first note
    // keeps the password it already had.
    let reset = accepted(apply(
        &worker,
        5,
        LibraryAction::ResetLockPassword {
            new_password: secret("three"),
            hint: "third".into(),
        },
    ));
    assert_eq!(reset.accepted.snapshot.lock_keys.len(), 2);
    let second = create_secret_note(&worker, 6);
    let second_locked = accepted(apply(
        &worker,
        7,
        LibraryAction::LockNote {
            note_id: second,
            expected_revision: 1,
            credential: LockCredential::Open,
        },
    ));
    let second_key = second_locked
        .accepted
        .snapshot
        .notes
        .iter()
        .find(|note| note.id == second)
        .and_then(|note| note.lock.as_ref())
        .unwrap()
        .key_id;
    assert_eq!(
        Some(second_key),
        second_locked.accepted.snapshot.current_lock_key
    );
    assert_ne!(second_key, after.key_id);

    worker
        .try_send(WorkerCommand::CloseLockedNotes { request_id: 8 })
        .unwrap();
    lock_state(next(&worker));
    let unlock = |request_id, note_id, password: &str| {
        worker
            .try_send(WorkerCommand::UnlockNotes {
                request_id,
                note_id,
                password: secret(password),
            })
            .unwrap();
        next(&worker)
    };
    assert_eq!(
        rejected(unlock(9, first, "one")),
        WorkerFailure::Lock(LockError::WrongPassword)
    );
    assert_eq!(
        rejected(unlock(10, first, "three")),
        WorkerFailure::Lock(LockError::WrongPassword)
    );
    let opened = lock_state(unlock(11, first, "two"));
    assert!(opened.accepted.open_notes.contains(&first));
    assert!(!opened.accepted.open_notes.contains(&second));
    let opened = lock_state(unlock(12, second, "three"));
    assert!(opened.accepted.open_notes.contains(&second));
    drop(worker);
    std::fs::remove_dir_all(container).unwrap();
}

#[test]
fn search_never_indexes_a_locked_note_even_while_it_is_open() {
    let (container, paths) = roots("search");
    let keyring = NotesKeyring::new();
    let worker = start(&paths, &keyring);
    ready(&worker);
    let locked_id = create_secret_note(&worker, 1);
    let plain = accepted(apply(
        &worker,
        2,
        LibraryAction::CreateNote(NewNote {
            created_unix_ms: 10,
            title: "Shopping".into(),
            body: "vault combination is also a phrase here".into(),
            tags: Vec::new(),
            folder_id: None,
        }),
    ));
    let plain_id = match plain.result {
        ActionResult::CreatedNote(note_id) => note_id,
        result => panic!("unexpected {result:?}"),
    };
    let locked = accepted(apply(
        &worker,
        3,
        LibraryAction::LockNote {
            note_id: locked_id,
            expected_revision: 1,
            credential: LockCredential::NewPassword {
                password: secret("pw"),
                hint: String::new(),
            },
        },
    ));
    // The UI's view holds the open note's decrypted content…
    assert!(locked.accepted.open_notes.contains(&locked_id));
    let snapshot = locked.accepted.snapshot.clone();
    let index = NotesSearchIndex::build(snapshot.clone()).unwrap();
    for query in ["vault combination", SECRET_TAG, "Quarterly"] {
        let session = SearchRequest::new(
            SearchGeneration::new(1).unwrap(),
            snapshot.revision,
            query,
            50,
            SearchCancellation::default(),
        )
        .unwrap();
        let batch = index.search(&session).unwrap();
        // …but the index never contains it.
        assert!(
            batch.hits.iter().all(|hit| hit.note_id != locked_id),
            "{query} matched the locked note"
        );
        if query == "vault combination" {
            assert!(batch.hits.iter().any(|hit| hit.note_id == plain_id));
        }
    }
    drop(worker);
    std::fs::remove_dir_all(container).unwrap();
}

#[test]
fn smart_folders_persist_with_the_library() {
    let (container, paths) = roots("smart");
    let keyring = NotesKeyring::new();
    let worker = start(&paths, &keyring);
    ready(&worker);
    let created = accepted(apply(
        &worker,
        1,
        LibraryAction::CreateSmartFolder {
            name: "Recipes".into(),
            tag: "cooking".into(),
        },
    ));
    let ActionResult::CreatedSmartFolder(smart_folder_id) = created.result else {
        panic!("unexpected {:?}", created.result);
    };
    drop(worker);

    let worker = start(&paths, &keyring);
    let relaunched = ready(&worker);
    assert_eq!(relaunched.snapshot.smart_folders.len(), 1);
    assert_eq!(relaunched.snapshot.smart_folders[0].name, "Recipes");
    assert_eq!(relaunched.snapshot.smart_folders[0].tag, "cooking");
    let deleted = accepted(apply(
        &worker,
        2,
        LibraryAction::DeleteSmartFolder { smart_folder_id },
    ));
    assert!(deleted.accepted.snapshot.smart_folders.is_empty());
    drop(worker);
    std::fs::remove_dir_all(container).unwrap();
}
