//! Locked notes inside the repository worker.
//!
//! The worker owns the only authoritative snapshot, so it is the only place
//! a locked note is sealed, re-sealed or opened for an edit. Keys come from
//! the session's [`NotesKeyring`]; the UI only ever receives the decrypted
//! in-memory view built by [`open_view`].

use sha2::{Digest as _, Sha256};

use rmac_notes_store::{
    create_lock_key, is_valid_name, open_attachment, open_note, seal_attachment, seal_note,
    unlock_lock_key, AttachmentKind, LibraryTransaction, LockKey, LockedNoteContent, NoteRecord,
    SEALED_ATTACHMENT_NAME,
};

use super::*;

/// The UI's view of `snapshot`: locked notes whose key is open carry their
/// decrypted content and attachment names. Returns the opened note ids.
pub(super) fn open_view(
    snapshot: &LibrarySnapshot,
    keyring: &NotesKeyring,
) -> (LibrarySnapshot, BTreeSet<NoteId>) {
    let mut view = snapshot.clone();
    let mut open = BTreeSet::new();
    if keyring.is_empty() {
        return (view, open);
    }
    let mut names = BTreeMap::new();
    for note in &mut view.notes {
        let Some(lock) = &note.lock else {
            continue;
        };
        let Some(key) = keyring.get(lock.key_id) else {
            continue;
        };
        let Ok(content) = open_note(&key, note.id, lock) else {
            continue;
        };
        note.title = content.title.clone();
        note.body = content.body.clone();
        note.tags = content.tags.clone();
        for (id, name) in &content.attachment_names {
            names.insert(*id, name.clone());
        }
        open.insert(note.id);
    }
    for attachment in &mut view.attachments {
        if attachment.sealed_key.is_some() {
            if let Some(name) = names.remove(&attachment.id) {
                attachment.display_name = name;
            }
        }
    }
    (view, open)
}

pub(super) fn emit_lock_state(
    events: &SyncSender<WorkerEvent>,
    ready: &ReadyState,
    request_id: u64,
    unlocked_note: Option<NoteId>,
) -> bool {
    events
        .send(WorkerEvent::LockStateChanged(LockStateEvent {
            request_id,
            unlocked_note,
            accepted: SnapshotEvent::from_library(
                &ready.library,
                &ready.lock.keyring,
                Some(request_id),
            ),
        }))
        .is_ok()
}

pub(super) fn reject_lock(
    events: &SyncSender<WorkerEvent>,
    request_id: u64,
    failure: WorkerFailure,
) -> CommitDisposition {
    reject_with(events, request_id, None, failure)
}

fn reject_with(
    events: &SyncSender<WorkerEvent>,
    request_id: u64,
    generation: Option<EditGeneration>,
    failure: WorkerFailure,
) -> CommitDisposition {
    if emit_request_rejected(events, request_id, generation, failure) {
        CommitDisposition::Rejected
    } else {
        CommitDisposition::Stopped
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn fallback_attachment_name(kind: AttachmentKind) -> String {
    match kind {
        AttachmentKind::Png => "Image.png",
        AttachmentKind::Jpeg => "Image.jpg",
        AttachmentKind::Gif => "Image.gif",
        AttachmentKind::WebP => "Image.webp",
    }
    .to_owned()
}

fn find_note(snapshot: &LibrarySnapshot, note_id: NoteId) -> Result<&NoteRecord, WorkerFailure> {
    snapshot
        .notes
        .iter()
        .find(|note| note.id == note_id)
        .ok_or(WorkerFailure::Mutation(MutationError::MissingNote))
}

/// The open key for `key_id`, or the one `password` derives (then cached).
fn key_for(
    ready: &ReadyState,
    key_id: u32,
    password: Option<&LockSecret>,
) -> Result<Arc<rmac_notes_store::LockKey>, WorkerFailure> {
    if let Some(key) = ready.lock.keyring.get(key_id) {
        return Ok(key);
    }
    let Some(password) = password else {
        return Err(WorkerFailure::LockedNoteClosed);
    };
    let record = ready
        .library
        .snapshot()
        .lock_key(key_id)
        .ok_or(WorkerFailure::Mutation(MutationError::InvalidLockKey))?;
    let key = unlock_lock_key(record, password.expose()).map_err(WorkerFailure::Lock)?;
    ready.lock.keyring.insert(key);
    ready
        .lock
        .keyring
        .get(key_id)
        .ok_or(WorkerFailure::LockedNoteClosed)
}

/// Seal (or re-seal under `key`) every attachment of `note_id` that is not
/// already sealed with it, recording the new bytes for the rewrite commit.
fn seal_note_attachments(
    ready: &ReadyState,
    transaction: &mut LibraryTransaction,
    note_id: NoteId,
    key: &LockKey,
) -> Result<Vec<AttachmentRewrite>, WorkerFailure> {
    let mut rewrites = Vec::new();
    for attachment in ready
        .library
        .snapshot()
        .attachments
        .iter()
        .filter(|attachment| {
            attachment.note_id == note_id && attachment.sealed_key != Some(key.key_id())
        })
    {
        let bytes = match ready.library.read_managed_attachment(attachment) {
            Ok(bytes) => bytes,
            // A removed attachment whose bytes are already gone has nothing
            // left to protect.
            Err(error)
                if attachment.deleted
                    && error.kind == rmac_notes_storage::ErrorKind::Io(io::ErrorKind::NotFound) =>
            {
                continue
            }
            Err(error) => return Err(WorkerFailure::Storage(error)),
        };
        let plain = match attachment.sealed_key {
            None => Zeroizing::new(bytes),
            Some(old) => {
                let old_key = ready
                    .lock
                    .keyring
                    .get(old)
                    .ok_or(WorkerFailure::LockedNoteClosed)?;
                open_attachment(&old_key, attachment.id, &bytes).map_err(WorkerFailure::Lock)?
            }
        };
        let sealed = seal_attachment(key, attachment.id, &plain).map_err(WorkerFailure::Lock)?;
        transaction
            .rewrite_attachment(
                attachment.id,
                sealed.len() as u64,
                digest(&sealed),
                Some(key.key_id()),
                SEALED_ATTACHMENT_NAME.to_owned(),
            )
            .map_err(WorkerFailure::Mutation)?;
        rewrites.push(AttachmentRewrite {
            attachment_id: attachment.id,
            bytes: sealed,
        });
    }
    Ok(rewrites)
}

fn commit_with_rewrites(
    ready: &mut ReadyState,
    transaction: LibraryTransaction,
    rewrites: Vec<AttachmentRewrite>,
    request_id: u64,
    result: ActionResult,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let context = RequestContext {
        request_id,
        generation: None,
        result,
        draft: None,
        conflict: None,
    };
    let commit = if rewrites.is_empty() {
        TransactionCommit::Ordinary
    } else {
        TransactionCommit::AttachmentRewrite(rewrites)
    };
    commit_transaction(ready, transaction, commit, context, events)
}

/// An edit of an open locked note: validated, sealed, never drafted.
pub(super) fn commit_locked_edit(
    ready: &mut ReadyState,
    edit: ScheduledEdit,
    current_revision: u64,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let request_id = edit.request_id();
    let generation = Some(edit.generation());
    let note_id = edit.note_id();
    let changes = edit.changes();
    if let Err(error) = changes.validate_content() {
        return reject_mutation(events, request_id, generation, error);
    }
    let snapshot = ready.library.snapshot();
    let Some(lock) = snapshot
        .notes
        .iter()
        .find(|note| note.id == note_id)
        .and_then(|note| note.lock.clone())
    else {
        return reject_mutation(events, request_id, generation, MutationError::NoteNotLocked);
    };
    let modified = snapshot
        .notes
        .iter()
        .find(|note| note.id == note_id)
        .map_or(0, |note| note.modified_unix_ms);
    let Some(key) = ready.lock.keyring.get(lock.key_id) else {
        return reject_with(
            events,
            request_id,
            generation,
            WorkerFailure::LockedNoteClosed,
        );
    };
    let current = match open_note(&key, note_id, &lock) {
        Ok(current) => current,
        Err(error) => {
            return reject_with(events, request_id, generation, WorkerFailure::Lock(error))
        }
    };
    if current.title == changes.title
        && current.body == changes.body
        && current.tags == changes.tags
        && modified == changes.modified_unix_ms
    {
        return reject_mutation(events, request_id, generation, MutationError::NoChanges);
    }
    let content = LockedNoteContent {
        title: changes.title.clone(),
        body: changes.body.clone(),
        tags: changes.tags.clone(),
        attachment_names: current.attachment_names.clone(),
    };
    let sealed = match seal_note(&key, note_id, &content) {
        Ok(sealed) => sealed,
        Err(error) => {
            return reject_with(events, request_id, generation, WorkerFailure::Lock(error))
        }
    };
    let mut transaction = match ready.library.begin() {
        Ok(transaction) => transaction,
        Err(error) => return reject_mutation(events, request_id, generation, error),
    };
    if let Err(error) =
        transaction.edit_locked_note(note_id, current_revision, changes.modified_unix_ms, sealed)
    {
        return reject_mutation(events, request_id, generation, error);
    }
    let context = RequestContext {
        request_id,
        generation,
        result: ActionResult::Edited(note_id),
        // Never a plaintext recovery draft, and no "keep both" copy: both
        // would put the locked note's content on disk unencrypted.
        draft: None,
        conflict: None,
    };
    commit_transaction(
        ready,
        transaction,
        TransactionCommit::Ordinary,
        context,
        events,
    )
}

/// File ▸ Lock Note.
pub(super) fn lock_note(
    ready: &mut ReadyState,
    request_id: u64,
    note_id: NoteId,
    expected_revision: u64,
    credential: LockCredential,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    match prepare_lock(ready, note_id, expected_revision, credential) {
        Ok((transaction, rewrites, key)) => {
            // The note stays open after locking, as on macOS, until Close
            // All Locked Notes (or inactivity, sleep, the lock screen).
            let key_id = key.key_id();
            let was_open = ready.lock.keyring.contains(key_id);
            ready.lock.keyring.insert_shared(key);
            if ready.drafts.remove(note_id).is_ok() {
                ready.recoverable_drafts.remove(&note_id);
            }
            let disposition = commit_with_rewrites(
                ready,
                transaction,
                rewrites,
                request_id,
                ActionResult::LockedNote(note_id),
                events,
            );
            if matches!(disposition, CommitDisposition::Rejected) && !was_open {
                ready.lock.keyring.remove(key_id);
            }
            disposition
        }
        Err(failure) => reject_lock(events, request_id, failure),
    }
}

fn prepare_lock(
    ready: &ReadyState,
    note_id: NoteId,
    expected_revision: u64,
    credential: LockCredential,
) -> Result<(LibraryTransaction, Vec<AttachmentRewrite>, Arc<LockKey>), WorkerFailure> {
    let snapshot = ready.library.snapshot();
    let note = find_note(snapshot, note_id)?;
    if note.deleted {
        return Err(WorkerFailure::Mutation(MutationError::DeletedNote));
    }
    if note.lock.is_some() {
        return Err(WorkerFailure::Mutation(MutationError::NoteLocked));
    }
    if note.revision != expected_revision {
        return Err(WorkerFailure::Mutation(MutationError::RevisionConflict));
    }
    let mut transaction = ready.library.begin().map_err(WorkerFailure::Mutation)?;
    let key = match credential {
        LockCredential::Open => {
            let current = snapshot
                .current_lock_key
                .ok_or(WorkerFailure::LockedNoteClosed)?;
            ready
                .lock
                .keyring
                .get(current)
                .ok_or(WorkerFailure::LockedNoteClosed)?
        }
        LockCredential::Password(password) => {
            let current = snapshot
                .current_lock_key
                .ok_or(WorkerFailure::Mutation(MutationError::InvalidLockKey))?;
            key_for(ready, current, Some(&password))?
        }
        LockCredential::NewPassword { password, hint } => {
            if snapshot.current_lock_key.is_some() {
                return Err(WorkerFailure::Mutation(MutationError::InvalidLockKey));
            }
            let id = transaction.next_lock_key_id();
            let (record, key) = create_lock_key(id, password.expose(), &hint, ready.lock.kdf)
                .map_err(WorkerFailure::Lock)?;
            transaction
                .add_lock_key(record, true)
                .map_err(WorkerFailure::Mutation)?;
            Arc::new(key)
        }
    };
    let content = LockedNoteContent {
        title: note.title.clone(),
        body: note.body.clone(),
        tags: note.tags.clone(),
        attachment_names: snapshot
            .attachments
            .iter()
            .filter(|attachment| attachment.note_id == note_id)
            .map(|attachment| (attachment.id, attachment.display_name.clone()))
            .collect(),
    };
    let sealed = seal_note(&key, note_id, &content).map_err(WorkerFailure::Lock)?;
    transaction
        .lock_note(note_id, expected_revision, sealed)
        .map_err(WorkerFailure::Mutation)?;
    let rewrites = seal_note_attachments(ready, &mut transaction, note_id, &key)?;
    Ok((transaction, rewrites, key))
}

/// File ▸ Remove Lock: decrypt the note and its attachments back to
/// plaintext. Forgets an older password nothing uses any more.
pub(super) fn remove_note_lock(
    ready: &mut ReadyState,
    request_id: u64,
    note_id: NoteId,
    expected_revision: u64,
    password: Option<LockSecret>,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    match prepare_remove_lock(ready, note_id, expected_revision, password.as_ref()) {
        Ok((transaction, rewrites)) => commit_with_rewrites(
            ready,
            transaction,
            rewrites,
            request_id,
            ActionResult::RemovedLock(note_id),
            events,
        ),
        Err(failure) => reject_lock(events, request_id, failure),
    }
}

fn prepare_remove_lock(
    ready: &ReadyState,
    note_id: NoteId,
    expected_revision: u64,
    password: Option<&LockSecret>,
) -> Result<(LibraryTransaction, Vec<AttachmentRewrite>), WorkerFailure> {
    let snapshot = ready.library.snapshot();
    let note = find_note(snapshot, note_id)?;
    if note.deleted {
        return Err(WorkerFailure::Mutation(MutationError::DeletedNote));
    }
    let lock = note
        .lock
        .clone()
        .ok_or(WorkerFailure::Mutation(MutationError::NoteNotLocked))?;
    if note.revision != expected_revision {
        return Err(WorkerFailure::Mutation(MutationError::RevisionConflict));
    }
    let key = key_for(ready, lock.key_id, password)?;
    let content = open_note(&key, note_id, &lock).map_err(WorkerFailure::Lock)?;
    let mut transaction = ready.library.begin().map_err(WorkerFailure::Mutation)?;
    transaction
        .remove_note_lock(
            note_id,
            expected_revision,
            content.title.clone(),
            content.body.clone(),
            content.tags.clone(),
        )
        .map_err(WorkerFailure::Mutation)?;
    let mut rewrites = Vec::new();
    for attachment in snapshot
        .attachments
        .iter()
        .filter(|attachment| attachment.note_id == note_id && attachment.sealed_key.is_some())
    {
        let attachment_key = ready
            .lock
            .keyring
            .get(attachment.sealed_key.unwrap_or_default())
            .ok_or(WorkerFailure::LockedNoteClosed)?;
        let bytes = match ready.library.read_managed_attachment(attachment) {
            Ok(bytes) => bytes,
            Err(error)
                if attachment.deleted
                    && error.kind == rmac_notes_storage::ErrorKind::Io(io::ErrorKind::NotFound) =>
            {
                continue
            }
            Err(error) => return Err(WorkerFailure::Storage(error)),
        };
        let plain =
            open_attachment(&attachment_key, attachment.id, &bytes).map_err(WorkerFailure::Lock)?;
        let name = content
            .attachment_name(attachment.id)
            .filter(|name| is_valid_name(name))
            .map_or_else(|| fallback_attachment_name(attachment.kind), str::to_owned);
        transaction
            .rewrite_attachment(
                attachment.id,
                plain.len() as u64,
                digest(&plain),
                None,
                name,
            )
            .map_err(WorkerFailure::Mutation)?;
        rewrites.push(AttachmentRewrite {
            attachment_id: attachment.id,
            bytes: plain.to_vec(),
        });
    }
    if snapshot.current_lock_key != Some(lock.key_id) {
        // An older password nothing else uses is forgotten with this note.
        let _ = transaction.remove_lock_key(lock.key_id);
    }
    Ok((transaction, rewrites))
}

/// Notes ▸ Settings… ▸ Change Password…
pub(super) fn change_password(
    ready: &mut ReadyState,
    request_id: u64,
    old_password: &LockSecret,
    new_password: &LockSecret,
    hint: &str,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let prepared = prepare_change_password(ready, old_password, new_password, hint);
    let (transaction, rewrites, old_id, new_key) = match prepared {
        Ok(prepared) => prepared,
        Err(failure) => return reject_lock(events, request_id, failure),
    };
    let new_id = new_key.key_id();
    let old_key = ready.lock.keyring.get(old_id);
    ready.lock.keyring.remove(old_id);
    ready.lock.keyring.insert_shared(new_key);
    let disposition = commit_with_rewrites(
        ready,
        transaction,
        rewrites,
        request_id,
        ActionResult::LockPasswordChanged,
        events,
    );
    if matches!(disposition, CommitDisposition::Rejected) {
        ready.lock.keyring.remove(new_id);
        if let Some(old_key) = old_key {
            ready.lock.keyring.insert_shared(old_key);
        }
    }
    disposition
}

#[allow(clippy::type_complexity)]
fn prepare_change_password(
    ready: &ReadyState,
    old_password: &LockSecret,
    new_password: &LockSecret,
    hint: &str,
) -> Result<
    (
        LibraryTransaction,
        Vec<AttachmentRewrite>,
        u32,
        Arc<LockKey>,
    ),
    WorkerFailure,
> {
    let snapshot = ready.library.snapshot();
    let current = snapshot
        .current_lock_key
        .ok_or(WorkerFailure::Mutation(MutationError::InvalidLockKey))?;
    let record = snapshot
        .lock_key(current)
        .ok_or(WorkerFailure::Mutation(MutationError::InvalidLockKey))?;
    let old_key = unlock_lock_key(record, old_password.expose()).map_err(WorkerFailure::Lock)?;
    let mut transaction = ready.library.begin().map_err(WorkerFailure::Mutation)?;
    let new_id = transaction.next_lock_key_id();
    let (new_record, new_key) =
        create_lock_key(new_id, new_password.expose(), hint, ready.lock.kdf)
            .map_err(WorkerFailure::Lock)?;
    transaction
        .add_lock_key(new_record, true)
        .map_err(WorkerFailure::Mutation)?;
    for note in snapshot.notes.iter().filter(|note| {
        note.lock
            .as_ref()
            .is_some_and(|lock| lock.key_id == current)
    }) {
        let lock = note.lock.as_ref().expect("filtered to locked notes");
        let content = open_note(&old_key, note.id, lock).map_err(WorkerFailure::Lock)?;
        let sealed = seal_note(&new_key, note.id, &content).map_err(WorkerFailure::Lock)?;
        transaction
            .reseal_note(note.id, sealed)
            .map_err(WorkerFailure::Mutation)?;
    }
    let mut rewrites = Vec::new();
    for attachment in snapshot
        .attachments
        .iter()
        .filter(|attachment| attachment.sealed_key == Some(current))
    {
        let bytes = match ready.library.read_managed_attachment(attachment) {
            Ok(bytes) => bytes,
            Err(error)
                if attachment.deleted
                    && error.kind == rmac_notes_storage::ErrorKind::Io(io::ErrorKind::NotFound) =>
            {
                continue
            }
            Err(error) => return Err(WorkerFailure::Storage(error)),
        };
        let plain =
            open_attachment(&old_key, attachment.id, &bytes).map_err(WorkerFailure::Lock)?;
        let sealed =
            seal_attachment(&new_key, attachment.id, &plain).map_err(WorkerFailure::Lock)?;
        transaction
            .rewrite_attachment(
                attachment.id,
                sealed.len() as u64,
                digest(&sealed),
                Some(new_id),
                SEALED_ATTACHMENT_NAME.to_owned(),
            )
            .map_err(WorkerFailure::Mutation)?;
        rewrites.push(AttachmentRewrite {
            attachment_id: attachment.id,
            bytes: sealed,
        });
    }
    // Nothing is sealed with the old password any more.
    let _ = transaction.remove_lock_key(current);
    Ok((transaction, rewrites, current, Arc::new(new_key)))
}

/// Notes ▸ Settings… ▸ Reset Password…: a new password for future locks.
/// Notes already locked keep the password they were locked with (macOS
/// behaviour), so their key generation stays in the library.
pub(super) fn reset_password(
    ready: &mut ReadyState,
    request_id: u64,
    new_password: &LockSecret,
    hint: &str,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let snapshot = ready.library.snapshot();
    let previous = snapshot.current_lock_key;
    let mut transaction = match ready.library.begin() {
        Ok(transaction) => transaction,
        Err(error) => return reject_mutation(events, request_id, None, error),
    };
    let id = transaction.next_lock_key_id();
    let (record, key) = match create_lock_key(id, new_password.expose(), hint, ready.lock.kdf) {
        Ok(created) => created,
        Err(error) => return reject_lock(events, request_id, WorkerFailure::Lock(error)),
    };
    if let Err(error) = transaction.add_lock_key(record, true) {
        return reject_mutation(events, request_id, None, error);
    }
    if let Some(previous) = previous {
        // Forgotten only when no locked note or attachment still uses it.
        let _ = transaction.remove_lock_key(previous);
    }
    ready.lock.keyring.insert(key);
    let disposition = commit_with_rewrites(
        ready,
        transaction,
        Vec::new(),
        request_id,
        ActionResult::LockPasswordChanged,
        events,
    );
    if matches!(disposition, CommitDisposition::Rejected) {
        ready.lock.keyring.remove(id);
    }
    disposition
}

/// Edit ▸ Rename Attachment… on an open locked note: the name lives in the
/// note's sealed payload, so the note is re-sealed.
pub(super) fn rename_sealed_attachment(
    ready: &mut ReadyState,
    request_id: u64,
    attachment_id: AttachmentId,
    expected_attachment_revision: u64,
    display_name: String,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let prepared = (|| {
        if !is_valid_name(&display_name) {
            return Err(WorkerFailure::Mutation(MutationError::InvalidCandidate(
                rmac_notes_store::ValidationError::InvalidName,
            )));
        }
        let snapshot = ready.library.snapshot();
        let attachment = snapshot
            .attachments
            .iter()
            .find(|attachment| attachment.id == attachment_id)
            .ok_or(WorkerFailure::Mutation(MutationError::MissingAttachment))?;
        if attachment.revision != expected_attachment_revision {
            return Err(WorkerFailure::Mutation(MutationError::RevisionConflict));
        }
        if attachment.deleted {
            return Err(WorkerFailure::Mutation(
                MutationError::AttachmentAlreadyRemoved,
            ));
        }
        let note = find_note(snapshot, attachment.note_id)?;
        let lock = note
            .lock
            .as_ref()
            .ok_or(WorkerFailure::Mutation(MutationError::NoteNotLocked))?;
        let key = ready
            .lock
            .keyring
            .get(lock.key_id)
            .ok_or(WorkerFailure::LockedNoteClosed)?;
        let mut content = open_note(&key, note.id, lock).map_err(WorkerFailure::Lock)?;
        if content.attachment_name(attachment_id) == Some(display_name.as_str()) {
            return Err(WorkerFailure::Mutation(MutationError::NoChanges));
        }
        content
            .attachment_names
            .retain(|(id, _)| *id != attachment_id);
        content
            .attachment_names
            .push((attachment_id, display_name.clone()));
        let sealed = seal_note(&key, note.id, &content).map_err(WorkerFailure::Lock)?;
        let mut transaction = ready.library.begin().map_err(WorkerFailure::Mutation)?;
        transaction
            .reseal_note(note.id, sealed)
            .map_err(WorkerFailure::Mutation)?;
        Ok(transaction)
    })();
    match prepared {
        Ok(transaction) => commit_with_rewrites(
            ready,
            transaction,
            Vec::new(),
            request_id,
            ActionResult::Changed,
            events,
        ),
        Err(failure) => reject_lock(events, request_id, failure),
    }
}

/// Seal any still-plaintext attachment of a locked note whose key `key_id`
/// is open: an image just added to it, or one left by an interrupted lock.
pub(super) fn seal_unsealed_attachments(
    ready: &mut ReadyState,
    request_id: u64,
    key_id: u32,
    events: &SyncSender<WorkerEvent>,
) -> CommitDisposition {
    let Some(key) = ready.lock.keyring.get(key_id) else {
        return CommitDisposition::Ready;
    };
    let prepared = (|| {
        let snapshot = ready.library.snapshot();
        let mut transaction = ready.library.begin().map_err(WorkerFailure::Mutation)?;
        let mut rewrites = Vec::new();
        for note in snapshot
            .notes
            .iter()
            .filter(|note| note.lock.as_ref().is_some_and(|lock| lock.key_id == key_id))
        {
            let unsealed = snapshot
                .attachments
                .iter()
                .filter(|attachment| {
                    attachment.note_id == note.id && attachment.sealed_key.is_none()
                })
                .collect::<Vec<_>>();
            if unsealed.is_empty() {
                continue;
            }
            let lock = note.lock.as_ref().expect("filtered to locked notes");
            let mut content = open_note(&key, note.id, lock).map_err(WorkerFailure::Lock)?;
            for attachment in &unsealed {
                content
                    .attachment_names
                    .retain(|(id, _)| *id != attachment.id);
                content
                    .attachment_names
                    .push((attachment.id, attachment.display_name.clone()));
            }
            let sealed = seal_note(&key, note.id, &content).map_err(WorkerFailure::Lock)?;
            transaction
                .reseal_note(note.id, sealed)
                .map_err(WorkerFailure::Mutation)?;
            rewrites.extend(seal_note_attachments(
                ready,
                &mut transaction,
                note.id,
                &key,
            )?);
        }
        Ok((transaction, rewrites))
    })();
    match prepared {
        Ok((_, rewrites)) if rewrites.is_empty() => CommitDisposition::Ready,
        Ok((transaction, rewrites)) => commit_with_rewrites(
            ready,
            transaction,
            rewrites,
            request_id,
            ActionResult::Changed,
            events,
        ),
        Err(failure) => reject_lock(events, request_id, failure),
    }
}

/// The locked-note placeholder's password prompt.
pub(super) fn unlock_notes(
    mut ready: ReadyState,
    request_id: u64,
    note_id: NoteId,
    password: &LockSecret,
    events: &SyncSender<WorkerEvent>,
) -> Phase {
    let opened = (|| {
        let snapshot = ready.library.snapshot();
        let note = find_note(snapshot, note_id)?;
        let lock = note
            .lock
            .as_ref()
            .ok_or(WorkerFailure::Mutation(MutationError::NoteNotLocked))?;
        let record = snapshot
            .lock_key(lock.key_id)
            .ok_or(WorkerFailure::Mutation(MutationError::InvalidLockKey))?;
        let key = unlock_lock_key(record, password.expose()).map_err(WorkerFailure::Lock)?;
        // A tampered note must not open, even with the right password.
        open_note(&key, note_id, lock).map_err(WorkerFailure::Lock)?;
        Ok(key)
    })();
    let key = match opened {
        Ok(key) => key,
        Err(failure) => {
            return if emit_request_rejected(events, request_id, None, failure) {
                Phase::Ready(ready)
            } else {
                Phase::Stopped
            };
        }
    };
    let key_id = key.key_id();
    ready.lock.keyring.insert(key);
    if !emit_lock_state(events, &ready, request_id, Some(note_id)) {
        return Phase::Stopped;
    }
    match seal_unsealed_attachments(&mut ready, request_id, key_id, events) {
        CommitDisposition::Ready | CommitDisposition::Rejected => Phase::Ready(ready),
        CommitDisposition::Pending(pending, context) => Phase::Pending(PendingState {
            ready,
            pending,
            context,
        }),
        CommitDisposition::Stopped => Phase::Stopped,
    }
}
