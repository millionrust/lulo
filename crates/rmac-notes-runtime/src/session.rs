use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use rmac_notes_storage::{PendingReason, StartupError};
use rmac_notes_store::{FolderId, FolderRecord, LibrarySnapshot, NoteId, NoteRecord, SortOrder};

use crate::{ActionResult, EditGeneration, MigrationReviewSummary, RejectedEvent, WorkerEvent};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FolderSelection {
    #[default]
    All,
    Folder(FolderId),
    Trash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Starting,
    MigrationReview(MigrationReviewSummary),
    Ready,
    Pending {
        request_id: u64,
        generation: Option<EditGeneration>,
        reason: PendingReason,
    },
    Failed(StartupError),
    Stopped,
}

pub struct NotesSession {
    phase: SessionPhase,
    snapshot: Option<Arc<LibrarySnapshot>>,
    folder: FolderSelection,
    selected_note: Option<NoteId>,
    last_rejection: Option<RejectedEvent>,
}

impl NotesSession {
    pub fn new() -> Self {
        Self {
            phase: SessionPhase::Starting,
            snapshot: None,
            folder: FolderSelection::All,
            selected_note: None,
            last_rejection: None,
        }
    }

    pub fn phase(&self) -> &SessionPhase {
        &self.phase
    }

    pub fn snapshot(&self) -> Option<&Arc<LibrarySnapshot>> {
        self.snapshot.as_ref()
    }

    pub fn folder_selection(&self) -> FolderSelection {
        self.folder
    }

    pub fn selected_note_id(&self) -> Option<NoteId> {
        self.selected_note
    }

    pub fn selected_note(&self) -> Option<&NoteRecord> {
        let selected = self.selected_note?;
        self.snapshot
            .as_ref()?
            .notes
            .iter()
            .find(|note| note.id == selected && self.note_is_visible(note))
    }

    pub fn last_rejection(&self) -> Option<RejectedEvent> {
        self.last_rejection
    }

    pub fn folders(&self) -> Vec<&FolderRecord> {
        let mut folders = self
            .snapshot
            .as_ref()
            .into_iter()
            .flat_map(|snapshot| snapshot.folders.iter())
            .filter(|folder| !folder.deleted)
            .collect::<Vec<_>>();
        folders.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        folders
    }

    pub fn folder_count(&self, folder_id: FolderId) -> usize {
        self.snapshot.as_ref().map_or(0, |snapshot| {
            snapshot
                .notes
                .iter()
                .filter(|note| !note.deleted && note.folder_id == Some(folder_id))
                .count()
        })
    }

    pub fn visible_notes(&self) -> Vec<&NoteRecord> {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return Vec::new();
        };
        let mut notes = snapshot
            .notes
            .iter()
            .filter(|note| self.note_is_visible(note))
            .collect::<Vec<_>>();
        notes.sort_by(|left, right| compare_notes(snapshot.sort_order, left, right));
        notes
    }

    pub fn select_folder(&mut self, folder: FolderSelection) {
        self.folder = self.normalize_folder(folder);
        self.reconcile_selection(None);
    }

    pub fn select_note(&mut self, note_id: NoteId) -> bool {
        if self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .notes
                .iter()
                .any(|note| note.id == note_id && self.note_is_visible(note))
        }) {
            self.selected_note = Some(note_id);
            true
        } else {
            false
        }
    }

    pub fn apply(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::MigrationReview(summary) => {
                self.phase = SessionPhase::MigrationReview(summary);
                self.snapshot = None;
                self.selected_note = None;
                self.last_rejection = None;
            }
            WorkerEvent::Ready(event) => {
                self.adopt_snapshot(event.snapshot, None, false);
                self.phase = SessionPhase::Ready;
                self.last_rejection = None;
            }
            WorkerEvent::Coalesced { .. } => {}
            WorkerEvent::Accepted(event) => {
                let (preferred, reveal_preferred) = match event.result {
                    ActionResult::CreatedNote(note_id) => (Some(note_id), true),
                    ActionResult::CreatedFolder(folder_id) => {
                        self.folder = FolderSelection::Folder(folder_id);
                        (None, false)
                    }
                    _ => (self.selected_note, false),
                };
                self.adopt_snapshot(event.accepted.snapshot, preferred, reveal_preferred);
                self.phase = SessionPhase::Ready;
                self.last_rejection = None;
            }
            WorkerEvent::Pending(event) => {
                self.adopt_snapshot(event.accepted.snapshot, self.selected_note, false);
                self.phase = SessionPhase::Pending {
                    request_id: event.request_id,
                    generation: event.generation,
                    reason: event.reason,
                };
                self.last_rejection = None;
            }
            WorkerEvent::Rejected(event) => self.last_rejection = Some(event),
            WorkerEvent::StartupFailed(error) => {
                self.phase = SessionPhase::Failed(error);
                self.snapshot = None;
                self.selected_note = None;
            }
            WorkerEvent::Stopped { .. } => self.phase = SessionPhase::Stopped,
        }
    }

    fn adopt_snapshot(
        &mut self,
        snapshot: Arc<LibrarySnapshot>,
        preferred: Option<NoteId>,
        reveal_preferred: bool,
    ) {
        self.snapshot = Some(snapshot);
        self.folder = self.normalize_folder(self.folder);
        if reveal_preferred {
            if let Some(note) = preferred.and_then(|note_id| {
                self.snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
            }) {
                if !self.note_is_visible(note) {
                    self.folder = if note.deleted {
                        FolderSelection::Trash
                    } else {
                        note.folder_id
                            .map(FolderSelection::Folder)
                            .unwrap_or(FolderSelection::All)
                    };
                    self.folder = self.normalize_folder(self.folder);
                }
            }
        }
        self.reconcile_selection(preferred);
    }

    fn normalize_folder(&self, folder: FolderSelection) -> FolderSelection {
        let FolderSelection::Folder(folder_id) = folder else {
            return folder;
        };
        if self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .folders
                .iter()
                .any(|folder| folder.id == folder_id && !folder.deleted)
        }) {
            folder
        } else {
            FolderSelection::All
        }
    }

    fn reconcile_selection(&mut self, preferred: Option<NoteId>) {
        let candidate = preferred.or(self.selected_note);
        if candidate.is_some_and(|note_id| {
            self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .notes
                    .iter()
                    .any(|note| note.id == note_id && self.note_is_visible(note))
            })
        }) {
            self.selected_note = candidate;
            return;
        }
        self.selected_note = self.visible_notes().first().map(|note| note.id);
    }

    fn note_is_visible(&self, note: &NoteRecord) -> bool {
        match self.folder {
            FolderSelection::All => !note.deleted,
            FolderSelection::Folder(folder_id) => {
                !note.deleted && note.folder_id == Some(folder_id)
            }
            FolderSelection::Trash => note.deleted,
        }
    }
}

impl Default for NotesSession {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for NotesSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotesSession")
            .field("phase", &self.phase)
            .field(
                "revision",
                &self.snapshot.as_ref().map(|value| value.revision),
            )
            .field(
                "folder_count",
                &self.snapshot.as_ref().map(|value| value.folders.len()),
            )
            .field(
                "note_count",
                &self.snapshot.as_ref().map(|value| value.notes.len()),
            )
            .field("folder", &self.folder)
            .field("selected_note", &self.selected_note)
            .field("last_rejection", &self.last_rejection)
            .finish()
    }
}

fn compare_notes(sort_order: SortOrder, left: &NoteRecord, right: &NoteRecord) -> Ordering {
    (!left.pinned)
        .cmp(&(!right.pinned))
        .then_with(|| match sort_order {
            SortOrder::Edited => right.modified_unix_ms.cmp(&left.modified_unix_ms),
            SortOrder::Created => right.created_unix_ms.cmp(&left.created_unix_ms),
            SortOrder::Title => left.title.to_lowercase().cmp(&right.title.to_lowercase()),
        })
        .then_with(|| left.id.cmp(&right.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notes_storage::{AcceptedCommit, RecoveryNotice};
    use rmac_notes_store::{FolderRecord, NoteRecord};

    use crate::{AcceptedEvent, ActionResult, PendingEvent, SnapshotEvent, WorkerFailure};

    fn folder(id: u64, name: &str) -> FolderRecord {
        FolderRecord {
            id: FolderId::new(id).unwrap(),
            revision: 1,
            name: name.into(),
            deleted: false,
        }
    }

    #[test]
    fn accepted_created_note_is_revealed_outside_the_current_filter() {
        let mut session = NotesSession::new();
        let initial = snapshot(SortOrder::Edited);
        session.apply(WorkerEvent::Ready(snapshot_event(initial)));
        session.select_folder(FolderSelection::Folder(FolderId::new(1).unwrap()));

        let mut created = (*snapshot(SortOrder::Edited)).clone();
        created.revision += 1;
        created
            .notes
            .push(note(5, Some(2), "Created elsewhere", 40, 40, false, false));
        created.next_note_id = 6;
        let created = Arc::new(created);
        session.apply(WorkerEvent::Accepted(AcceptedEvent {
            request_id: 11,
            generation: None,
            result: ActionResult::CreatedNote(NoteId::new(5).unwrap()),
            commit: AcceptedCommit {
                revision: created.revision,
                maintenance_pending: false,
                recovered_after_error: false,
            },
            accepted: snapshot_event(created),
        }));

        assert_eq!(
            session.folder_selection(),
            FolderSelection::Folder(FolderId::new(2).unwrap())
        );
        assert_eq!(session.selected_note_id(), NoteId::new(5));
    }

    fn note(
        id: u64,
        folder_id: Option<u64>,
        title: &str,
        created: u64,
        modified: u64,
        pinned: bool,
        deleted: bool,
    ) -> NoteRecord {
        NoteRecord {
            id: NoteId::new(id).unwrap(),
            revision: 1,
            created_unix_ms: created,
            modified_unix_ms: modified,
            title: title.into(),
            body: format!("private body for {title}"),
            tags: vec!["private-tag".into()],
            folder_id: folder_id.map(|id| FolderId::new(id).unwrap()),
            pinned,
            deleted,
            attachments: Vec::new(),
        }
    }

    fn snapshot(sort_order: SortOrder) -> Arc<LibrarySnapshot> {
        Arc::new(LibrarySnapshot {
            revision: 4,
            sort_order,
            next_note_id: 5,
            next_folder_id: 3,
            next_attachment_id: 1,
            folders: vec![folder(2, "Work"), folder(1, "Home")],
            notes: vec![
                note(1, Some(1), "Old", 10, 20, false, false),
                note(2, Some(1), "Pinned", 11, 21, true, false),
                note(3, Some(2), "Newest", 12, 30, false, false),
                note(4, Some(2), "Trashed", 13, 31, false, true),
            ],
            attachments: Vec::new(),
        })
    }

    fn snapshot_event(snapshot: Arc<LibrarySnapshot>) -> SnapshotEvent {
        SnapshotEvent {
            request_id: None,
            snapshot,
            notices: Vec::<RecoveryNotice>::new(),
        }
    }

    #[test]
    fn stable_selection_survives_reorder_and_falls_forward_after_delete() {
        let mut session = NotesSession::new();
        session.apply(WorkerEvent::Ready(snapshot_event(snapshot(
            SortOrder::Edited,
        ))));
        assert_eq!(session.selected_note_id(), NoteId::new(2));
        assert!(session.select_note(NoteId::new(1).unwrap()));

        let mut changed = (*snapshot(SortOrder::Title)).clone();
        changed.revision += 1;
        changed.notes.reverse();
        session.apply(WorkerEvent::Ready(snapshot_event(Arc::new(changed))));
        assert_eq!(session.selected_note_id(), NoteId::new(1));

        let mut deleted = (*snapshot(SortOrder::Edited)).clone();
        deleted.revision += 2;
        deleted.notes[0].deleted = true;
        session.apply(WorkerEvent::Ready(snapshot_event(Arc::new(deleted))));
        assert_eq!(session.selected_note_id(), NoteId::new(2));
    }

    #[test]
    fn folder_and_trash_views_are_stable_id_filtered_and_sorted() {
        let mut session = NotesSession::new();
        session.apply(WorkerEvent::Ready(snapshot_event(snapshot(
            SortOrder::Edited,
        ))));
        assert_eq!(
            session
                .folders()
                .iter()
                .map(|folder| folder.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Home", "Work"]
        );
        assert_eq!(session.folder_count(FolderId::new(1).unwrap()), 2);
        assert_eq!(
            session
                .visible_notes()
                .iter()
                .map(|note| note.id)
                .collect::<Vec<_>>(),
            vec![
                NoteId::new(2).unwrap(),
                NoteId::new(3).unwrap(),
                NoteId::new(1).unwrap()
            ]
        );

        session.select_folder(FolderSelection::Folder(FolderId::new(2).unwrap()));
        assert_eq!(session.selected_note_id(), NoteId::new(3));
        assert!(!session.select_note(NoteId::new(4).unwrap()));
        session.select_folder(FolderSelection::Trash);
        assert_eq!(session.selected_note_id(), NoteId::new(4));
    }

    #[test]
    fn worker_phases_and_created_identity_project_without_optimistic_state() {
        let mut session = NotesSession::new();
        session.apply(WorkerEvent::MigrationReview(MigrationReviewSummary {
            revision: 1,
            folders: 2,
            notes: 4,
            managed_attachments: 0,
            recovery_files: 0,
            warnings: Vec::new(),
        }));
        assert!(matches!(session.phase(), SessionPhase::MigrationReview(_)));

        let snapshot = snapshot(SortOrder::Edited);
        session.apply(WorkerEvent::Ready(snapshot_event(snapshot.clone())));
        session.apply(WorkerEvent::Pending(PendingEvent {
            request_id: 9,
            generation: EditGeneration::new(3),
            reason: PendingReason::AcceptedStateChanged,
            accepted: snapshot_event(snapshot.clone()),
        }));
        assert!(matches!(
            session.phase(),
            SessionPhase::Pending {
                request_id: 9,
                reason: PendingReason::AcceptedStateChanged,
                ..
            }
        ));

        let rejection = RejectedEvent {
            request_id: 10,
            generation: None,
            failure: WorkerFailure::CommitPending,
        };
        session.apply(WorkerEvent::Rejected(rejection));
        assert_eq!(session.last_rejection(), Some(rejection));
        assert!(matches!(session.phase(), SessionPhase::Pending { .. }));

        session.apply(WorkerEvent::Accepted(AcceptedEvent {
            request_id: 9,
            generation: EditGeneration::new(3),
            result: ActionResult::CreatedNote(NoteId::new(3).unwrap()),
            commit: AcceptedCommit {
                revision: 4,
                maintenance_pending: false,
                recovered_after_error: false,
            },
            accepted: snapshot_event(snapshot),
        }));
        assert_eq!(session.phase(), &SessionPhase::Ready);
        assert_eq!(session.selected_note_id(), NoteId::new(3));
        assert_eq!(session.last_rejection(), None);
    }

    #[test]
    fn debug_output_contains_no_note_content() {
        let mut session = NotesSession::new();
        session.apply(WorkerEvent::Ready(snapshot_event(snapshot(
            SortOrder::Edited,
        ))));

        let debug = format!("{session:?}");

        assert!(!debug.contains("private body"));
        assert!(!debug.contains("private-tag"));
        assert!(!debug.contains("Pinned"));
        assert!(debug.contains("note_count"));
    }
}
