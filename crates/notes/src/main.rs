//! rmac Notes live application.
//!
//! The GPUI view is a projection of `rmac-notes-runtime`: it never scans or
//! writes the library directly. Stable IDs, accepted snapshots, recovery, and
//! the single writer remain authoritative off the UI thread.

use std::io;
use std::thread;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    actions, div, prelude::FluentBuilder as _, px, AnyElement, AppContext as _, Context, Div,
    Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding, ParentElement, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::{Icon, IconName, Sizable as _, Size, StyledExt as _};
use rmac_editor::InputState;
use rmac_notes_runtime::{
    ActionRequest, ActionResult, DraftRecoveryKind, EditGeneration, LibraryAction, NotesSession,
    NotesWorker, NotesWorkerClient, NotesWorkerEvents, ScheduledEdit, SessionPhase, WorkerCommand,
    WorkerEvent, WorkerFailure, WorkerSendError, EVENT_CAPACITY,
};
use rmac_notes_storage::{resolve_notes_paths, PendingReason};
use rmac_notes_store::{NewNote, NoteChanges, NoteId, SortOrder};
use rmac_ui::{mac, Button, InputEvent, TextField};

const FOLDERS_W: f32 = 210.0;
const LIST_W: f32 = 310.0;

actions!(
    notes,
    [
        ComposeNote,
        CreateFolder,
        TrashOrRestore,
        TogglePin,
        SortByEdited,
        SortByCreated,
        SortByTitle
    ]
);

struct NotesView {
    worker: Option<NotesWorkerClient>,
    session: NotesSession,
    title: Entity<InputState>,
    body: Entity<InputState>,
    focus: FocusHandle,
    applying_snapshot: bool,
    next_request_id: u64,
    next_edit_generation: u64,
    latest_local_generation: Option<EditGeneration>,
    message: Option<SharedString>,
    recovery_notice_dismissed: bool,
    recovery_decision: Option<(NoteId, RecoveryDecision)>,
    recovery_copy_pending: Option<(u64, NoteId)>,
    closing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryDecision {
    RestoreOriginal,
    PreserveCopy,
}

impl NotesView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("cmd-n", ComposeNote, Some("Notes")),
            KeyBinding::new("shift-cmd-n", CreateFolder, Some("Notes")),
            KeyBinding::new("cmd-backspace", TrashOrRestore, Some("Notes")),
        ]);

        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let body = rmac_editor::multiline("Note", window, cx);
        cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_current_edit(cx);
            }
        })
        .detach();
        cx.subscribe(&body, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_current_edit(cx);
            }
        })
        .detach();

        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut view = Self {
            worker: None,
            session: NotesSession::new(),
            title,
            body,
            focus,
            applying_snapshot: false,
            next_request_id: 1,
            next_edit_generation: 1,
            latest_local_generation: None,
            message: None,
            recovery_notice_dismissed: false,
            recovery_decision: None,
            recovery_copy_pending: None,
            closing: false,
        };

        match resolve_notes_paths()
            .map_err(|error| error.to_string())
            .and_then(|paths| NotesWorker::start(paths).map_err(|error| error.to_string()))
            .and_then(|worker| {
                let (client, events) = worker.into_parts();
                bridge_worker_events(events)
                    .map(|receiver| (client, receiver))
                    .map_err(|error| format!("Notes could not start its event bridge: {error}"))
            }) {
            Ok((client, receiver)) => {
                view.worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, window, cx| {
                                this.apply_worker_event(event, window, cx)
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(message) => view.message = Some(message.into()),
        }

        view
    }

    fn apply_worker_event(
        &mut self,
        event: WorkerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let accepted_generation = match &event {
            WorkerEvent::Accepted(accepted) => accepted.generation,
            _ => None,
        };
        let sync_editor = match &event {
            WorkerEvent::Ready(_) => self.latest_local_generation.is_none(),
            WorkerEvent::Accepted(accepted) => match accepted.generation {
                Some(generation) => self
                    .latest_local_generation
                    .is_none_or(|latest| generation >= latest),
                None => self.latest_local_generation.is_none(),
            },
            _ => false,
        };
        let restored_draft = match &event {
            WorkerEvent::DraftRestored(restored) => Some(restored.clone()),
            _ => None,
        };
        let accepted_request_id = match &event {
            WorkerEvent::Accepted(accepted) => Some(accepted.request_id),
            _ => None,
        };
        let rejected_request_id = match &event {
            WorkerEvent::Rejected(rejected) => Some(rejected.request_id),
            _ => None,
        };
        if matches!(&event, WorkerEvent::DraftReview(_)) {
            self.recovery_notice_dismissed = false;
        }
        let reveal_created = matches!(
            &event,
            WorkerEvent::Accepted(accepted)
                if matches!(
                    accepted.result,
                    ActionResult::CreatedNote(_) | ActionResult::ImportedNote { .. }
                )
        );
        let rejection = match &event {
            WorkerEvent::Rejected(rejected) => Some(worker_failure_message(rejected.failure)),
            WorkerEvent::Pending(pending) => Some(pending_message(pending.reason)),
            WorkerEvent::StartupFailed(error) => Some(error.to_string()),
            _ => None,
        };
        self.session.apply(event);
        if let Some(accepted) = accepted_generation {
            if self
                .latest_local_generation
                .is_some_and(|latest| accepted >= latest)
            {
                self.latest_local_generation = None;
            }
        }
        if let Some(message) = rejection {
            self.message = Some(message.into());
        } else if sync_editor {
            self.message = None;
        }
        if sync_editor {
            self.sync_editor(window, cx);
        }
        if let Some(restored) = restored_draft {
            let decision = self
                .recovery_decision
                .take()
                .filter(|(note_id, _)| *note_id == restored.draft.note_id)
                .map(|(_, decision)| decision);
            match decision {
                Some(decision) => self.commit_restored_draft(restored, decision, window, cx),
                None => {
                    self.message = Some(
                        "Notes received an unexpected recovery response. The recovery record was preserved."
                            .into(),
                    );
                }
            }
        }
        if let Some(request_id) = rejected_request_id {
            if self
                .recovery_copy_pending
                .is_some_and(|(pending, _)| pending == request_id)
            {
                self.recovery_copy_pending = None;
            }
        }
        if let Some(request_id) = accepted_request_id {
            if let Some((_, draft_note_id)) = self
                .recovery_copy_pending
                .take_if(|(pending, _)| *pending == request_id)
            {
                self.discard_draft(draft_note_id, cx);
            }
        }
        if reveal_created {
            self.title.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (title, body) = self
            .session
            .selected_note()
            .map(|note| (note.title.clone(), note.body.clone()))
            .unwrap_or_default();
        self.applying_snapshot = true;
        self.title
            .update(cx, |state, cx| state.set_value(title, window, cx));
        self.body
            .update(cx, |state, cx| state.set_value(body, window, cx));
        self.applying_snapshot = false;
    }

    fn schedule_current_edit(&mut self, cx: &mut Context<Self>) {
        if self.applying_snapshot || !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note() else {
            return;
        };
        let note_id = note.id;
        let expected_revision = note.revision;
        let created_unix_ms = note.created_unix_ms;
        let previous_modified = note.modified_unix_ms;
        let tags = note.tags.clone();
        let title = self.title.read(cx).value().to_string();
        let body = self.body.read(cx).value().to_string();
        if title == note.title && body == note.body {
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let Some(generation) = self.take_edit_generation() else {
            return;
        };
        let modified_unix_ms = now_unix_ms().max(created_unix_ms).max(previous_modified);
        let edit = ScheduledEdit::new(
            request_id,
            generation,
            note_id,
            expected_revision,
            NoteChanges {
                modified_unix_ms,
                title,
                body,
                tags,
            },
        );
        match edit {
            Ok(edit) => {
                if self.send(WorkerCommand::ScheduleEdit(edit), cx) {
                    self.latest_local_generation = Some(generation);
                }
            }
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    fn commit_restored_draft(
        &mut self,
        restored: rmac_notes_runtime::DraftRestoredEvent,
        decision: RecoveryDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let note_id = restored.draft.note_id;
        if decision == RecoveryDecision::PreserveCopy {
            self.preserve_recovered_copy(restored, cx);
            return;
        }
        let destination = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .notes
                .iter()
                .find(|note| note.id == note_id && !note.deleted)
                .map(|note| note.folder_id)
        });
        let Some(folder_id) = destination else {
            self.message = Some(
                "The recovered edit no longer has a safe destination. Its recovery record was preserved."
                    .into(),
            );
            cx.notify();
            return;
        };
        self.session.select_folder(folder_id.map_or(
            rmac_notes_runtime::FolderSelection::All,
            rmac_notes_runtime::FolderSelection::Folder,
        ));
        if !self.session.select_note(note_id) {
            self.message = Some(
                "The recovered edit could not be selected safely. Its recovery record was preserved."
                    .into(),
            );
            cx.notify();
            return;
        }

        let changes = restored.draft.changes.clone();
        self.applying_snapshot = true;
        self.title.update(cx, |state, cx| {
            state.set_value(changes.title.clone(), window, cx)
        });
        self.body.update(cx, |state, cx| {
            state.set_value(changes.body.clone(), window, cx)
        });
        self.applying_snapshot = false;

        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let Some(generation) = self.take_edit_generation() else {
            return;
        };
        match ScheduledEdit::new(
            request_id,
            generation,
            note_id,
            restored.draft.base_note_revision,
            changes,
        ) {
            Ok(edit) => {
                self.message = Some("Restoring the recovered edit…".into());
                if self.send(WorkerCommand::ScheduleEdit(edit), cx) {
                    self.latest_local_generation = Some(generation);
                }
            }
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    fn preserve_recovered_copy(
        &mut self,
        restored: rmac_notes_runtime::DraftRestoredEvent,
        cx: &mut Context<Self>,
    ) {
        let draft_note_id = restored.draft.note_id;
        let folder_id = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .notes
                .iter()
                .find(|note| note.id == draft_note_id && !note.deleted)
                .and_then(|note| note.folder_id)
        });
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let changes = restored.draft.changes;
        let action = LibraryAction::CreateNote(NewNote {
            created_unix_ms: now_unix_ms(),
            title: changes.title,
            body: changes.body,
            tags: changes.tags,
            folder_id,
        });
        let request = match ActionRequest::new(request_id, action) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.recovery_copy_pending = Some((request_id, draft_note_id));
            self.message = Some("Preserving the recovered edit as a new note…".into());
            cx.notify();
        }
    }

    fn is_interactive_ready(&self) -> bool {
        matches!(self.session.phase(), SessionPhase::Ready) && !self.recovery_review_is_blocking()
    }

    fn recovery_review_is_blocking(&self) -> bool {
        self.session.draft_review().is_some() && !self.recovery_notice_dismissed
    }

    fn send_action(&mut self, action: LibraryAction, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        match ActionRequest::new(request_id, action) {
            Ok(request) => {
                self.send(WorkerCommand::Apply(request), cx);
            }
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    fn send(&mut self, command: WorkerCommand, cx: &mut Context<Self>) -> bool {
        let result = self
            .worker
            .as_ref()
            .ok_or(WorkerSendError::Closed)
            .and_then(|worker| worker.try_send(command));
        if let Err(error) = result {
            self.message = Some(error.to_string().into());
            cx.notify();
            false
        } else {
            true
        }
    }

    fn take_request_id(&mut self) -> Option<u64> {
        take_counter(&mut self.next_request_id).or_else(|| {
            self.message = Some("Notes exhausted its request identity sequence".into());
            None
        })
    }

    fn take_edit_generation(&mut self) -> Option<EditGeneration> {
        take_counter(&mut self.next_edit_generation)
            .and_then(EditGeneration::new)
            .or_else(|| {
                self.message = Some("Notes exhausted its edit generation sequence".into());
                None
            })
    }

    fn select_folder(
        &mut self,
        folder: rmac_notes_runtime::FolderSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        self.session.select_folder(folder);
        self.sync_editor(window, cx);
        cx.notify();
    }

    fn select_note(&mut self, note_id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.session.select_note(note_id) {
            self.sync_editor(window, cx);
            cx.notify();
        }
    }

    fn create_note(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let folder_id = match self.session.folder_selection() {
            rmac_notes_runtime::FolderSelection::Folder(folder_id) => Some(folder_id),
            _ => None,
        };
        self.send_action(
            LibraryAction::CreateNote(NewNote {
                created_unix_ms: now_unix_ms(),
                title: "New Note".into(),
                body: String::new(),
                tags: Vec::new(),
                folder_id,
            }),
            cx,
        );
    }

    fn create_folder(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let existing = self
            .session
            .folders()
            .into_iter()
            .map(|folder| folder.name.to_lowercase())
            .collect::<Vec<_>>();
        let name = unique_folder_name(&existing);
        self.send_action(LibraryAction::CreateFolder { name }, cx);
    }

    fn trash_or_restore(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note() else {
            return;
        };
        let action = if note.deleted {
            LibraryAction::RestoreNote {
                note_id: note.id,
                expected_revision: note.revision,
            }
        } else {
            LibraryAction::TrashNote {
                note_id: note.id,
                expected_revision: note.revision,
            }
        };
        self.send_action(action, cx);
    }

    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note() else {
            return;
        };
        self.send_action(
            LibraryAction::SetPinned {
                note_id: note.id,
                expected_revision: note.revision,
                pinned: !note.pinned,
            },
            cx,
        );
    }

    fn set_sort(&mut self, sort_order: SortOrder, cx: &mut Context<Self>) {
        if self.is_interactive_ready() {
            self.send_action(LibraryAction::SetSort(sort_order), cx);
        }
    }

    fn accept_migration(&mut self, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.send(WorkerCommand::AcceptMigration { request_id }, cx);
    }

    fn start_empty(&mut self, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.send(WorkerCommand::StartEmpty { request_id }, cx);
    }

    fn retry_pending(&mut self, cx: &mut Context<Self>) {
        self.send(WorkerCommand::RetryPending, cx);
    }

    fn discard_pending(&mut self, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        if self.send(WorkerCommand::DiscardPending { request_id }, cx) {
            self.latest_local_generation = None;
        }
    }

    fn restore_draft(
        &mut self,
        note_id: NoteId,
        decision: RecoveryDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.recovery_decision = Some((note_id, decision));
        if !self.send(
            WorkerCommand::RestoreDraft {
                request_id,
                note_id,
            },
            cx,
        ) {
            self.recovery_decision = None;
        }
    }

    fn discard_draft(&mut self, note_id: NoteId, cx: &mut Context<Self>) {
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        self.send(
            WorkerCommand::DiscardDraft {
                request_id,
                note_id,
            },
            cx,
        );
    }

    fn continue_after_recovery_notice(&mut self, cx: &mut Context<Self>) {
        if self
            .session
            .draft_review()
            .is_some_and(|review| review.drafts.is_empty())
        {
            self.recovery_notice_dismissed = true;
            self.message = None;
            cx.notify();
        }
    }

    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.session.phase(), SessionPhase::Pending { .. }) {
            self.message = Some("Retry or discard the pending change before closing Notes".into());
            cx.notify();
            return;
        }
        self.closing = true;
        let shutdown = self
            .worker
            .as_ref()
            .ok_or(WorkerSendError::Closed)
            .and_then(|worker| worker.try_send(WorkerCommand::Shutdown));
        if let Err(error) = shutdown {
            self.closing = false;
            self.message =
                Some(format!("Notes could not safely close yet: {error}. Try again.").into());
            cx.notify();
            return;
        }
        window.remove_window();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.is_interactive_ready();
        let selected = self.session.selected_note();
        let deleted = selected.is_some_and(|note| note.deleted);
        let pinned = selected.is_some_and(|note| note.pinned);
        let sort_order = self
            .session
            .snapshot()
            .map(|snapshot| snapshot.sort_order)
            .unwrap_or(SortOrder::Edited);
        let row = div()
            .size_full()
            .flex()
            .items_center()
            .child(div().w(px(FOLDERS_W - 76.0)))
            .child(
                div()
                    .w(px(LIST_W))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .pr_3()
                    .child(
                        Button::new("sort", "")
                            .icon(IconName::SortDescending)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(!ready)
                            .tooltip("Sort Notes")
                            .dropdown_menu(move |menu, _, _| {
                                menu.menu_with_check(
                                    "Date Edited",
                                    sort_order == SortOrder::Edited,
                                    Box::new(SortByEdited),
                                )
                                .menu_with_check(
                                    "Date Created",
                                    sort_order == SortOrder::Created,
                                    Box::new(SortByCreated),
                                )
                                .menu_with_check(
                                    "Title",
                                    sort_order == SortOrder::Title,
                                    Box::new(SortByTitle),
                                )
                            }),
                    )
                    .child(
                        Button::new("compose", "")
                            .icon(IconName::Plus)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(!ready)
                            .tooltip("New Note")
                            .on_click(cx.listener(|this, _, _, cx| this.create_note(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .pr_4()
                    .child(
                        Button::new("pin", "")
                            .icon(IconName::Star)
                            .ghost()
                            .selected(pinned)
                            .with_size(Size::Medium)
                            .disabled(!ready || deleted || selected.is_none())
                            .tooltip(if pinned { "Unpin Note" } else { "Pin Note" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_pin(cx))),
                    )
                    .child(
                        Button::new("trash", "")
                            .icon(if deleted {
                                IconName::ArrowUp
                            } else {
                                IconName::Delete
                            })
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(!ready || selected.is_none())
                            .tooltip(if deleted {
                                "Restore Note"
                            } else {
                                "Move to Trash"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.trash_or_restore(cx))),
                    ),
            );
        rmac_ui::toolbar(row)
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use rmac_notes_runtime::FolderSelection;

        let snapshot = self.session.snapshot();
        let all_count = snapshot.map_or(0, |snapshot| {
            snapshot.notes.iter().filter(|note| !note.deleted).count()
        });
        let trash_count = snapshot.map_or(0, |snapshot| {
            snapshot.notes.iter().filter(|note| note.deleted).count()
        });
        let current = self.session.folder_selection();
        let mut sidebar = div()
            .w(px(FOLDERS_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_3()
            .px_2()
            .bg(mac::sidebar())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .pb_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_tertiary())
                            .child("ON THIS COMPUTER"),
                    )
                    .child(
                        Button::new("new-folder", "")
                            .icon(IconName::Plus)
                            .ghost()
                            .with_size(Size::XSmall)
                            .disabled(!self.is_interactive_ready())
                            .tooltip("New Folder")
                            .on_click(cx.listener(|this, _, _, cx| this.create_folder(cx))),
                    ),
            )
            .child(folder_row(
                "all-notes",
                "All Notes",
                IconName::Folder,
                all_count,
                current == FolderSelection::All,
                cx.listener(|this, _, window, cx| {
                    this.select_folder(FolderSelection::All, window, cx)
                }),
            ));

        for folder in self.session.folders() {
            let folder_id = folder.id;
            sidebar = sidebar.child(folder_row(
                ("folder", folder_id.get()),
                folder.name.clone(),
                IconName::Folder,
                self.session.folder_count(folder_id),
                current == FolderSelection::Folder(folder_id),
                cx.listener(move |this, _, window, cx| {
                    this.select_folder(FolderSelection::Folder(folder_id), window, cx)
                }),
            ));
        }

        sidebar.child(div().mt_2().child(folder_row(
            "trash-notes",
            "Recently Deleted",
            IconName::Delete,
            trash_count,
            current == FolderSelection::Trash,
            cx.listener(|this, _, window, cx| {
                this.select_folder(FolderSelection::Trash, window, cx)
            }),
        )))
    }

    fn render_note_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.session.selected_note_id();
        let notes = self.session.visible_notes();
        let note_count = notes.len();
        let mut items = Vec::<AnyElement>::new();
        for note in notes {
            let note_id = note.id;
            let tags = note.tags.clone();
            items.push(
                div()
                    .id(("note", note.id.get()))
                    .mx_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.0))
                    .when(selected == Some(note.id), |element: Stateful<Div>| {
                        element.bg(mac::notes_selection())
                    })
                    .when(selected != Some(note.id), |element: Stateful<Div>| {
                        element.hover(|hover| hover.bg(mac::hover()))
                    })
                    .child(
                        div()
                            .v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .when(note.pinned, |element| {
                                        element.child(
                                            Icon::new(IconName::Star)
                                                .text_color(mac::notes_accent())
                                                .with_size(Size::XSmall),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(rmac_ui::text_px(14.0))
                                            .font_weight(mac::SEMIBOLD)
                                            .text_color(mac::text())
                                            .truncate()
                                            .child(display_title(&note.title)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .text_size(rmac_ui::text_px(12.0))
                                            .font_weight(mac::MEDIUM)
                                            .text_color(mac::text())
                                            .child(date_label(note.modified_unix_ms)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(rmac_ui::text_px(12.0))
                                            .text_color(mac::text_secondary())
                                            .truncate()
                                            .child(snippet(&note.body)),
                                    ),
                            )
                            .when(!tags.is_empty(), |element| {
                                element.child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_1()
                                        .pt_0p5()
                                        .children(tags.into_iter().map(tag_pill)),
                                )
                            }),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.select_note(note_id, window, cx)
                        }),
                    )
                    .into_any_element(),
            );
        }
        if items.is_empty() {
            items.push(
                div()
                    .px_4()
                    .py_6()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(mac::text_tertiary())
                    .child(
                        if self.session.folder_selection()
                            == rmac_notes_runtime::FolderSelection::Trash
                        {
                            "Recently Deleted is empty"
                        } else {
                            "No notes in this folder"
                        },
                    )
                    .into_any_element(),
            );
        }
        div()
            .w(px(LIST_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .bg(mac::list())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div()
                    .h(px(42.0))
                    .flex()
                    .items_center()
                    .px_4()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "{note_count} {}",
                        if note_count == 1 { "Note" } else { "Notes" }
                    )),
            )
            .child(
                div()
                    .id("notes-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .py_1()
                    .children(items),
            )
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(note) = self.session.selected_note() else {
            return centered_state("No Note Selected", "Choose a note or create a new one.");
        };
        let editable = self.is_interactive_ready();
        let words = self.body.read(cx).value().split_whitespace().count();
        let characters = self.body.read(cx).value().chars().count();
        div()
            .size_full()
            .v_flex()
            .bg(mac::window())
            .child(
                div()
                    .pt_3()
                    .pb_1()
                    .flex()
                    .justify_center()
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(mac::text_secondary())
                    .child(date_label(note.modified_unix_ms)),
            )
            .child(
                div()
                    .px(px(44.0))
                    .pt_1()
                    .text_size(px(28.0))
                    .line_height(px(34.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text())
                    .child(
                        TextField::new(&self.title)
                            .appearance(false)
                            .disabled(!editable),
                    ),
            )
            .when(!note.tags.is_empty(), |element| {
                element.child(
                    div()
                        .px(px(44.0))
                        .py_2()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(note.tags.clone().into_iter().map(tag_pill)),
                )
            })
            .when(!note.attachments.is_empty(), |element| {
                element.child(
                    div()
                        .px(px(44.0))
                        .pb_1()
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(mac::text_tertiary())
                        .child(format!(
                            "{} attachment{}",
                            note.attachments.len(),
                            if note.attachments.len() == 1 { "" } else { "s" }
                        )),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .px(px(44.0))
                    .pt_2()
                    .pb_4()
                    .text_size(px(16.0))
                    .line_height(px(24.0))
                    .text_color(mac::text())
                    .child(
                        TextField::new(&self.body)
                            .h_full()
                            .appearance(false)
                            .disabled(!editable),
                    ),
            )
            .child(
                div()
                    .h(px(24.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(mac::separator())
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(mac::text_tertiary())
                    .child(format!("{words} words"))
                    .child("•")
                    .child(format!("{characters} characters")),
            )
            .into_any_element()
    }

    fn render_migration_review(
        &self,
        review: &rmac_notes_runtime::MigrationReviewSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(mac::window())
            .child(
                div()
                    .w(px(460.0))
                    .p_6()
                    .v_flex()
                    .gap_3()
                    .rounded(px(14.0))
                    .bg(mac::raised())
                    .border_1()
                    .border_color(mac::separator())
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(20.0))
                            .font_weight(mac::BOLD)
                            .child("Bring your existing notes into rmac Notes?"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(mac::text_secondary())
                            .child(format!(
                                "Notes found {} notes in {} folders, with {} managed attachments and {} recovery files. The source stays untouched.",
                                review.notes,
                                review.folders,
                                review.managed_attachments,
                                review.recovery_files
                            )),
                    )
                    .when(!review.warnings.is_empty(), |element| {
                        element.child(
                            div()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(mac::warning_text())
                                .child(format!(
                                    "{} item{} need recovery attention after import.",
                                    review.warnings.len(),
                                    if review.warnings.len() == 1 { "" } else { "s" }
                                )),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("start-empty", "Not Now")
                                    .on_click(cx.listener(|this, _, _, cx| this.start_empty(cx))),
                            )
                            .child(
                                Button::new("accept-migration", "Import Notes")
                                    .primary()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.accept_migration(cx)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_draft_review(
        &self,
        review: &rmac_notes_runtime::DraftReviewSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.recovery_decision.is_some() || self.recovery_copy_pending.is_some();
        let warning_count = review
            .malformed
            .saturating_add(review.quarantined)
            .saturating_add(review.cleanup_pending);
        let mut card = div()
            .w(px(500.0))
            .p_6()
            .v_flex()
            .gap_3()
            .rounded(px(14.0))
            .bg(mac::raised())
            .border_1()
            .border_color(mac::separator());

        if let Some(draft) = review.drafts.first() {
            let note_id = draft.note_id;
            let note_title = self
                .session
                .snapshot()
                .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
                .map(|note| display_title(&note.title))
                .unwrap_or_else(|| "Deleted or unavailable note".into());
            let (heading, detail, decision, action_label) = match draft.kind {
                DraftRecoveryKind::Applicable => (
                    "Recover unsaved changes?",
                    "This recovery copy matches the durable note and can be restored safely.",
                    RecoveryDecision::RestoreOriginal,
                    "Restore",
                ),
                DraftRecoveryKind::Conflict => (
                    "Keep both versions?",
                    "The durable note changed after this recovery copy was written. Preserve the recovered text as a new note to avoid overwriting either version.",
                    RecoveryDecision::PreserveCopy,
                    "Keep as New Note",
                ),
                DraftRecoveryKind::Orphaned => (
                    "Preserve recovered text?",
                    "The original note is no longer available. Preserve this recovery copy as a new note before continuing.",
                    RecoveryDecision::PreserveCopy,
                    "Keep as New Note",
                ),
            };
            card = card
                .child(
                    div()
                        .text_size(rmac_ui::text_px(20.0))
                        .font_weight(mac::BOLD)
                        .child(heading),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(14.0))
                        .font_weight(mac::SEMIBOLD)
                        .child(note_title),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(mac::text_secondary())
                        .child(detail),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(mac::text_tertiary())
                        .child(format!(
                            "{} recovery {} remaining",
                            review.drafts.len(),
                            if review.drafts.len() == 1 {
                                "copy"
                            } else {
                                "copies"
                            }
                        )),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("close-recovery", "Review Later")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.request_close(window, cx)
                                })),
                        )
                        .child(
                            Button::new("discard-recovery", "Discard Recovery")
                                .destructive()
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.discard_draft(note_id, cx)
                                })),
                        )
                        .child(
                            Button::new("restore-recovery", action_label)
                                .primary()
                                .busy(busy)
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.restore_draft(note_id, decision, cx)
                                })),
                        ),
                );
        } else {
            let cannot_continue = review.unavailable || review.excessive;
            let detail = if cannot_continue {
                "Notes could not enumerate every recovery record safely. The records remain untouched; close Notes and resolve the storage problem before editing."
            } else {
                "No recoverable note text remains. Any malformed records were isolated and will not be treated as valid note content."
            };
            card = card
                .child(
                    div()
                        .text_size(rmac_ui::text_px(20.0))
                        .font_weight(mac::BOLD)
                        .child(if cannot_continue {
                            "Recovery needs attention"
                        } else {
                            "Recovery review complete"
                        }),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(mac::text_secondary())
                        .child(detail),
                )
                .when(warning_count != 0, |element| {
                    element.child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::warning_text())
                            .child(format!(
                                "{warning_count} recovery record operation{} reported attention.",
                                if warning_count == 1 { "" } else { "s" }
                            )),
                    )
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("close-recovery-notice", "Close Notes").on_click(
                                cx.listener(|this, _, window, cx| this.request_close(window, cx)),
                            ),
                        )
                        .when(!cannot_continue, |element| {
                            element.child(
                                Button::new("continue-recovery", "Continue")
                                    .primary()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.continue_after_recovery_notice(cx)
                                    })),
                            )
                        }),
                );
        }

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(mac::window())
            .child(card)
            .into_any_element()
    }

    fn render_status_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (message, actions) = match self.session.phase() {
            SessionPhase::Pending { reason, .. } => (
                pending_message(*reason),
                Some(("Retry", "Discard")),
            ),
            SessionPhase::Maintenance { .. } => (
                "Notes recovered the library but maintenance still needs attention. Editing is paused."
                    .to_string(),
                None,
            ),
            _ => match &self.message {
                Some(message) => (message.to_string(), None),
                None => return None,
            },
        };
        let mut banner = div()
            .h(px(38.0))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(mac::error_background())
            .border_b_1()
            .border_color(mac::error_border())
            .text_size(rmac_ui::text_px(12.0))
            .text_color(mac::danger())
            .child(div().flex_1().child(message));
        if let Some((retry, discard)) = actions {
            banner = banner
                .child(
                    Button::new("retry-pending", retry)
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.retry_pending(cx))),
                )
                .child(
                    Button::new("discard-pending", discard)
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.discard_pending(cx))),
                );
        }
        Some(banner.into_any_element())
    }
}

impl Drop for NotesView {
    fn drop(&mut self) {
        if !self.closing {
            if let Some(worker) = &self.worker {
                let _ = worker.try_send(WorkerCommand::Shutdown);
            }
        }
    }
}

impl Render for NotesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.session.phase() {
            SessionPhase::Starting if self.message.is_none() => centered_state(
                "Opening Notes…",
                "Checking the private library and recovery state.",
            ),
            SessionPhase::Starting => centered_state(
                "Notes could not start",
                self.message
                    .clone()
                    .unwrap_or_else(|| "The private Notes worker is unavailable.".into()),
            ),
            SessionPhase::MigrationReview(review) => self.render_migration_review(review, cx),
            SessionPhase::Failed(error) => {
                centered_state("Notes could not open", error.to_string())
            }
            SessionPhase::Stopped if !self.closing => centered_state(
                "Notes stopped",
                "Close and reopen the app to reconnect to the private library.",
            ),
            SessionPhase::Ready if self.recovery_review_is_blocking() => self
                .session
                .draft_review()
                .map(|review| self.render_draft_review(review, cx))
                .unwrap_or_else(|| {
                    centered_state("Recovery unavailable", "Close and reopen Notes safely.")
                }),
            SessionPhase::Ready
            | SessionPhase::Maintenance { .. }
            | SessionPhase::Pending { .. }
            | SessionPhase::Stopped => div()
                .size_full()
                .v_flex()
                .child(self.render_toolbar(cx))
                .when_some(self.render_status_banner(cx), |element, banner| {
                    element.child(banner)
                })
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .min_h(px(0.0))
                        .child(self.render_sidebar(cx))
                        .child(self.render_note_list(cx))
                        .child(div().flex_1().min_w(px(0.0)).child(self.render_editor(cx))),
                )
                .into_any_element(),
        };

        div()
            .track_focus(&self.focus)
            .key_context("Notes")
            .on_action(cx.listener(|this, _: &ComposeNote, _, cx| this.create_note(cx)))
            .on_action(cx.listener(|this, _: &CreateFolder, _, cx| this.create_folder(cx)))
            .on_action(cx.listener(|this, _: &TrashOrRestore, _, cx| this.trash_or_restore(cx)))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| this.toggle_pin(cx)))
            .on_action(
                cx.listener(|this, _: &SortByEdited, _, cx| this.set_sort(SortOrder::Edited, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SortByCreated, _, cx| this.set_sort(SortOrder::Created, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SortByTitle, _, cx| this.set_sort(SortOrder::Title, cx)),
            )
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.request_close(window, cx)
            }))
            .size_full()
            .bg(mac::window())
            .text_color(mac::text())
            .child(content)
    }
}

fn bridge_worker_events(
    events: NotesWorkerEvents,
) -> io::Result<async_channel::Receiver<WorkerEvent>> {
    let (sender, receiver) = async_channel::bounded(EVENT_CAPACITY);
    thread::Builder::new()
        .name("rmac-notes-ui-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if sender.send_blocking(event).is_err() {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

fn folder_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    icon: IconName,
    count: usize,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(px(6.0))
        .when(selected, |element: Stateful<Div>| {
            element.bg(mac::sidebar_selection())
        })
        .when(!selected, |element: Stateful<Div>| {
            element.hover(|hover| hover.bg(mac::hover()))
        })
        .child(
            Icon::new(icon)
                .text_color(mac::notes_accent())
                .with_size(Size::Small),
        )
        .child(
            div()
                .flex_1()
                .truncate()
                .text_size(rmac_ui::text_px(13.0))
                .child(label.into()),
        )
        .child(
            div()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(mac::text_tertiary())
                .child(count.to_string()),
        )
        .on_click(on_click)
}

fn centered_state(title: impl Into<SharedString>, detail: impl Into<SharedString>) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(mac::window())
        .child(
            div()
                .w(px(440.0))
                .v_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(18.0))
                        .font_weight(mac::SEMIBOLD)
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(mac::text_secondary())
                        .text_center()
                        .child(detail.into()),
                ),
        )
        .into_any_element()
}

fn worker_failure_message(failure: WorkerFailure) -> String {
    match failure {
        WorkerFailure::WrongPhase => "Notes is not ready for that action".into(),
        WorkerFailure::CommitPending => "Resolve the pending Notes change first".into(),
        WorkerFailure::Scheduler(error) => error.to_string(),
        WorkerFailure::Mutation(error) => error.to_string(),
        WorkerFailure::Storage(error) => error.to_string(),
        WorkerFailure::TextImport(error) => error.to_string(),
        WorkerFailure::Draft(error) => error.to_string(),
        WorkerFailure::MissingDraft => "That recovery draft is no longer available".into(),
        WorkerFailure::ExportPlan(error) => error.to_string(),
        WorkerFailure::Export(error) => error.to_string(),
        WorkerFailure::BundleImport(error) => error.to_string(),
        WorkerFailure::BundlePlan(error) => error.to_string(),
        WorkerFailure::MissingBundleImportReview => {
            "Review the selected Notes bundle again before importing".into()
        }
    }
}

fn pending_message(reason: PendingReason) -> String {
    match reason {
        PendingReason::Store(error) => error.to_string(),
        PendingReason::AcceptedStateChanged => {
            "The durable Notes library changed while this local change was pending".into()
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX - 1)
        .max(1)
}

fn take_counter(counter: &mut u64) -> Option<u64> {
    let current = *counter;
    if current == 0 || current == u64::MAX {
        return None;
    }
    *counter = current + 1;
    Some(current)
}

fn unique_folder_name(existing: &[String]) -> String {
    for suffix in 1..=existing.len().saturating_add(2) {
        let candidate = if suffix == 1 {
            "New Folder".to_string()
        } else {
            format!("New Folder {suffix}")
        };
        if !existing
            .iter()
            .any(|name| name == &candidate.to_lowercase())
        {
            return candidate;
        }
    }
    "Imported Notes".into()
}

fn display_title(title: &str) -> SharedString {
    if title.trim().is_empty() {
        "New Note".into()
    } else {
        title.to_string().into()
    }
}

fn snippet(body: &str) -> SharedString {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        "No additional text".into()
    } else {
        compact.chars().take(90).collect::<String>().into()
    }
}

fn tag_pill(tag: String) -> impl IntoElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded(px(5.0))
        .bg(mac::control_fill())
        .text_size(rmac_ui::text_px(10.0))
        .text_color(mac::text_secondary())
        .child(format!("#{tag}"))
}

fn date_label(unix_ms: u64) -> SharedString {
    let time = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(unix_ms))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let date: DateTime<Local> = time.into();
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(date.date_naive())
        .num_days();
    if days == 0 {
        let hour = date.hour();
        let (hour, suffix) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        format!("{hour}:{:02} {suffix}", date.minute()).into()
    } else if days == 1 {
        "Yesterday".into()
    } else if (2..7).contains(&days) {
        date.format("%A").to_string().into()
    } else {
        format!("{}/{}/{:02}", date.month(), date.day(), date.year() % 100).into()
    }
}

fn main() {
    rmac_ui::boot("Notes", 1080.0, 720.0, |window, cx| {
        NotesView::new(window, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_never_wrap_or_emit_zero() {
        let mut counter = 1;
        assert_eq!(take_counter(&mut counter), Some(1));
        assert_eq!(take_counter(&mut counter), Some(2));
        counter = u64::MAX;
        assert_eq!(take_counter(&mut counter), None);
    }

    #[test]
    fn folder_names_are_case_insensitive_and_deterministic() {
        let existing = vec!["new folder".into(), "new folder 2".into()];
        assert_eq!(unique_folder_name(&existing), "New Folder 3");
    }

    #[test]
    fn empty_note_metadata_has_private_safe_fallbacks() {
        assert_eq!(display_title(""), SharedString::from("New Note"));
        assert_eq!(snippet("  \n"), SharedString::from("No additional text"));
    }
}
