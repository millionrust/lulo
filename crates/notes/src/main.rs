//! rmac Notes live application.
//!
//! The GPUI view is a projection of `rmac-notes-runtime`: it never scans or
//! writes the library directly. Stable IDs, accepted snapshots, recovery, and
//! the single writer remain authoritative off the UI thread.

use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    actions, div, img, prelude::FluentBuilder as _, px, AnyElement, AppContext as _, Context, Div,
    Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding, ObjectFit,
    ParentElement, Render, RenderImage, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled, StyledImage as _, Window,
};
use gpui_component::{Icon, IconName, Sizable as _, Size, StyledExt as _};
use rmac_editor::InputState;
use rmac_notes_runtime::{
    ActionRequest, ActionResult, BundleImportAcceptRequest, BundleImportReviewRequest,
    DraftRecoveryKind, EditGeneration, ExportRequest, LibraryAction, NotesPreviewSession,
    NotesPreviewWorker, NotesPreviewWorkerClient, NotesPreviewWorkerEvents, NotesSearchSession,
    NotesSearchWorker, NotesSearchWorkerClient, NotesSearchWorkerEvents, NotesSession, NotesWorker,
    NotesWorkerClient, NotesWorkerEvents, PreviewState, PreviewWorkerEvent, PreviewWorkerSendError,
    ScheduledEdit, SearchState, SearchWorkerEvent, SearchWorkerSendError, SessionPhase,
    WorkerCommand, WorkerEvent, WorkerFailure, WorkerSendError, EVENT_CAPACITY, MAX_SEARCH_RESULTS,
    PREVIEW_EVENT_CAPACITY, SEARCH_EVENT_CAPACITY,
};
use rmac_notes_storage::{
    resolve_notes_paths, DecodedImagePreview, ExportFormat, ExportOutcome, MarkdownImportReview,
    PendingReason, PreviewSize,
};
use rmac_notes_store::{
    AttachmentId, BundleCollisionPolicy, BundleImportReview, ExportScope, FolderId, NewNote,
    NoteChanges, NoteId, NoteRecord, SortOrder, MAX_TAGS_PER_NOTE, MAX_TAG_BYTES,
};
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
        SortByTitle,
        FocusSearch,
        ExportNotes,
        RenameSelectedFolder,
        DeleteSelectedFolder
    ]
);

struct NotesView {
    worker: Option<NotesWorkerClient>,
    search_worker: Option<NotesSearchWorkerClient>,
    preview_worker: Option<NotesPreviewWorkerClient>,
    session: NotesSession,
    search: NotesSearchSession,
    preview: NotesPreviewSession,
    preview_image: Option<Arc<RenderImage>>,
    selected_attachment: Option<AttachmentId>,
    search_query: Entity<InputState>,
    folder_name_input: Entity<InputState>,
    title: Entity<InputState>,
    tags: Entity<InputState>,
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
    folder_dialog: Option<FolderDialog>,
    purge_dialog: Option<PurgeDialog>,
    move_dialog: Option<MoveDialog>,
    attachment_dialog: Option<AttachmentDialog>,
    attachment_chooser_open: bool,
    attachment_request_id: Option<u64>,
    attachment_remove_request: Option<(u64, AttachmentId)>,
    orphan_collection_request: Option<(u64, AttachmentId)>,
    note_import_chooser_open: bool,
    note_import_request_id: Option<u64>,
    markdown_import_review: Option<(u64, u64, MarkdownImportReview)>,
    markdown_import_action_request_id: Option<u64>,
    export_dialog: Option<ExportDialog>,
    export_chooser_open: bool,
    export_request_id: Option<u64>,
    bundle_chooser_open: bool,
    bundle_review_request_id: Option<u64>,
    bundle_review: Option<(u64, BundleImportReview)>,
    bundle_action_request_id: Option<u64>,
    bundle_import_completion: Option<BundleImportCompletion>,
    search_shutdown_requested: bool,
    preview_shutdown_requested: bool,
    closing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryDecision {
    RestoreOriginal,
    PreserveCopy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FolderDialog {
    Rename(FolderId),
    Delete(FolderId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PurgeDialog {
    Note {
        note_id: NoteId,
        note_revision: u64,
        attachment_count: usize,
        attachment_bytes: u64,
    },
    EmptyTrash {
        library_revision: u64,
        note_count: usize,
        attachment_count: usize,
        attachment_bytes: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MoveDialog {
    note_id: NoteId,
    note_revision: u64,
    current_folder: Option<FolderId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachmentDialog {
    Remove {
        note_id: NoteId,
        note_revision: u64,
        attachment_id: AttachmentId,
        attachment_revision: u64,
        byte_len: u64,
    },
    CollectOrphan {
        attachment_id: AttachmentId,
        attachment_revision: u64,
        byte_len: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusActions {
    Pending,
    OrphanCleanup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExportReview {
    library_revision: u64,
    scope: ExportScope,
    note_count: usize,
    attachment_count: usize,
    markdown_bytes: u64,
    attachment_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportDialog {
    Review(ExportReview),
    Complete(ExportOutcome),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BundleImportCompletion {
    folder_count: usize,
    note_count: usize,
    attachment_count: usize,
    attachment_bytes: u64,
    maintenance_pending: bool,
}

struct PreviewBridgeEvent {
    event: PreviewWorkerEvent,
    rendered: Option<Arc<RenderImage>>,
}

impl NotesView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("cmd-n", ComposeNote, Some("Notes")),
            KeyBinding::new("shift-cmd-n", CreateFolder, Some("Notes")),
            KeyBinding::new("cmd-backspace", TrashOrRestore, Some("Notes")),
            KeyBinding::new("cmd-f", FocusSearch, Some("Notes")),
            KeyBinding::new("cmd-shift-e", ExportNotes, Some("Notes")),
        ]);

        let search_query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let folder_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder Name"));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let tags = cx.new(|cx| InputState::new(window, cx).placeholder("Tags"));
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
        cx.subscribe(&tags, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_current_edit(cx);
            }
        })
        .detach();
        cx.subscribe(&search_query, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.dispatch_search(cx);
            }
        })
        .detach();
        cx.subscribe(&folder_name_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.commit_folder_rename(cx);
            }
        })
        .detach();

        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut view = Self {
            worker: None,
            search_worker: None,
            preview_worker: None,
            session: NotesSession::new(),
            search: NotesSearchSession::new(),
            preview: NotesPreviewSession::new(),
            preview_image: None,
            selected_attachment: None,
            search_query,
            folder_name_input,
            title,
            tags,
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
            folder_dialog: None,
            purge_dialog: None,
            move_dialog: None,
            attachment_dialog: None,
            attachment_chooser_open: false,
            attachment_request_id: None,
            attachment_remove_request: None,
            orphan_collection_request: None,
            note_import_chooser_open: false,
            note_import_request_id: None,
            markdown_import_review: None,
            markdown_import_action_request_id: None,
            export_dialog: None,
            export_chooser_open: false,
            export_request_id: None,
            bundle_chooser_open: false,
            bundle_review_request_id: None,
            bundle_review: None,
            bundle_action_request_id: None,
            bundle_import_completion: None,
            search_shutdown_requested: false,
            preview_shutdown_requested: false,
            closing: false,
        };

        let notes_paths = match resolve_notes_paths() {
            Ok(paths) => Some(paths),
            Err(error) => {
                view.message = Some(error.to_string().into());
                None
            }
        };

        if let Some(paths) = notes_paths.as_ref() {
            match NotesWorker::start(paths.clone())
                .map_err(|error| error.to_string())
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

            if let Ok((client, receiver)) =
                NotesPreviewWorker::start(paths.data_root().to_path_buf())
                    .map_err(|error| error.to_string())
                    .and_then(|worker| {
                        let (client, events) = worker.into_parts();
                        bridge_preview_events(events)
                            .map(|receiver| (client, receiver))
                            .map_err(|error| {
                                format!("Notes could not start its preview bridge: {error}")
                            })
                    })
            {
                view.preview_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, _window, cx| this.apply_preview_event(event, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
        }

        match NotesSearchWorker::start()
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                let (client, events) = worker.into_parts();
                bridge_search_events(events)
                    .map(|receiver| (client, receiver))
                    .map_err(|error| format!("Notes could not start its search bridge: {error}"))
            }) {
            Ok((client, receiver)) => {
                view.search_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, window, cx| {
                                this.apply_search_event(event, window, cx)
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(message) => {
                if view.message.is_none() {
                    view.message = Some(message.into());
                }
            }
        }

        view
    }

    fn apply_search_event(
        &mut self,
        event: SearchWorkerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(&event, SearchWorkerEvent::Stopped { .. }) {
            self.search.cancel();
            let unexpected = !self.closing && !self.search_shutdown_requested;
            self.search_shutdown_requested = true;
            self.search_worker = None;
            if unexpected {
                self.message = Some("Notes search stopped unexpectedly".into());
            }
            cx.notify();
            return;
        }
        if event.project(&mut self.search) {
            if self.search.state() == SearchState::Results && self.is_interactive_ready() {
                if let Some(note_id) = self.search.selected() {
                    let previous = self.session.selected_note_id();
                    self.session
                        .select_folder(rmac_notes_runtime::FolderSelection::All);
                    if self.session.select_note(note_id)
                        && (previous != Some(note_id) || self.latest_local_generation.is_none())
                    {
                        self.sync_editor(window, cx);
                    }
                }
            }
            cx.notify();
        }
    }

    fn apply_preview_event(&mut self, bridged: PreviewBridgeEvent, cx: &mut Context<Self>) {
        if matches!(&bridged.event, PreviewWorkerEvent::Stopped { .. }) {
            let unexpected = !self.closing && !self.preview_shutdown_requested;
            self.preview.clear();
            self.preview_image = None;
            self.preview_shutdown_requested = true;
            self.preview_worker = None;
            if unexpected {
                self.message = Some("Notes image previews stopped unexpectedly".into());
            }
            cx.notify();
            return;
        }
        if bridged.event.project(&mut self.preview) {
            self.preview_image = bridged.rendered;
            cx.notify();
        }
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
        let refresh_search = matches!(&event, WorkerEvent::Ready(_) | WorkerEvent::Accepted(_));
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
        let exported = match &event {
            WorkerEvent::Exported(exported) => Some(*exported),
            _ => None,
        };
        let worker_ready = matches!(&event, WorkerEvent::Ready(_));
        let bundle_reviewed = match &event {
            WorkerEvent::BundleImportReviewed(reviewed) => Some(*reviewed),
            _ => None,
        };
        let markdown_reviewed = match &event {
            WorkerEvent::MarkdownImportReviewed(reviewed) => Some(*reviewed),
            _ => None,
        };
        let markdown_review_discarded = match &event {
            WorkerEvent::MarkdownImportReviewDiscarded {
                request_id,
                review_request_id,
            } => Some((*request_id, *review_request_id)),
            _ => None,
        };
        let bundle_review_discarded = match &event {
            WorkerEvent::BundleImportReviewDiscarded {
                request_id,
                review_request_id,
            } => Some((*request_id, *review_request_id)),
            _ => None,
        };
        let imported_bundle = match &event {
            WorkerEvent::Accepted(accepted) => match accepted.result {
                ActionResult::ImportedBundle {
                    folder_count,
                    note_count,
                    attachment_count,
                    attachment_bytes,
                } => Some((
                    accepted.request_id,
                    BundleImportCompletion {
                        folder_count,
                        note_count,
                        attachment_count,
                        attachment_bytes,
                        maintenance_pending: accepted.commit.maintenance_pending,
                    },
                )),
                _ => None,
            },
            _ => None,
        };
        let attached_image = match &event {
            WorkerEvent::Accepted(accepted) => match accepted.result {
                ActionResult::AttachedImage {
                    note_id,
                    attachment_id,
                    ..
                } => Some((note_id, attachment_id)),
                _ => None,
            },
            _ => None,
        };
        let removed_attachment = match &event {
            WorkerEvent::Accepted(accepted) => match accepted.result {
                ActionResult::AttachmentReferenceRemoved { attachment_id, .. } => {
                    Some((accepted.request_id, attachment_id))
                }
                _ => None,
            },
            _ => None,
        };
        let chain_orphan = removed_attachment.is_some_and(|(request_id, attachment_id)| {
            self.attachment_remove_request == Some((request_id, attachment_id))
        });
        if accepted_request_id
            .or(rejected_request_id)
            .is_some_and(|request_id| self.attachment_request_id == Some(request_id))
            || matches!(&event, WorkerEvent::Ready(_)) && self.attachment_request_id.is_some()
        {
            self.attachment_request_id = None;
        }
        if accepted_request_id
            .or(rejected_request_id)
            .is_some_and(|request_id| {
                self.attachment_remove_request
                    .is_some_and(|(pending, _)| pending == request_id)
            })
            || matches!(&event, WorkerEvent::Ready(_)) && self.attachment_remove_request.is_some()
        {
            self.attachment_remove_request = None;
        }
        if accepted_request_id
            .or(rejected_request_id)
            .is_some_and(|request_id| {
                self.orphan_collection_request
                    .is_some_and(|(pending, _)| pending == request_id)
            })
            || matches!(&event, WorkerEvent::Ready(_)) && self.orphan_collection_request.is_some()
        {
            self.orphan_collection_request = None;
        }
        if accepted_request_id
            .or(rejected_request_id)
            .is_some_and(|request_id| self.note_import_request_id == Some(request_id))
        {
            self.note_import_request_id = None;
            self.markdown_import_review = None;
        }
        let tracked_export =
            exported.is_some_and(|exported| self.export_request_id == Some(exported.request_id));
        if tracked_export
            || rejected_request_id
                .is_some_and(|request_id| self.export_request_id == Some(request_id))
            || matches!(&event, WorkerEvent::Ready(_)) && self.export_request_id.is_some()
        {
            self.export_request_id = None;
        }
        let tracked_bundle_discard =
            bundle_review_discarded.is_some_and(|(request_id, review_request_id)| {
                self.bundle_action_request_id == Some(request_id)
                    && self.bundle_review_request_id == Some(review_request_id)
            });
        let tracked_bundle_import = imported_bundle
            .is_some_and(|(request_id, _)| self.bundle_action_request_id == Some(request_id));
        let tracked_markdown_discard =
            markdown_review_discarded.is_some_and(|(request_id, review_request_id)| {
                self.markdown_import_action_request_id == Some(request_id)
                    && self.note_import_request_id == Some(review_request_id)
            });
        let tracked_markdown_import = matches!(
            &event,
            WorkerEvent::Accepted(accepted)
                if self.markdown_import_action_request_id == Some(accepted.request_id)
                    && matches!(accepted.result, ActionResult::ImportedNote { .. })
        );
        if rejected_request_id
            .is_some_and(|request_id| self.markdown_import_action_request_id == Some(request_id))
        {
            self.markdown_import_action_request_id = None;
        }
        if rejected_request_id
            .is_some_and(|request_id| self.bundle_action_request_id == Some(request_id))
        {
            self.bundle_action_request_id = None;
        }
        if rejected_request_id.is_some_and(|request_id| {
            self.bundle_review_request_id == Some(request_id)
                && self.bundle_action_request_id != Some(request_id)
        }) {
            self.bundle_review_request_id = None;
            self.bundle_review = None;
        }
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
        if let Some(reviewed) = markdown_reviewed
            .filter(|reviewed| self.note_import_request_id == Some(reviewed.request_id))
        {
            self.markdown_import_review = Some((
                reviewed.request_id,
                reviewed.base_library_revision,
                reviewed.review,
            ));
        }
        if tracked_markdown_discard || tracked_markdown_import {
            self.note_import_request_id = None;
            self.markdown_import_review = None;
            self.markdown_import_action_request_id = None;
        }
        if worker_ready
            && self.markdown_import_action_request_id.is_some()
            && self.note_import_request_id.is_some()
        {
            self.note_import_request_id = None;
            self.markdown_import_review = None;
            self.markdown_import_action_request_id = None;
        }
        if let Some(reviewed) = bundle_reviewed
            .filter(|reviewed| self.bundle_review_request_id == Some(reviewed.request_id))
        {
            self.bundle_review = Some((reviewed.request_id, reviewed.review));
        }
        if tracked_bundle_discard {
            self.bundle_review_request_id = None;
            self.bundle_review = None;
            self.bundle_action_request_id = None;
        }
        if tracked_bundle_import {
            self.bundle_review_request_id = None;
            self.bundle_review = None;
            self.bundle_action_request_id = None;
            self.bundle_import_completion = imported_bundle.map(|(_, completion)| completion);
        }
        if worker_ready
            && self.bundle_action_request_id.is_some()
            && self.bundle_review_request_id.is_some()
        {
            self.bundle_review_request_id = None;
            self.bundle_review = None;
            self.bundle_action_request_id = None;
        }
        if let Some(ExportDialog::Review(review)) = self.export_dialog {
            if self
                .session
                .snapshot()
                .is_none_or(|snapshot| snapshot.revision != review.library_revision)
            {
                self.export_dialog = None;
                self.message = Some(
                    "The Notes library changed. Review the export again before choosing a destination."
                        .into(),
                );
            }
        }
        if let Some((note_id, attachment_id)) = attached_image {
            if self.session.selected_note_id() == Some(note_id) {
                self.selected_attachment = Some(attachment_id);
            }
        }
        let orphan_to_collect = chain_orphan.then(|| {
            removed_attachment
                .expect("a chained orphan comes from an accepted removal")
                .1
        });
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
        if let Some(attachment_id) = orphan_to_collect {
            self.queue_current_orphan_collection(attachment_id, cx);
        }
        if tracked_export {
            self.export_dialog = exported.map(|exported| ExportDialog::Complete(exported.outcome));
            self.message = None;
        }
        if refresh_search {
            self.dispatch_search(cx);
        }
        cx.notify();
    }

    fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (title, tags, body) = self
            .session
            .selected_note()
            .map(|note| (note.title.clone(), note.tags.join(", "), note.body.clone()))
            .unwrap_or_default();
        self.applying_snapshot = true;
        self.title
            .update(cx, |state, cx| state.set_value(title, window, cx));
        self.tags
            .update(cx, |state, cx| state.set_value(tags, window, cx));
        self.body
            .update(cx, |state, cx| state.set_value(body, window, cx));
        self.applying_snapshot = false;
        self.sync_attachment_preview(false, cx);
    }

    fn sync_attachment_preview(&mut self, force: bool, cx: &mut Context<Self>) {
        let candidate = self.session.snapshot().and_then(|snapshot| {
            let note = self.session.selected_note()?;
            let selected = self
                .selected_attachment
                .filter(|id| note.attachments.contains(id))
                .or_else(|| note.attachments.first().copied());
            let attachment_id = selected?;
            let attachment = snapshot
                .attachments
                .iter()
                .find(|attachment| attachment.id == attachment_id && !attachment.deleted)?
                .clone();
            Some((snapshot.revision, attachment_id, attachment))
        });
        let Some((library_revision, attachment_id, attachment)) = candidate else {
            self.selected_attachment = None;
            self.preview.clear();
            self.preview_image = None;
            cx.notify();
            return;
        };
        self.selected_attachment = Some(attachment_id);
        let current_is_exact = match self.preview.state() {
            PreviewState::Loading {
                library_revision: current_revision,
                attachment_id: current_attachment,
                ..
            }
            | PreviewState::Unavailable {
                library_revision: current_revision,
                attachment_id: current_attachment,
                ..
            } => *current_revision == library_revision && *current_attachment == attachment_id,
            PreviewState::Ready {
                library_revision: current_revision,
                image,
                ..
            } => *current_revision == library_revision && image.attachment_id() == attachment_id,
            PreviewState::Empty => false,
        };
        if current_is_exact && !force {
            return;
        }
        let Some(worker) = self.preview_worker.clone() else {
            self.preview.clear();
            self.preview_image = None;
            cx.notify();
            return;
        };
        let target = match PreviewSize::new(720, 360) {
            Ok(target) => target,
            Err(error) => {
                self.preview.clear();
                self.preview_image = None;
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        let request = match self.preview.request(library_revision, attachment, target) {
            Ok(request) => request,
            Err(error) => {
                self.preview.clear();
                self.preview_image = None;
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        self.preview_image = None;
        if let Err(error) = worker.try_run(request) {
            self.preview.clear();
            self.message = Some(error.to_string().into());
        }
        cx.notify();
    }

    fn select_attachment_preview(&mut self, attachment_id: AttachmentId, cx: &mut Context<Self>) {
        let belongs_to_note = self
            .session
            .selected_note()
            .is_some_and(|note| note.attachments.contains(&attachment_id));
        if !belongs_to_note {
            return;
        }
        self.selected_attachment = Some(attachment_id);
        self.sync_attachment_preview(true, cx);
    }

    fn retry_attachment_preview(&mut self, cx: &mut Context<Self>) {
        self.sync_attachment_preview(true, cx);
    }

    fn first_orphaned_attachment(&self) -> Option<&rmac_notes_store::AttachmentRecord> {
        self.session
            .snapshot()?
            .attachments
            .iter()
            .find(|attachment| attachment.deleted)
    }

    fn begin_attachment_removal(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message =
                Some("Wait for this note to finish saving before removing a photo".into());
            cx.notify();
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            return;
        };
        let Some(attachment_id) = self.selected_attachment else {
            return;
        };
        if !note.attachments.contains(&attachment_id) {
            return;
        }
        let Some(attachment) = self.session.snapshot().and_then(|snapshot| {
            snapshot.attachments.iter().find(|attachment| {
                attachment.id == attachment_id
                    && attachment.note_id == note.id
                    && !attachment.deleted
            })
        }) else {
            return;
        };
        self.attachment_dialog = Some(AttachmentDialog::Remove {
            note_id: note.id,
            note_revision: note.revision,
            attachment_id,
            attachment_revision: attachment.revision,
            byte_len: attachment.byte_len,
        });
        cx.notify();
    }

    fn begin_orphan_cleanup(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(attachment) = self.first_orphaned_attachment() else {
            return;
        };
        self.attachment_dialog = Some(AttachmentDialog::CollectOrphan {
            attachment_id: attachment.id,
            attachment_revision: attachment.revision,
            byte_len: attachment.byte_len,
        });
        cx.notify();
    }

    fn confirm_attachment_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.attachment_dialog.take() else {
            return;
        };
        match dialog {
            AttachmentDialog::Remove {
                note_id,
                note_revision,
                attachment_id,
                attachment_revision,
                ..
            } => self.queue_attachment_removal(
                note_id,
                note_revision,
                attachment_id,
                attachment_revision,
                cx,
            ),
            AttachmentDialog::CollectOrphan {
                attachment_id,
                attachment_revision,
                ..
            } => self.queue_orphan_collection(attachment_id, attachment_revision, cx),
        }
    }

    fn cancel_attachment_dialog(&mut self, cx: &mut Context<Self>) {
        self.attachment_dialog = None;
        cx.notify();
    }

    fn queue_attachment_removal(
        &mut self,
        note_id: NoteId,
        expected_note_revision: u64,
        attachment_id: AttachmentId,
        expected_attachment_revision: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() || self.latest_local_generation.is_some() {
            self.message = Some(
                "The note changed before the image could be removed. Review the removal again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(note) = self
            .session
            .selected_note()
            .filter(|note| note.id == note_id && !note.deleted)
        else {
            self.message = Some("The selected note is no longer available".into());
            cx.notify();
            return;
        };
        let modified_unix_ms = now_unix_ms()
            .max(note.created_unix_ms)
            .max(note.modified_unix_ms);
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ActionRequest::new(
            request_id,
            LibraryAction::RemoveAttachmentReference {
                note_id,
                expected_note_revision,
                attachment_id,
                expected_attachment_revision,
                modified_unix_ms,
            },
        ) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.attachment_remove_request = Some((request_id, attachment_id));
            self.message = None;
            cx.notify();
        }
    }

    fn queue_current_orphan_collection(
        &mut self,
        attachment_id: AttachmentId,
        cx: &mut Context<Self>,
    ) {
        let Some(attachment_revision) = self
            .session
            .snapshot()
            .and_then(|snapshot| {
                snapshot
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == attachment_id && attachment.deleted)
            })
            .map(|attachment| attachment.revision)
        else {
            cx.notify();
            return;
        };
        self.queue_orphan_collection(attachment_id, attachment_revision, cx);
    }

    fn queue_orphan_collection(
        &mut self,
        attachment_id: AttachmentId,
        expected_attachment_revision: u64,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.session.phase(), SessionPhase::Ready) || self.attachment_action_pending()
        {
            cx.notify();
            return;
        }
        let exact_orphan_exists = self.session.snapshot().is_some_and(|snapshot| {
            snapshot.attachments.iter().any(|attachment| {
                attachment.id == attachment_id
                    && attachment.deleted
                    && attachment.revision == expected_attachment_revision
            })
        });
        if !exact_orphan_exists {
            self.message = Some(
                "The removed attachment changed before cleanup. Review the cleanup again.".into(),
            );
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ActionRequest::new(
            request_id,
            LibraryAction::CollectOrphanedAttachment {
                attachment_id,
                expected_attachment_revision,
            },
        ) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.orphan_collection_request = Some((request_id, attachment_id));
            self.message = None;
            cx.notify();
        }
    }

    fn dispatch_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query.read(cx).value().to_string();
        if query.trim().is_empty() {
            self.search.cancel();
            cx.notify();
            return;
        }
        let Some(snapshot) = self.session.snapshot().cloned() else {
            self.search.cancel();
            return;
        };
        let Some(worker) = self.search_worker.clone() else {
            self.search.cancel();
            self.message = Some("Notes search is unavailable".into());
            cx.notify();
            return;
        };
        let request = match self
            .search
            .begin(query, MAX_SEARCH_RESULTS, snapshot.revision)
        {
            Ok(Some(request)) => request,
            Ok(None) => {
                cx.notify();
                return;
            }
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if let Err(error) = worker.try_run(snapshot, request) {
            self.search.cancel();
            self.message = Some(error.to_string().into());
        }
        cx.notify();
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.folder_dialog.is_some()
            || self.purge_dialog.is_some()
            || self.move_dialog.is_some()
            || self.attachment_dialog.is_some()
            || self.export_dialog.is_some()
            || self.attachment_chooser_open
            || self.note_import_chooser_open
            || self.note_import_request_id.is_some()
            || self.markdown_import_review.is_some()
            || self.markdown_import_action_request_id.is_some()
            || self.export_chooser_open
            || self.export_request_id.is_some()
            || self.bundle_chooser_open
            || self.bundle_review_request_id.is_some()
            || self.bundle_review.is_some()
            || self.bundle_action_request_id.is_some()
            || self.bundle_import_completion.is_some()
            || self.attachment_action_pending()
        {
            return;
        }
        self.search_query
            .update(cx, |state, cx| state.focus(window, cx));
    }

    fn select_search_result(
        &mut self,
        note_id: NoteId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() || !self.search.select(note_id) {
            return;
        }
        let previous = self.session.selected_note_id();
        self.session
            .select_folder(rmac_notes_runtime::FolderSelection::All);
        if self.session.select_note(note_id)
            && (previous != Some(note_id) || self.latest_local_generation.is_none())
        {
            self.sync_editor(window, cx);
        }
        cx.notify();
    }

    fn schedule_current_edit(&mut self, cx: &mut Context<Self>) {
        if self.applying_snapshot || !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note() else {
            return;
        };
        if note.deleted {
            return;
        }
        let note_id = note.id;
        let expected_revision = note.revision;
        let created_unix_ms = note.created_unix_ms;
        let previous_modified = note.modified_unix_ms;
        let accepted_tags = note.tags.clone();
        let title = self.title.read(cx).value().to_string();
        let body = self.body.read(cx).value().to_string();
        let tag_text = self.tags.read(cx).value().to_string();
        let tags = match parse_tags(&tag_text) {
            Ok(tags) => tags,
            Err(message) => {
                self.message = Some(message.into());
                cx.notify();
                return;
            }
        };
        if title == note.title && body == note.body && tags == accepted_tags {
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
        self.tags.update(cx, |state, cx| {
            state.set_value(changes.tags.join(", "), window, cx)
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
        matches!(self.session.phase(), SessionPhase::Ready)
            && !self.recovery_review_is_blocking()
            && self.folder_dialog.is_none()
            && self.purge_dialog.is_none()
            && self.move_dialog.is_none()
            && self.attachment_dialog.is_none()
            && self.export_dialog.is_none()
            && !self.attachment_chooser_open
            && !self.note_import_chooser_open
            && self.note_import_request_id.is_none()
            && self.markdown_import_review.is_none()
            && self.markdown_import_action_request_id.is_none()
            && !self.export_chooser_open
            && self.export_request_id.is_none()
            && !self.bundle_chooser_open
            && self.bundle_review_request_id.is_none()
            && self.bundle_review.is_none()
            && self.bundle_action_request_id.is_none()
            && self.bundle_import_completion.is_none()
            && !self.attachment_action_pending()
    }

    fn attachment_action_pending(&self) -> bool {
        self.attachment_request_id.is_some()
            || self.attachment_remove_request.is_some()
            || self.orphan_collection_request.is_some()
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
        let previous = self.session.selected_note_id();
        self.session.select_folder(folder);
        if self.session.selected_note_id() != previous || self.latest_local_generation.is_none() {
            self.sync_editor(window, cx);
        }
        cx.notify();
    }

    fn select_note(&mut self, note_id: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let previous = self.session.selected_note_id();
        if self.session.select_note(note_id) {
            if previous != Some(note_id) || self.latest_local_generation.is_none() {
                self.sync_editor(window, cx);
            }
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

    fn begin_folder_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let rmac_notes_runtime::FolderSelection::Folder(folder_id) =
            self.session.folder_selection()
        else {
            return;
        };
        let Some(name) = self
            .session
            .folders()
            .into_iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| folder.name.clone())
        else {
            return;
        };
        self.folder_name_input
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.folder_dialog = Some(FolderDialog::Rename(folder_id));
        self.folder_name_input
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn commit_folder_rename(&mut self, cx: &mut Context<Self>) {
        let Some(FolderDialog::Rename(folder_id)) = self.folder_dialog else {
            return;
        };
        let name = self.folder_name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.message = Some("A Notes folder name cannot be empty".into());
            cx.notify();
            return;
        }
        let Some(expected_revision) = self
            .session
            .folders()
            .into_iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| folder.revision)
        else {
            self.folder_dialog = None;
            self.message = Some("That Notes folder is no longer available".into());
            cx.notify();
            return;
        };
        self.folder_dialog = None;
        self.send_action(
            LibraryAction::RenameFolder {
                folder_id,
                expected_revision,
                name,
            },
            cx,
        );
        cx.notify();
    }

    fn begin_folder_delete(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if let rmac_notes_runtime::FolderSelection::Folder(folder_id) =
            self.session.folder_selection()
        {
            self.folder_dialog = Some(FolderDialog::Delete(folder_id));
            cx.notify();
        }
    }

    fn confirm_folder_delete(&mut self, cx: &mut Context<Self>) {
        let Some(FolderDialog::Delete(folder_id)) = self.folder_dialog else {
            return;
        };
        let Some(expected_revision) = self
            .session
            .folders()
            .into_iter()
            .find(|folder| folder.id == folder_id)
            .map(|folder| folder.revision)
        else {
            self.folder_dialog = None;
            self.message = Some("That Notes folder is no longer available".into());
            cx.notify();
            return;
        };
        self.folder_dialog = None;
        self.send_action(
            LibraryAction::DeleteFolder {
                folder_id,
                expected_revision,
            },
            cx,
        );
        cx.notify();
    }

    fn cancel_folder_dialog(&mut self, cx: &mut Context<Self>) {
        self.folder_dialog = None;
        cx.notify();
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

    fn begin_permanent_note_delete(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| note.deleted) else {
            return;
        };
        let note_id = note.id;
        let note_revision = note.revision;
        let Some(snapshot) = self.session.snapshot() else {
            return;
        };
        let mut attachment_count = 0_usize;
        let mut attachment_bytes = 0_u64;
        for attachment in snapshot
            .attachments
            .iter()
            .filter(|attachment| attachment.note_id == note_id)
        {
            attachment_count = attachment_count.saturating_add(1);
            let Some(total) = attachment_bytes.checked_add(attachment.byte_len) else {
                self.message = Some("The attachment deletion total is too large to review".into());
                cx.notify();
                return;
            };
            attachment_bytes = total;
        }
        self.purge_dialog = Some(PurgeDialog::Note {
            note_id,
            note_revision,
            attachment_count,
            attachment_bytes,
        });
        cx.notify();
    }

    fn begin_empty_trash(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(snapshot) = self.session.snapshot() else {
            return;
        };
        let note_ids = snapshot
            .notes
            .iter()
            .filter(|note| note.deleted)
            .map(|note| note.id)
            .collect::<BTreeSet<_>>();
        if note_ids.is_empty() {
            return;
        }
        let mut attachment_count = 0_usize;
        let mut attachment_bytes = 0_u64;
        for attachment in snapshot
            .attachments
            .iter()
            .filter(|attachment| note_ids.contains(&attachment.note_id))
        {
            attachment_count = attachment_count.saturating_add(1);
            let Some(total) = attachment_bytes.checked_add(attachment.byte_len) else {
                self.message = Some("The Trash deletion total is too large to review".into());
                cx.notify();
                return;
            };
            attachment_bytes = total;
        }
        self.purge_dialog = Some(PurgeDialog::EmptyTrash {
            library_revision: snapshot.revision,
            note_count: note_ids.len(),
            attachment_count,
            attachment_bytes,
        });
        cx.notify();
    }

    fn confirm_purge(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.purge_dialog.take() else {
            return;
        };
        let action = match dialog {
            PurgeDialog::Note {
                note_id,
                note_revision,
                ..
            } => LibraryAction::DeleteNotePermanently {
                note_id,
                expected_revision: note_revision,
            },
            PurgeDialog::EmptyTrash {
                library_revision, ..
            } => LibraryAction::EmptyTrash {
                expected_library_revision: library_revision,
            },
        };
        self.send_action(action, cx);
        cx.notify();
    }

    fn cancel_purge(&mut self, cx: &mut Context<Self>) {
        self.purge_dialog = None;
        cx.notify();
    }

    fn begin_move_note(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            return;
        };
        self.move_dialog = Some(MoveDialog {
            note_id: note.id,
            note_revision: note.revision,
            current_folder: note.folder_id,
        });
        cx.notify();
    }

    fn move_note_to(&mut self, folder_id: Option<FolderId>, cx: &mut Context<Self>) {
        let Some(dialog) = self.move_dialog.take() else {
            return;
        };
        if dialog.current_folder == folder_id {
            cx.notify();
            return;
        }
        self.send_action(
            LibraryAction::MoveNote {
                note_id: dialog.note_id,
                expected_revision: dialog.note_revision,
                folder_id,
            },
            cx,
        );
        cx.notify();
    }

    fn cancel_move_note(&mut self, cx: &mut Context<Self>) {
        self.move_dialog = None;
        cx.notify();
    }

    fn choose_image_attachment(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message = Some("Wait for this note to finish saving before adding a photo".into());
            cx.notify();
            return;
        }
        if self.session.selected_note().is_none_or(|note| note.deleted) {
            return;
        }
        self.attachment_chooser_open = true;
        self.message = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_notes_image().await;
            let _ = this.update(cx, |this, cx| {
                this.attachment_chooser_open = false;
                match choice {
                    Ok(Some(path)) => this.queue_image_attachment(path, cx),
                    Ok(None) => cx.notify(),
                    Err(_) => {
                        this.message = Some("Notes could not open the Linux image chooser".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn queue_image_attachment(
        &mut self,
        selected_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() || self.latest_local_generation.is_some() {
            self.message = Some(
                "The note changed while the image chooser was open. Save it, then choose the image again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(note) = self.session.selected_note().filter(|note| !note.deleted) else {
            self.message = Some("The selected note is no longer available".into());
            cx.notify();
            return;
        };
        let note_id = note.id;
        let expected_revision = note.revision;
        let modified_unix_ms = now_unix_ms()
            .max(note.created_unix_ms)
            .max(note.modified_unix_ms);
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ActionRequest::new(
            request_id,
            LibraryAction::AttachImage {
                note_id,
                expected_revision,
                modified_unix_ms,
                selected_path,
            },
        ) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.attachment_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    fn choose_text_note_import(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        self.note_import_chooser_open = true;
        self.message = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_notes_text().await;
            let _ = this.update(cx, |this, cx| {
                this.note_import_chooser_open = false;
                match choice {
                    Ok(Some(path)) => this.queue_text_note_import(path, cx),
                    Ok(None) => cx.notify(),
                    Err(_) => {
                        this.message = Some("Notes could not open the Linux note importer".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn queue_text_note_import(
        &mut self,
        selected_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            self.message = Some(
                "The Notes library changed while the importer was open. Choose the file again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let folder_id = match self.session.folder_selection() {
            rmac_notes_runtime::FolderSelection::Folder(folder_id) => Some(folder_id),
            _ => None,
        };
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ActionRequest::new(
            request_id,
            LibraryAction::ImportTextNote {
                created_unix_ms: now_unix_ms(),
                folder_id,
                selected_path,
            },
        ) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Apply(request), cx) {
            self.note_import_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    fn accept_markdown_import(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.session.phase(), SessionPhase::Ready)
            || self.markdown_import_action_request_id.is_some()
        {
            return;
        }
        let Some(review_request_id) = self.note_import_request_id else {
            return;
        };
        let Some((reviewed_request_id, base_library_revision, _)) = self.markdown_import_review
        else {
            return;
        };
        if reviewed_request_id != review_request_id {
            return;
        }
        if self
            .session
            .snapshot()
            .is_none_or(|snapshot| snapshot.revision != base_library_revision)
        {
            self.message = Some(
                "The Notes library changed. Cancel this review and choose the Markdown file again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        if self.send(
            WorkerCommand::AcceptMarkdownImport {
                request_id,
                review_request_id,
            },
            cx,
        ) {
            self.markdown_import_action_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    fn discard_markdown_import_review(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.session.phase(), SessionPhase::Ready)
            || self.markdown_import_action_request_id.is_some()
        {
            return;
        }
        let Some(review_request_id) = self.note_import_request_id else {
            return;
        };
        if self
            .markdown_import_review
            .is_none_or(|(request_id, _, _)| request_id != review_request_id)
        {
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        if self.send(
            WorkerCommand::DiscardMarkdownImportReview {
                request_id,
                review_request_id,
            },
            cx,
        ) {
            self.markdown_import_action_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    fn choose_bundle_import(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message =
                Some("Wait for this note to finish saving before importing a bundle".into());
            cx.notify();
            return;
        }
        self.bundle_chooser_open = true;
        self.message = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let choice = rmac_portal::choose_notes_bundle().await;
            let _ = this.update(cx, |this, cx| {
                this.bundle_chooser_open = false;
                match choice {
                    Ok(Some(path)) => this.queue_bundle_import_review(path, cx),
                    Ok(None) => cx.notify(),
                    Err(_) => {
                        this.message =
                            Some("Notes could not open the Linux bundle importer".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn queue_bundle_import_review(
        &mut self,
        selected_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() || self.latest_local_generation.is_some() {
            self.message = Some(
                "The Notes library changed while the bundle chooser was open. Choose the bundle again."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match BundleImportReviewRequest::new(request_id, selected_path) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::ReviewBundleImport(request), cx) {
            self.bundle_review_request_id = Some(request_id);
            self.bundle_review = None;
            self.message = None;
            cx.notify();
        }
    }

    fn accept_bundle_import(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.session.phase(), SessionPhase::Ready)
            || self.bundle_action_request_id.is_some()
            || self.bundle_import_completion.is_some()
        {
            return;
        }
        let Some(review_request_id) = self.bundle_review_request_id else {
            return;
        };
        let Some((reviewed_request_id, review)) = self.bundle_review else {
            return;
        };
        if reviewed_request_id != review_request_id {
            return;
        }
        if self
            .session
            .snapshot()
            .is_none_or(|snapshot| snapshot.revision != review.base_library_revision)
        {
            self.message = Some(
                "The Notes library changed. Cancel this review and choose the bundle again.".into(),
            );
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match BundleImportAcceptRequest::new(
            request_id,
            review_request_id,
            BundleCollisionPolicy::KeepBoth,
        ) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::AcceptBundleImport(request), cx) {
            self.bundle_action_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    fn discard_bundle_import_review(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.session.phase(), SessionPhase::Ready)
            || self.bundle_action_request_id.is_some()
        {
            return;
        }
        let Some(review_request_id) = self.bundle_review_request_id else {
            return;
        };
        if self
            .bundle_review
            .is_none_or(|(request_id, _)| request_id != review_request_id)
        {
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        if self.send(
            WorkerCommand::DiscardBundleImportReview {
                request_id,
                review_request_id,
            },
            cx,
        ) {
            self.bundle_action_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    fn dismiss_bundle_import_completion(&mut self, cx: &mut Context<Self>) {
        self.bundle_import_completion = None;
        cx.notify();
    }

    fn begin_export(&mut self, cx: &mut Context<Self>) {
        if !self.is_interactive_ready() {
            return;
        }
        if self.latest_local_generation.is_some() {
            self.message = Some("Wait for this note to finish saving before exporting".into());
            cx.notify();
            return;
        }
        let Some(snapshot) = self.session.snapshot() else {
            return;
        };
        let scope = if let Some(note) = self.session.selected_note() {
            ExportScope::Note {
                note_id: note.id,
                expected_note_revision: note.revision,
            }
        } else if let rmac_notes_runtime::FolderSelection::Folder(folder_id) =
            self.session.folder_selection()
        {
            let Some(folder) = snapshot
                .folders
                .iter()
                .find(|folder| folder.id == folder_id && !folder.deleted)
            else {
                return;
            };
            ExportScope::Folder {
                folder_id,
                expected_folder_revision: folder.revision,
            }
        } else {
            ExportScope::Library {
                expected_library_revision: snapshot.revision,
            }
        };
        self.set_export_scope(scope, cx);
    }

    fn review_selected_note_export(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.session.selected_note() else {
            return;
        };
        self.set_export_scope(
            ExportScope::Note {
                note_id: note.id,
                expected_note_revision: note.revision,
            },
            cx,
        );
    }

    fn review_current_folder_export(&mut self, cx: &mut Context<Self>) {
        let rmac_notes_runtime::FolderSelection::Folder(folder_id) =
            self.session.folder_selection()
        else {
            return;
        };
        let Some(folder) = self.session.snapshot().and_then(|snapshot| {
            snapshot
                .folders
                .iter()
                .find(|folder| folder.id == folder_id && !folder.deleted)
        }) else {
            return;
        };
        self.set_export_scope(
            ExportScope::Folder {
                folder_id,
                expected_folder_revision: folder.revision,
            },
            cx,
        );
    }

    fn review_library_export(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.session.snapshot() else {
            return;
        };
        self.set_export_scope(
            ExportScope::Library {
                expected_library_revision: snapshot.revision,
            },
            cx,
        );
    }

    fn set_export_scope(&mut self, scope: ExportScope, cx: &mut Context<Self>) {
        let review = self
            .session
            .snapshot()
            .ok_or_else(|| "The Notes library is unavailable".to_string())
            .and_then(|snapshot| {
                snapshot
                    .plan_export(scope)
                    .map(|plan| ExportReview {
                        library_revision: plan.library_revision,
                        scope,
                        note_count: plan.note_ids.len(),
                        attachment_count: plan.attachments.len(),
                        markdown_bytes: plan.markdown_bytes,
                        attachment_bytes: plan.attachment_bytes,
                    })
                    .map_err(|error| error.to_string())
            });
        match review {
            Ok(review) => {
                self.export_dialog = Some(ExportDialog::Review(review));
                self.message = None;
            }
            Err(error) => {
                self.export_dialog = None;
                self.message = Some(error.into());
            }
        }
        cx.notify();
    }

    fn choose_export_destination(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        let Some(ExportDialog::Review(review)) = self.export_dialog else {
            return;
        };
        if format == ExportFormat::Markdown
            && (!matches!(review.scope, ExportScope::Note { .. }) || review.attachment_count != 0)
        {
            self.message =
                Some("Markdown export is available only for one note without attachments".into());
            cx.notify();
            return;
        }
        let suggested_name = self.export_suggested_name(review.scope, format);
        let portal_format = match format {
            ExportFormat::Markdown => rmac_portal::NotesExportFormat::Markdown,
            ExportFormat::RmacBundle => rmac_portal::NotesExportFormat::Bundle,
        };
        self.export_dialog = None;
        self.export_chooser_open = true;
        self.message = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let destination =
                rmac_portal::choose_notes_export_destination(portal_format, &suggested_name).await;
            let _ = this.update(cx, |this, cx| {
                this.export_chooser_open = false;
                match destination {
                    Ok(Some(path)) => this.queue_export(review.scope, format, path, cx),
                    Ok(None) => cx.notify(),
                    Err(_) => {
                        this.message = Some(
                            "Notes could not open the Linux export destination chooser".into(),
                        );
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn queue_export(
        &mut self,
        scope: ExportScope,
        format: ExportFormat,
        selected_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            self.message = Some("The Notes library changed. Review the export again.".into());
            cx.notify();
            return;
        }
        if let Err(error) = self
            .session
            .snapshot()
            .ok_or_else(|| "The Notes library is unavailable".to_string())
            .and_then(|snapshot| {
                snapshot
                    .plan_export(scope)
                    .map_err(|error| error.to_string())
            })
        {
            self.message = Some(error.into());
            cx.notify();
            return;
        }
        let Some(request_id) = self.take_request_id() else {
            return;
        };
        let request = match ExportRequest::new(request_id, scope, format, selected_path) {
            Ok(request) => request,
            Err(error) => {
                self.message = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        if self.send(WorkerCommand::Export(request), cx) {
            self.export_request_id = Some(request_id);
            self.message = None;
            cx.notify();
        }
    }

    fn export_suggested_name(&self, scope: ExportScope, format: ExportFormat) -> String {
        let label = match scope {
            ExportScope::Note { note_id, .. } => self
                .session
                .snapshot()
                .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
                .map_or_else(
                    || "Note".to_string(),
                    |note| display_title(&note.title).to_string(),
                ),
            ExportScope::Folder { folder_id, .. } => self
                .session
                .snapshot()
                .and_then(|snapshot| {
                    snapshot
                        .folders
                        .iter()
                        .find(|folder| folder.id == folder_id)
                })
                .map_or_else(|| "Notes Folder".to_string(), |folder| folder.name.clone()),
            ExportScope::Library { .. } => "All Notes".to_string(),
        };
        let extension = match format {
            ExportFormat::Markdown => "md",
            ExportFormat::RmacBundle => "rmacnotes",
        };
        format!("{}.{}", safe_export_stem(&label), extension)
    }

    fn dismiss_export_dialog(&mut self, cx: &mut Context<Self>) {
        self.export_dialog = None;
        cx.notify();
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
        let dismissed_folder_dialog = self.folder_dialog.take().is_some();
        let dismissed_purge_dialog = self.purge_dialog.take().is_some();
        let dismissed_move_dialog = self.move_dialog.take().is_some();
        let dismissed_attachment_dialog = self.attachment_dialog.take().is_some();
        let dismissed_export_dialog = self.export_dialog.take().is_some();
        let dismissed_bundle_completion = self.bundle_import_completion.take().is_some();
        if dismissed_folder_dialog
            || dismissed_purge_dialog
            || dismissed_move_dialog
            || dismissed_attachment_dialog
            || dismissed_export_dialog
            || dismissed_bundle_completion
        {
            cx.notify();
            return;
        }
        if self.attachment_chooser_open {
            self.message = Some("Finish or cancel the image chooser before closing Notes".into());
            cx.notify();
            return;
        }
        if self.note_import_chooser_open {
            self.message = Some("Finish or cancel the note importer before closing Notes".into());
            cx.notify();
            return;
        }
        if self.markdown_import_action_request_id.is_some() {
            self.message = Some("Wait for the current Markdown import action to finish".into());
            cx.notify();
            return;
        }
        if self.note_import_request_id.is_some() {
            self.message = Some(if self.markdown_import_review.is_some() {
                "Import or cancel the reviewed Markdown file before closing Notes.".into()
            } else {
                "Wait for the selected note file to finish its private review.".into()
            });
            cx.notify();
            return;
        }
        if self.export_chooser_open {
            self.message = Some("Finish or cancel the export chooser before closing Notes".into());
            cx.notify();
            return;
        }
        if self.export_request_id.is_some() {
            self.message = Some("Wait for the Notes export to finish".into());
            cx.notify();
            return;
        }
        if self.bundle_chooser_open {
            self.message = Some("Finish or cancel the bundle chooser before closing Notes".into());
            cx.notify();
            return;
        }
        if self.bundle_action_request_id.is_some() {
            self.message = Some("Wait for the current bundle operation to finish".into());
            cx.notify();
            return;
        }
        if self.bundle_review_request_id.is_some() {
            self.message = Some(if self.bundle_review.is_some() {
                "Import or cancel the reviewed bundle before closing Notes so its private review can be released."
                    .into()
            } else {
                "Wait for the selected Notes bundle to finish its private review before closing."
                    .into()
            });
            cx.notify();
            return;
        }
        if self.attachment_action_pending() {
            self.message = Some("Wait for the current attachment operation to finish".into());
            cx.notify();
            return;
        }
        if matches!(self.session.phase(), SessionPhase::Pending { .. }) {
            self.message = Some("Retry or discard the pending change before closing Notes".into());
            cx.notify();
            return;
        }
        if !self.request_preview_shutdown(cx) {
            return;
        }
        if !self.request_search_shutdown(cx) {
            return;
        }
        let shutdown = self
            .worker
            .as_ref()
            .ok_or(WorkerSendError::Closed)
            .and_then(|worker| worker.try_send(WorkerCommand::Shutdown));
        if let Err(error) = shutdown {
            self.message =
                Some(format!("Notes could not safely close yet: {error}. Try again.").into());
            cx.notify();
            return;
        }
        self.closing = true;
        window.remove_window();
    }

    fn request_preview_shutdown(&mut self, cx: &mut Context<Self>) -> bool {
        if self.preview_shutdown_requested {
            return true;
        }
        self.preview.clear();
        self.preview_image = None;
        let result = self
            .preview_worker
            .as_ref()
            .ok_or(PreviewWorkerSendError::Closed)
            .and_then(NotesPreviewWorkerClient::try_shutdown);
        match result {
            Ok(()) | Err(PreviewWorkerSendError::Closed) => {
                self.preview_shutdown_requested = true;
                true
            }
            Err(error) => {
                self.message = Some(
                    format!("Notes image preview is still finishing: {error}. Try again.").into(),
                );
                cx.notify();
                false
            }
        }
    }

    fn request_search_shutdown(&mut self, cx: &mut Context<Self>) -> bool {
        if self.search_shutdown_requested {
            return true;
        }
        self.search.cancel();
        let result = self
            .search_worker
            .as_ref()
            .ok_or(SearchWorkerSendError::Closed)
            .and_then(NotesSearchWorkerClient::try_shutdown);
        match result {
            Ok(()) | Err(SearchWorkerSendError::Closed) => {
                self.search_shutdown_requested = true;
                true
            }
            Err(error) => {
                self.message =
                    Some(format!("Notes search is still finishing: {error}. Try again.").into());
                cx.notify();
                false
            }
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.is_interactive_ready();
        let selected = self.session.selected_note();
        let deleted = selected.is_some_and(|note| note.deleted);
        let pinned = selected.is_some_and(|note| note.pinned);
        let note_save_pending = self.latest_local_generation.is_some();
        let attachment_busy = self.attachment_chooser_open || self.attachment_action_pending();
        let note_import_busy = self.note_import_chooser_open
            || self.note_import_request_id.is_some()
            || self.markdown_import_action_request_id.is_some();
        let export_busy = self.export_chooser_open || self.export_request_id.is_some();
        let bundle_import_busy = self.bundle_chooser_open
            || self.bundle_review_request_id.is_some()
            || self.bundle_action_request_id.is_some();
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
                        Button::new("import-note", "")
                            .icon(IconName::File)
                            .ghost()
                            .with_size(Size::Medium)
                            .busy(note_import_busy)
                            .disabled(!ready)
                            .tooltip(if note_import_busy {
                                "Importing Note…"
                            } else {
                                "Import Note…"
                            })
                            .on_click(
                                cx.listener(|this, _, _, cx| this.choose_text_note_import(cx)),
                            ),
                    )
                    .child(
                        Button::new("import-bundle", "")
                            .icon(IconName::FolderOpen)
                            .ghost()
                            .with_size(Size::Medium)
                            .busy(bundle_import_busy)
                            .disabled(!ready || note_save_pending)
                            .tooltip(if bundle_import_busy {
                                "Importing Notes Bundle…"
                            } else if note_save_pending {
                                "Saving Note…"
                            } else {
                                "Import Notes Bundle…"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.choose_bundle_import(cx))),
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
                        div()
                            .w(px(220.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .rounded(px(7.0))
                            .bg(mac::control_fill())
                            .child(
                                Icon::new(IconName::Search)
                                    .with_size(Size::XSmall)
                                    .text_color(mac::text_tertiary()),
                            )
                            .child(
                                TextField::new(&self.search_query)
                                    .appearance(false)
                                    .cleanable(true)
                                    .small()
                                    .disabled(self.session.snapshot().is_none()),
                            ),
                    )
                    .child(
                        Button::new("export-notes", "")
                            .icon(IconName::ExternalLink)
                            .ghost()
                            .with_size(Size::Medium)
                            .busy(export_busy)
                            .disabled(
                                !ready || self.session.snapshot().is_none() || note_save_pending,
                            )
                            .tooltip(if export_busy {
                                "Exporting Notes…"
                            } else if note_save_pending {
                                "Saving Note…"
                            } else {
                                "Export…"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.begin_export(cx))),
                    )
                    .child(
                        Button::new("add-image", "")
                            .icon(IconName::GalleryVerticalEnd)
                            .ghost()
                            .with_size(Size::Medium)
                            .busy(attachment_busy)
                            .disabled(!ready || deleted || selected.is_none() || note_save_pending)
                            .tooltip(if attachment_busy {
                                "Updating Attachments…"
                            } else if note_save_pending {
                                "Saving Note…"
                            } else {
                                "Add Photo…"
                            })
                            .on_click(
                                cx.listener(|this, _, _, cx| this.choose_image_attachment(cx)),
                            ),
                    )
                    .child(
                        Button::new("move-note", "")
                            .icon(IconName::Folder)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(!ready || deleted || selected.is_none())
                            .tooltip("Move Note…")
                            .on_click(cx.listener(|this, _, _, cx| this.begin_move_note(cx))),
                    )
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
                    )
                    .when(deleted, |element| {
                        element.child(
                            Button::new("delete-permanently", "")
                                .icon(IconName::Delete)
                                .destructive()
                                .with_size(Size::Medium)
                                .disabled(!ready)
                                .tooltip("Delete Note Permanently…")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.begin_permanent_note_delete(cx)
                                })),
                        )
                    }),
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
        let has_selected_folder = matches!(current, FolderSelection::Folder(_));
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
                        div()
                            .flex()
                            .items_center()
                            .gap_0p5()
                            .child(
                                Button::new("folder-actions", "")
                                    .icon(IconName::Ellipsis)
                                    .ghost()
                                    .with_size(Size::XSmall)
                                    .disabled(!self.is_interactive_ready() || !has_selected_folder)
                                    .tooltip("Folder Actions")
                                    .dropdown_menu(|menu, _, _| {
                                        menu.menu("Rename Folder…", Box::new(RenameSelectedFolder))
                                            .menu("Delete Folder…", Box::new(DeleteSelectedFolder))
                                    }),
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
        let search_active = !self.search_query.read(cx).value().trim().is_empty();
        let in_trash =
            self.session.folder_selection() == rmac_notes_runtime::FolderSelection::Trash;
        let selected = if search_active {
            self.search.selected()
        } else {
            self.session.selected_note_id()
        };
        let notes = if search_active && self.search.state() == SearchState::Results {
            self.session.snapshot().map_or_else(Vec::new, |snapshot| {
                self.search
                    .hits()
                    .iter()
                    .filter_map(|hit| {
                        snapshot
                            .notes
                            .iter()
                            .find(|note| note.id == hit.note_id && !note.deleted)
                    })
                    .collect()
            })
        } else if search_active {
            Vec::new()
        } else {
            self.session.visible_notes()
        };
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if search_active {
                            this.select_search_result(note_id, window, cx)
                        } else {
                            this.select_note(note_id, window, cx)
                        }
                    }))
                    .into_any_element(),
            );
        }
        if items.is_empty() {
            let empty_message: SharedString = if search_active {
                match self.search.state() {
                    SearchState::Indexing => "Searching…".into(),
                    SearchState::NoMatches => "No matching notes".into(),
                    SearchState::Unavailable => self.search.failure().map_or_else(
                        || "Search is unavailable".into(),
                        |error| error.to_string().into(),
                    ),
                    SearchState::Empty | SearchState::Results => "Search is unavailable".into(),
                }
            } else if self.session.folder_selection() == rmac_notes_runtime::FolderSelection::Trash
            {
                "Recently Deleted is empty".into()
            } else {
                "No notes in this folder".into()
            };
            items.push(
                div()
                    .px_4()
                    .py_6()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(mac::text_tertiary())
                    .child(empty_message)
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
                    .justify_between()
                    .px_4()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_secondary())
                            .child(format!(
                                "{note_count} {}",
                                if search_active {
                                    if note_count == 1 {
                                        "Result"
                                    } else {
                                        "Results"
                                    }
                                } else if note_count == 1 {
                                    "Note"
                                } else {
                                    "Notes"
                                }
                            )),
                    )
                    .when(!search_active && in_trash && note_count != 0, |element| {
                        element.child(
                            Button::new("empty-trash", "Empty")
                                .destructive()
                                .xsmall()
                                .disabled(!self.is_interactive_ready())
                                .tooltip("Empty Recently Deleted…")
                                .on_click(cx.listener(|this, _, _, cx| this.begin_empty_trash(cx))),
                        )
                    }),
            )
            .child(
                div()
                    .id("notes-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .py_1()
                    .children(items),
            )
            .when(
                search_active && self.search.results_truncated(),
                |element| {
                    element.child(
                        div()
                            .px_3()
                            .py_1()
                            .text_size(rmac_ui::text_px(10.0))
                            .text_color(mac::text_tertiary())
                            .child("Showing the first 500 results"),
                    )
                },
            )
    }

    fn render_attachments(&self, note: &NoteRecord, cx: &mut Context<Self>) -> Option<AnyElement> {
        let snapshot = self.session.snapshot()?;
        let attachments = note
            .attachments
            .iter()
            .filter_map(|attachment_id| {
                snapshot
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == *attachment_id && !attachment.deleted)
            })
            .collect::<Vec<_>>();
        if attachments.is_empty() {
            return None;
        }
        let selected = self.selected_attachment;
        let can_remove = !note.deleted
            && selected.is_some_and(|attachment_id| {
                attachments
                    .iter()
                    .any(|attachment| attachment.id == attachment_id)
            });
        let preview = match self.preview.state() {
            PreviewState::Loading { attachment_id, .. } if selected == Some(*attachment_id) => {
                centered_attachment_state("Loading preview…", None, cx)
            }
            PreviewState::Ready { image, .. }
                if selected == Some(image.attachment_id()) && self.preview_image.is_some() =>
            {
                div()
                    .size_full()
                    .rounded(px(8.0))
                    .overflow_hidden()
                    .bg(mac::control_fill())
                    .child(
                        img(self
                            .preview_image
                            .as_ref()
                            .expect("preview image checked")
                            .clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                    )
                    .into_any_element()
            }
            PreviewState::Ready { image, .. } if selected == Some(image.attachment_id()) => {
                centered_attachment_state("Preview unavailable", Some("Try Again"), cx)
            }
            PreviewState::Unavailable { attachment_id, .. } if selected == Some(*attachment_id) => {
                centered_attachment_state("Preview unavailable", Some("Try Again"), cx)
            }
            _ if self.preview_worker.is_none() => {
                centered_attachment_state("Preview unavailable", None, cx)
            }
            _ => centered_attachment_state("Loading preview…", None, cx),
        };
        let rows = attachments.into_iter().map(|attachment| {
            let attachment_id = attachment.id;
            div()
                .v_flex()
                .gap_0p5()
                .child(
                    Button::new(
                        ("attachment-preview", attachment_id.get()),
                        attachment.display_name.clone(),
                    )
                    .selected(selected == Some(attachment_id))
                    .xsmall()
                    .w_full()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_attachment_preview(attachment_id, cx)
                    })),
                )
                .child(
                    div()
                        .px_2()
                        .text_size(rmac_ui::text_px(10.0))
                        .text_color(mac::text_tertiary())
                        .child(format_storage_bytes(attachment.byte_len)),
                )
        });
        Some(
            div()
                .mx(px(44.0))
                .mb_2()
                .h(px(174.0))
                .flex_none()
                .flex()
                .gap_3()
                .p_2()
                .rounded(px(10.0))
                .border_1()
                .border_color(mac::separator())
                .bg(mac::window())
                .child(div().w(px(260.0)).h_full().child(preview))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .id("attachment-list")
                                .flex_1()
                                .min_h(px(0.0))
                                .overflow_y_scroll()
                                .v_flex()
                                .gap_1()
                                .children(rows),
                        )
                        .when(can_remove, |element| {
                            element.child(
                                Button::new("remove-attachment", "Remove Photo…")
                                    .destructive()
                                    .xsmall()
                                    .disabled(
                                        !self.is_interactive_ready()
                                            || self.latest_local_generation.is_some(),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.begin_attachment_removal(cx)
                                    })),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(note) = self.session.selected_note() else {
            return centered_state("No Note Selected", "Choose a note or create a new one.");
        };
        let editable = self.is_interactive_ready() && !note.deleted;
        let words = self.body.read(cx).value().split_whitespace().count();
        let characters = self.body.read(cx).value().chars().count();
        let attachments = self.render_attachments(note, cx);
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
            .child(
                div()
                    .mx(px(44.0))
                    .mt_1()
                    .mb_2()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .rounded(px(7.0))
                    .bg(mac::control_fill())
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::notes_accent())
                    .child("#")
                    .child(
                        TextField::new(&self.tags)
                            .appearance(false)
                            .cleanable(true)
                            .small()
                            .disabled(!editable),
                    ),
            )
            .when_some(attachments, |element, attachments| {
                element.child(attachments)
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

    fn render_folder_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};

        let dialog = self.folder_dialog?;
        let folder_id = match dialog {
            FolderDialog::Rename(folder_id) | FolderDialog::Delete(folder_id) => folder_id,
        };
        let folder = self
            .session
            .folders()
            .into_iter()
            .find(|folder| folder.id == folder_id)?;
        match dialog {
            FolderDialog::Rename(_) => {
                let card = div()
                    .w(px(360.0))
                    .p(px(20.0))
                    .v_flex()
                    .gap_3()
                    .rounded(px(12.0))
                    .bg(mac::window())
                    .border_1()
                    .border_color(mac::separator())
                    .shadow_xl()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(15.0))
                            .font_weight(mac::BOLD)
                            .child("Rename Folder"),
                    )
                    .child(TextField::new(&self.folder_name_input).cleanable(true))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                rmac_ui::dialog_button("cancel-folder-rename", "Cancel", Normal)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.cancel_folder_dialog(cx)),
                                    ),
                            )
                            .child(
                                rmac_ui::dialog_button("commit-folder-rename", "Rename", Primary)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.commit_folder_rename(cx)),
                                    ),
                            ),
                    );
                Some(rmac_ui::dialog("rename-folder-dialog", card).into_any_element())
            }
            FolderDialog::Delete(_) => {
                let count = self.session.folder_count(folder_id);
                let message = format!(
                    "Delete “{}”? {} {} will move to All Notes. The notes and their attachments will not be deleted.",
                    folder.name,
                    count,
                    if count == 1 { "note" } else { "notes" }
                );
                Some(
                    rmac_ui::alert(
                        "Delete this folder?",
                        message,
                        vec![
                            rmac_ui::dialog_button("cancel-folder-delete", "Cancel", Normal)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_folder_dialog(cx)),
                                )
                                .into_any_element(),
                            rmac_ui::dialog_button(
                                "confirm-folder-delete",
                                "Delete Folder",
                                Destructive,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_folder_delete(cx)))
                            .into_any_element(),
                        ],
                    )
                    .into_any_element(),
                )
            }
        }
    }

    fn render_purge_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let dialog = self.purge_dialog?;
        let (title, message, confirm_label) = match dialog {
            PurgeDialog::Note {
                note_id,
                attachment_count,
                attachment_bytes,
                ..
            } => {
                let note_title = self
                    .session
                    .snapshot()
                    .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
                    .map_or_else(|| "this note".to_string(), |note| display_title(&note.title).to_string());
                (
                    "Delete this note permanently?",
                    format!(
                        "“{note_title}” and {attachment_count} {} ({}) will be permanently deleted. This cannot be undone. If attachment cleanup needs attention, Notes will pause further edits.",
                        if attachment_count == 1 { "attachment" } else { "attachments" },
                        format_storage_bytes(attachment_bytes)
                    ),
                    "Delete Note",
                )
            }
            PurgeDialog::EmptyTrash {
                note_count,
                attachment_count,
                attachment_bytes,
                ..
            } => (
                "Permanently delete all notes?",
                format!(
                    "{note_count} {} and {attachment_count} {} ({}) will be permanently deleted. This cannot be undone. If attachment cleanup needs attention, Notes will pause further edits.",
                    if note_count == 1 { "note" } else { "notes" },
                    if attachment_count == 1 { "attachment" } else { "attachments" },
                    format_storage_bytes(attachment_bytes)
                ),
                "Empty Trash",
            ),
        };
        Some(
            rmac_ui::alert(
                title,
                message,
                vec![
                    rmac_ui::dialog_button("cancel-purge", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_purge(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("confirm-purge", confirm_label, Destructive)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_purge(cx)))
                        .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }

    fn render_attachment_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let dialog = self.attachment_dialog?;
        let (attachment_id, expected_revision, byte_len) = match dialog {
            AttachmentDialog::Remove {
                attachment_id,
                attachment_revision,
                byte_len,
                ..
            }
            | AttachmentDialog::CollectOrphan {
                attachment_id,
                attachment_revision,
                byte_len,
            } => (attachment_id, attachment_revision, byte_len),
        };
        let name = self
            .session
            .snapshot()
            .and_then(|snapshot| {
                snapshot.attachments.iter().find(|attachment| {
                    attachment.id == attachment_id && attachment.revision == expected_revision
                })
            })
            .map_or_else(
                || "this photo".to_string(),
                |attachment| attachment.display_name.clone(),
            );
        let (title, message, confirm_label) = match dialog {
            AttachmentDialog::Remove { .. } => (
                "Remove this photo?",
                format!(
                    "Remove “{name}” ({}) from this note? Notes will first save the note without the reference, then delete its managed local copy in a separate verified cleanup. The original imported file is not changed.",
                    format_storage_bytes(byte_len)
                ),
                "Remove Photo",
            ),
            AttachmentDialog::CollectOrphan { .. } => (
                "Clean up this removed photo?",
                format!(
                    "“{name}” ({}) is no longer referenced by any note. Delete its managed local copy? The original imported file is not changed.",
                    format_storage_bytes(byte_len)
                ),
                "Delete Managed Copy",
            ),
        };
        Some(
            rmac_ui::alert(
                title,
                message,
                vec![
                    rmac_ui::dialog_button("cancel-attachment-action", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_attachment_dialog(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("confirm-attachment-action", confirm_label, Destructive)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_attachment_dialog(cx)))
                        .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }

    fn render_export_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Normal, Primary};

        match self.export_dialog? {
            ExportDialog::Complete(outcome) => {
                let format = match outcome.format {
                    ExportFormat::Markdown => "Markdown file",
                    ExportFormat::RmacBundle => "rmac Notes bundle",
                };
                Some(
                    rmac_ui::alert(
                        "Export complete",
                        format!(
                            "Notes verified the final {format}: {} {}, {} {}, {} total.",
                            outcome.note_count,
                            if outcome.note_count == 1 {
                                "note"
                            } else {
                                "notes"
                            },
                            outcome.attachment_count,
                            if outcome.attachment_count == 1 {
                                "attachment"
                            } else {
                                "attachments"
                            },
                            format_storage_bytes(outcome.output.byte_len)
                        ),
                        vec![rmac_ui::dialog_button("dismiss-export", "Done", Primary)
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_export_dialog(cx)))
                            .into_any_element()],
                    )
                    .into_any_element(),
                )
            }
            ExportDialog::Review(review) => {
                let snapshot = self.session.snapshot()?;
                let note_scope = self.session.selected_note().map(|note| ExportScope::Note {
                    note_id: note.id,
                    expected_note_revision: note.revision,
                });
                let folder_scope = match self.session.folder_selection() {
                    rmac_notes_runtime::FolderSelection::Folder(folder_id) => snapshot
                        .folders
                        .iter()
                        .find(|folder| folder.id == folder_id && !folder.deleted)
                        .map(|folder| ExportScope::Folder {
                            folder_id,
                            expected_folder_revision: folder.revision,
                        }),
                    _ => None,
                };
                let library_scope = ExportScope::Library {
                    expected_library_revision: snapshot.revision,
                };
                let can_markdown = matches!(review.scope, ExportScope::Note { .. })
                    && review.attachment_count == 0;
                let scope_detail = match review.scope {
                    ExportScope::Note { .. } => "The selected note is bound to its exact revision.",
                    ExportScope::Folder { .. } => {
                        "Only live notes in the current folder are included."
                    }
                    ExportScope::Library { .. } => {
                        "The complete library includes live and Recently Deleted notes."
                    }
                };
                let reviewed_bytes = review
                    .markdown_bytes
                    .saturating_add(review.attachment_bytes);
                let mut scope_buttons = Vec::<AnyElement>::new();
                if let Some(scope) = note_scope {
                    scope_buttons.push(
                        Button::new("export-this-note", "This Note")
                            .selected(review.scope == scope)
                            .w_full()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.review_selected_note_export(cx)),
                            )
                            .into_any_element(),
                    );
                }
                if let Some(scope) = folder_scope {
                    scope_buttons.push(
                        Button::new("export-current-folder", "Current Folder")
                            .selected(review.scope == scope)
                            .w_full()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.review_current_folder_export(cx)),
                            )
                            .into_any_element(),
                    );
                }
                scope_buttons.push(
                    Button::new("export-library", "Entire Library")
                        .selected(review.scope == library_scope)
                        .w_full()
                        .on_click(cx.listener(|this, _, _, cx| this.review_library_export(cx)))
                        .into_any_element(),
                );
                let card = div()
                    .w(px(440.0))
                    .p(px(20.0))
                    .v_flex()
                    .gap_3()
                    .rounded(px(12.0))
                    .bg(mac::window())
                    .border_1()
                    .border_color(mac::separator())
                    .shadow_xl()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(mac::BOLD)
                            .child("Export Notes"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child(scope_detail),
                    )
                    .child(div().v_flex().gap_1().children(scope_buttons))
                    .child(
                        div()
                            .p_3()
                            .rounded(px(8.0))
                            .bg(mac::control_fill())
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child(format!(
                                "{} {}, {} {}, {} of note and attachment content",
                                review.note_count,
                                if review.note_count == 1 { "note" } else { "notes" },
                                review.attachment_count,
                                if review.attachment_count == 1 {
                                    "attachment"
                                } else {
                                    "attachments"
                                },
                                format_storage_bytes(reviewed_bytes)
                            )),
                    )
                    .when(!can_markdown, |element| {
                        element.child(
                            div()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text_tertiary())
                                .child(
                                    "Use an rmac Notes bundle for folders, the library, or notes with attachments.",
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                rmac_ui::dialog_button("cancel-export", "Cancel", Normal)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.dismiss_export_dialog(cx)
                                    })),
                            )
                            .when(can_markdown, |element| {
                                element.child(
                                    rmac_ui::dialog_button(
                                        "export-markdown",
                                        "Export Markdown…",
                                        Normal,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choose_export_destination(ExportFormat::Markdown, cx)
                                    })),
                                )
                            })
                            .child(
                                rmac_ui::dialog_button(
                                    "export-bundle",
                                    "Export Bundle…",
                                    Primary,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.choose_export_destination(ExportFormat::RmacBundle, cx)
                                })),
                            ),
                    );
                Some(rmac_ui::dialog("export-dialog", card).into_any_element())
            }
        }
    }

    fn render_markdown_import_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Normal, Primary};

        let review_request_id = self.note_import_request_id?;
        if self.markdown_import_action_request_id.is_some() {
            if matches!(self.session.phase(), SessionPhase::Pending { .. }) {
                return None;
            }
            let card = div()
                .w(px(420.0))
                .p(px(20.0))
                .v_flex()
                .gap_3()
                .rounded(px(12.0))
                .bg(mac::window())
                .border_1()
                .border_color(mac::separator())
                .shadow_xl()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(17.0))
                        .font_weight(mac::BOLD)
                        .child("Importing Markdown note…"),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(
                            "Notes is committing the exact reviewed text candidate and verifying durable readback.",
                        ),
                );
            return Some(rmac_ui::dialog("markdown-import-progress", card).into_any_element());
        }

        let (reviewed_request_id, base_library_revision, review) = self.markdown_import_review?;
        if reviewed_request_id != review_request_id {
            return None;
        }
        let attention = if review.attention_count() == 0 {
            "No linked images, raw HTML, tables, tasks, footnotes, or frontmatter were recognized."
                .to_string()
        } else {
            format!(
                "Review found {} linked {}, {} raw HTML {}, {} {}, {} task-list {}, {} footnote {}, and {} frontmatter {}.",
                review.image_count,
                if review.image_count == 1 { "image" } else { "images" },
                review.raw_html_count,
                if review.raw_html_count == 1 { "construct" } else { "constructs" },
                review.table_count,
                if review.table_count == 1 { "table" } else { "tables" },
                review.task_count,
                if review.task_count == 1 { "item" } else { "items" },
                review.footnote_count,
                if review.footnote_count == 1 { "construct" } else { "constructs" },
                review.frontmatter_count,
                if review.frontmatter_count == 1 { "block" } else { "blocks" },
            )
        };
        let card = div()
            .w(px(470.0))
            .p(px(20.0))
            .v_flex()
            .gap_3()
            .rounded(px(12.0))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(rmac_ui::text_px(17.0))
                    .font_weight(mac::BOLD)
                    .child("Import Markdown Note"),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "This review is bound to Notes library revision {base_library_revision}. The selected source decoded as {}.",
                        review.encoding.label()
                    )),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(mac::control_fill())
                    .v_flex()
                    .gap_1()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "{} source; {} decoded text; {} {}; {} {}",
                        format_storage_bytes(review.source_bytes),
                        format_storage_bytes(review.decoded_bytes),
                        review.heading_count,
                        if review.heading_count == 1 { "heading" } else { "headings" },
                        review.link_count,
                        if review.link_count == 1 { "link" } else { "links" },
                    ))
                    .child(attention),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(mac::control_fill())
                    .text_size(rmac_ui::text_px(12.0))
                    .child(
                        "Import preserves the decoded Markdown characters and line endings as editable note source. Notes does not download linked images, execute raw HTML, or turn frontmatter, tables, tasks, or footnotes into active data. The original file is unchanged.",
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button("cancel-markdown-import", "Cancel", Normal)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.discard_markdown_import_review(cx)
                            })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            ("accept-markdown-import", review_request_id),
                            "Import as Markdown Source",
                            Primary,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.accept_markdown_import(cx))),
                    ),
            );
        Some(rmac_ui::dialog("markdown-import-review", card).into_any_element())
    }

    fn render_bundle_import_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Normal, Primary};

        if let Some(completion) = self.bundle_import_completion {
            let title = if completion.maintenance_pending {
                "Import accepted"
            } else {
                "Import complete"
            };
            let maintenance = if completion.maintenance_pending {
                " The imported library is durable, but verified storage maintenance is still pending. Editing remains paused until recovery finishes."
            } else {
                " Notes verified the accepted library and its imported attachments."
            };
            return Some(
                rmac_ui::alert(
                    title,
                    format!(
                        "Imported {} {}, {} {}, and {} {} ({} of attachments).{maintenance}",
                        completion.folder_count,
                        if completion.folder_count == 1 {
                            "folder"
                        } else {
                            "folders"
                        },
                        completion.note_count,
                        if completion.note_count == 1 {
                            "note"
                        } else {
                            "notes"
                        },
                        completion.attachment_count,
                        if completion.attachment_count == 1 {
                            "attachment"
                        } else {
                            "attachments"
                        },
                        format_storage_bytes(completion.attachment_bytes),
                    ),
                    vec![
                        rmac_ui::dialog_button("dismiss-bundle-import", "Done", Primary)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dismiss_bundle_import_completion(cx)
                            }))
                            .into_any_element(),
                    ],
                )
                .into_any_element(),
            );
        }

        let review_request_id = self.bundle_review_request_id?;
        if self.bundle_action_request_id.is_some() {
            if matches!(self.session.phase(), SessionPhase::Pending { .. }) {
                return None;
            }
            let card = div()
                .w(px(420.0))
                .p(px(20.0))
                .v_flex()
                .gap_3()
                .rounded(px(12.0))
                .bg(mac::window())
                .border_1()
                .border_color(mac::separator())
                .shadow_xl()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(17.0))
                        .font_weight(mac::BOLD)
                        .child("Applying Notes bundle…"),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(
                            "Notes is verifying the reviewed source and committing attachments before publishing metadata.",
                        ),
                );
            return Some(rmac_ui::dialog("bundle-import-progress", card).into_any_element());
        }

        let Some((reviewed_request_id, review)) = self.bundle_review else {
            let card = div()
                .w(px(420.0))
                .p(px(20.0))
                .v_flex()
                .gap_3()
                .rounded(px(12.0))
                .bg(mac::window())
                .border_1()
                .border_color(mac::separator())
                .shadow_xl()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(17.0))
                        .font_weight(mac::BOLD)
                        .child("Reviewing Notes bundle…"),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(
                            "Checking the versioned manifest, note records, hashes, and bounded image payloads. No library changes have been made.",
                        ),
                );
            return Some(rmac_ui::dialog("bundle-review-progress", card).into_any_element());
        };
        if reviewed_request_id != review_request_id {
            return None;
        }
        let collision_detail = if review.identity_collisions == 0
            && review.folder_name_collisions == 0
        {
            "No stable-identity or live folder-name collisions were found.".to_string()
        } else {
            format!(
                "{} stable-identity collision{} will be remapped, and {} live folder-name collision{} will receive a deterministic imported suffix.",
                review.identity_collisions,
                if review.identity_collisions == 1 { "" } else { "s" },
                review.folder_name_collisions,
                if review.folder_name_collisions == 1 { "" } else { "s" },
            )
        };
        let card = div()
            .w(px(460.0))
            .p(px(20.0))
            .v_flex()
            .gap_3()
            .rounded(px(12.0))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(rmac_ui::text_px(17.0))
                    .font_weight(mac::BOLD)
                    .child("Import Notes Bundle"),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "This review is bound to library revision {} and bundle library revision {}.",
                        review.base_library_revision, review.source_library_revision
                    )),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(mac::control_fill())
                    .v_flex()
                    .gap_1()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "{} {}, {} {}, and {} {}",
                        review.folder_count,
                        if review.folder_count == 1 { "folder" } else { "folders" },
                        review.note_count,
                        if review.note_count == 1 { "note" } else { "notes" },
                        review.attachment_count,
                        if review.attachment_count == 1 {
                            "attachment"
                        } else {
                            "attachments"
                        },
                    ))
                    .child(format!(
                        "{} bundle source; {} of attachment payloads",
                        format_storage_bytes(review.source_bytes),
                        format_storage_bytes(review.attachment_bytes),
                    )),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(collision_detail),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(mac::control_fill())
                    .text_size(rmac_ui::text_px(12.0))
                    .child(
                        "Keep Both never overwrites an existing note, folder, attachment, or purged identity. Imported records are safely renamed or remapped when needed.",
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button("cancel-bundle-import", "Cancel", Normal)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.discard_bundle_import_review(cx)
                            })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            ("accept-bundle-import", review_request_id),
                            "Import and Keep Both",
                            Primary,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.accept_bundle_import(cx))),
                    ),
            );
        Some(rmac_ui::dialog("bundle-import-review", card).into_any_element())
    }

    fn render_move_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::Normal;

        let dialog = self.move_dialog?;
        let note_title = self
            .session
            .snapshot()
            .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == dialog.note_id))
            .map_or_else(|| "Note".into(), |note| display_title(&note.title));
        let mut rows = vec![Button::new("move-to-all", "All Notes")
            .selected(dialog.current_folder.is_none())
            .w_full()
            .on_click(cx.listener(|this, _, _, cx| this.move_note_to(None, cx)))
            .into_any_element()];
        rows.extend(self.session.folders().into_iter().map(|folder| {
            let folder_id = folder.id;
            Button::new(("move-to-folder", folder_id.get()), folder.name.clone())
                .selected(dialog.current_folder == Some(folder_id))
                .w_full()
                .on_click(cx.listener(move |this, _, _, cx| this.move_note_to(Some(folder_id), cx)))
                .into_any_element()
        }));
        let card = div()
            .w(px(380.0))
            .max_h(px(480.0))
            .p(px(20.0))
            .v_flex()
            .gap_3()
            .rounded(px(12.0))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(mac::BOLD)
                    .child("Move Note"),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(mac::text_secondary())
                    .truncate()
                    .child(note_title),
            )
            .child(
                div()
                    .id("move-note-destinations")
                    .max_h(px(330.0))
                    .overflow_y_scroll()
                    .v_flex()
                    .gap_1()
                    .children(rows),
            )
            .child(
                div().flex().justify_end().child(
                    rmac_ui::dialog_button("cancel-move-note", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_move_note(cx))),
                ),
            );
        Some(rmac_ui::dialog("move-note-dialog", card).into_any_element())
    }

    fn render_status_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let orphan_waiting =
            !self.attachment_action_pending() && self.first_orphaned_attachment().is_some();
        let (message, actions) = match self.session.phase() {
            SessionPhase::Pending { reason, .. } => (
                pending_message(*reason),
                Some(StatusActions::Pending),
            ),
            SessionPhase::Maintenance { .. } => (
                "Notes recovered the library but maintenance still needs attention. Editing is paused."
                    .to_string(),
                None,
            ),
            _ => match &self.message {
                Some(message) => (
                    message.to_string(),
                    orphan_waiting.then_some(StatusActions::OrphanCleanup),
                ),
                None if orphan_waiting => (
                    "A removed photo is still stored until its managed copy is cleaned up."
                        .to_string(),
                    Some(StatusActions::OrphanCleanup),
                ),
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
        match actions {
            Some(StatusActions::Pending) => {
                banner = banner
                    .child(
                        Button::new("retry-pending", "Retry")
                            .xsmall()
                            .on_click(cx.listener(|this, _, _, cx| this.retry_pending(cx))),
                    )
                    .child(
                        Button::new("discard-pending", "Discard")
                            .xsmall()
                            .on_click(cx.listener(|this, _, _, cx| this.discard_pending(cx))),
                    );
            }
            Some(StatusActions::OrphanCleanup) => {
                banner = banner.child(
                    Button::new("review-orphan-cleanup", "Clean Up…")
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.begin_orphan_cleanup(cx))),
                );
            }
            None => {}
        }
        Some(banner.into_any_element())
    }
}

impl Drop for NotesView {
    fn drop(&mut self) {
        if !self.closing {
            self.preview.clear();
            if let Some(preview_worker) = &self.preview_worker {
                if matches!(
                    preview_worker.try_shutdown(),
                    Err(PreviewWorkerSendError::Full)
                ) {
                    let preview_worker = preview_worker.clone();
                    let _ = thread::Builder::new()
                        .name("rmac-notes-preview-close".into())
                        .spawn(move || {
                            let _ = preview_worker.shutdown_blocking();
                        });
                }
            }
            self.search.cancel();
            if let Some(search_worker) = &self.search_worker {
                if matches!(
                    search_worker.try_shutdown(),
                    Err(SearchWorkerSendError::Full)
                ) {
                    let search_worker = search_worker.clone();
                    let _ = thread::Builder::new()
                        .name("rmac-notes-search-close".into())
                        .spawn(move || {
                            let _ = search_worker.shutdown_blocking();
                        });
                }
            }
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
        let folder_dialog = self.render_folder_dialog(cx);
        let purge_dialog = self.render_purge_dialog(cx);
        let move_dialog = self.render_move_dialog(cx);
        let attachment_dialog = self.render_attachment_dialog(cx);
        let export_dialog = self.render_export_dialog(cx);
        let markdown_import_dialog = self.render_markdown_import_dialog(cx);
        let bundle_import_dialog = self.render_bundle_import_dialog(cx);

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
            .on_action(
                cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExportNotes, _, cx| this.begin_export(cx)))
            .on_action(cx.listener(|this, _: &RenameSelectedFolder, window, cx| {
                this.begin_folder_rename(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &DeleteSelectedFolder, _, cx| this.begin_folder_delete(cx)),
            )
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.request_close(window, cx)
            }))
            .size_full()
            .bg(mac::window())
            .text_color(mac::text())
            .child(content)
            .when_some(folder_dialog, |element, dialog| element.child(dialog))
            .when_some(purge_dialog, |element, dialog| element.child(dialog))
            .when_some(move_dialog, |element, dialog| element.child(dialog))
            .when_some(attachment_dialog, |element, dialog| element.child(dialog))
            .when_some(export_dialog, |element, dialog| element.child(dialog))
            .when_some(markdown_import_dialog, |element, dialog| {
                element.child(dialog)
            })
            .when_some(bundle_import_dialog, |element, dialog| {
                element.child(dialog)
            })
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

fn bridge_search_events(
    events: NotesSearchWorkerEvents,
) -> io::Result<async_channel::Receiver<SearchWorkerEvent>> {
    let (sender, receiver) = async_channel::bounded(SEARCH_EVENT_CAPACITY);
    thread::Builder::new()
        .name("rmac-notes-ui-search-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if sender.send_blocking(event).is_err() {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

fn bridge_preview_events(
    events: NotesPreviewWorkerEvents,
) -> io::Result<async_channel::Receiver<PreviewBridgeEvent>> {
    let (sender, receiver) = async_channel::bounded(PREVIEW_EVENT_CAPACITY);
    thread::Builder::new()
        .name("rmac-notes-ui-preview-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                let rendered = match &event {
                    PreviewWorkerEvent::Ready { image, .. } => render_preview_image(image),
                    _ => None,
                };
                if sender
                    .send_blocking(PreviewBridgeEvent { event, rendered })
                    .is_err()
                {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

fn render_preview_image(preview: &DecodedImagePreview) -> Option<Arc<RenderImage>> {
    let expected = u64::from(preview.width())
        .checked_mul(u64::from(preview.height()))?
        .checked_mul(4)?;
    if expected != preview.rgba().len() as u64 {
        return None;
    }
    let mut bgra = Vec::with_capacity(preview.rgba().len());
    let mut pixels = preview.rgba().chunks_exact(4);
    for pixel in &mut pixels {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    if !pixels.remainder().is_empty() {
        return None;
    }
    let buffer = image::RgbaImage::from_raw(preview.width(), preview.height(), bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
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

fn centered_attachment_state(
    message: &'static str,
    retry_label: Option<&'static str>,
    cx: &mut Context<NotesView>,
) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(mac::control_fill())
        .child(
            div()
                .v_flex()
                .items_center()
                .gap_2()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(mac::text_secondary())
                .child(message)
                .when_some(retry_label, |element, label| {
                    element.child(
                        Button::new("retry-attachment-preview", label)
                            .xsmall()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.retry_attachment_preview(cx)),
                            ),
                    )
                }),
        )
        .into_any_element()
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
        WorkerFailure::MissingMarkdownImportReview => {
            "Review the selected Markdown file again before importing".into()
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

fn parse_tags(input: &str) -> Result<Vec<String>, &'static str> {
    let mut tags = Vec::new();
    let mut unique = BTreeSet::new();
    for value in input.split(',') {
        let value = value.trim().trim_start_matches('#').trim();
        if value.is_empty() {
            continue;
        }
        if value.len() > MAX_TAG_BYTES || value.chars().any(char::is_control) {
            return Err("Each Notes tag must be valid text no longer than 256 bytes");
        }
        if unique.insert(value.to_lowercase()) {
            if tags.len() == MAX_TAGS_PER_NOTE {
                return Err("A note can contain at most 32 tags");
            }
            tags.push(value.to_string());
        }
    }
    Ok(tags)
}

fn display_title(title: &str) -> SharedString {
    if title.trim().is_empty() {
        "New Note".into()
    } else {
        title.to_string().into()
    }
}

fn safe_export_stem(label: &str) -> String {
    const MAX_STEM_BYTES: usize = 80;
    let mut stem = String::new();
    let mut previous_space = false;
    for character in label.trim().chars() {
        let character = if character.is_control() || "/\\:*?\"<>|".contains(character) {
            '-'
        } else if character.is_whitespace() {
            ' '
        } else {
            character
        };
        if character == ' ' && previous_space {
            continue;
        }
        if stem.len().saturating_add(character.len_utf8()) > MAX_STEM_BYTES {
            break;
        }
        stem.push(character);
        previous_space = character == ' ';
    }
    let stem = stem.trim().trim_matches('.').trim();
    if stem.is_empty() {
        "Notes".into()
    } else {
        stem.into()
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

fn format_storage_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes_f64 = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes_f64 < MIB {
        format!("{:.1} KiB", bytes_f64 / KIB)
    } else if bytes_f64 < GIB {
        format!("{:.1} MiB", bytes_f64 / MIB)
    } else {
        format!("{:.1} GiB", bytes_f64 / GIB)
    }
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
