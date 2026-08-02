//! rmac Notes live application.
//!
//! The GPUI view is a projection of `rmac-notes-runtime`: it never scans or
//! writes the library directly. Stable IDs, accepted snapshots, recovery, and
//! the single writer remain authoritative off the UI thread.

mod dialog_presentation;
mod editor_presentation;
mod input_support;
mod library_actions;
mod markdown_presentation;
mod note_navigation;
mod presentation;
mod preview_controller;
mod search_highlight;
mod toolbar;
mod transfer_controller;
mod worker_bridge;

use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::thread;

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
    DraftRecoveryKind, EditGeneration, ExportRequest, LibraryAction, MarkdownPreviewState,
    MarkdownPreviewWorkerEvent, MarkdownPreviewWorkerSendError, NotesMarkdownPreviewSession,
    NotesMarkdownPreviewWorker, NotesMarkdownPreviewWorkerClient, NotesMarkdownPreviewWorkerEvents,
    NotesPreviewSession, NotesPreviewWorker, NotesPreviewWorkerClient, NotesPreviewWorkerEvents,
    NotesSearchSession, NotesSearchWorker, NotesSearchWorkerClient, NotesSearchWorkerEvents,
    NotesSession, NotesWorker, NotesWorkerClient, NotesWorkerEvents, PreviewState,
    PreviewWorkerEvent, PreviewWorkerSendError, ScheduledEdit, SearchField, SearchHit, SearchState,
    SearchWorkerEvent, SearchWorkerSendError, SessionPhase, WorkerCommand, WorkerEvent,
    WorkerFailure, WorkerSendError, EVENT_CAPACITY, MARKDOWN_PREVIEW_EVENT_CAPACITY,
    MAX_SEARCH_RESULTS, PREVIEW_EVENT_CAPACITY, SEARCH_EVENT_CAPACITY,
};
use rmac_notes_storage::{
    resolve_notes_paths, DecodedImagePreview, ExportFormat, ExportOutcome, MarkdownImportReview,
    PendingReason, PreviewSize,
};
use rmac_notes_store::{
    AttachmentId, BundleCollisionPolicy, BundleImportReview, ExportScope, FolderId, NewNote,
    NoteChanges, NoteId, NoteRecord, SortOrder,
};
use rmac_ui::{mac, Button, InputEvent, TextField};

use input_support::{
    display_title, now_unix_ms, parse_tags, safe_export_stem, take_counter, unique_folder_name,
};
use markdown_presentation::render_markdown_document;
use presentation::{
    attachment_match_row, date_label, folder_row, format_storage_bytes, styled_search_fragment,
    tag_pill,
};
use search_highlight::{
    matched_search_fragment, plain_search_fragment, SearchTextFragment,
    MAX_SEARCH_DETAIL_FRAGMENT_CHARS, MAX_SEARCH_LABEL_FRAGMENT_CHARS,
    MAX_SEARCH_TITLE_FRAGMENT_CHARS,
};
use worker_bridge::PreviewBridgeEvent;

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
    markdown_preview_worker: Option<NotesMarkdownPreviewWorkerClient>,
    session: NotesSession,
    search: NotesSearchSession,
    preview: NotesPreviewSession,
    markdown_preview: NotesMarkdownPreviewSession,
    markdown_preview_visible: bool,
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
    markdown_preview_shutdown_requested: bool,
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
            markdown_preview_worker: None,
            session: NotesSession::new(),
            search: NotesSearchSession::new(),
            preview: NotesPreviewSession::new(),
            markdown_preview: NotesMarkdownPreviewSession::new(),
            markdown_preview_visible: false,
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
            markdown_preview_shutdown_requested: false,
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

        match NotesMarkdownPreviewWorker::start()
            .map_err(|error| error.to_string())
            .and_then(|worker| {
                let (client, events) = worker.into_parts();
                bridge_markdown_preview_events(events)
                    .map(|receiver| (client, receiver))
                    .map_err(|error| {
                        format!("Notes could not start its Markdown preview bridge: {error}")
                    })
            }) {
            Ok((client, receiver)) => {
                view.markdown_preview_worker = Some(client);
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(event) = receiver.recv().await {
                        if this
                            .update_in(cx, |this, _window, cx| {
                                this.apply_markdown_preview_event(event, cx)
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

    fn apply_markdown_preview_event(
        &mut self,
        event: MarkdownPreviewWorkerEvent,
        cx: &mut Context<Self>,
    ) {
        if matches!(&event, MarkdownPreviewWorkerEvent::Stopped { .. }) {
            let unexpected = !self.closing && !self.markdown_preview_shutdown_requested;
            self.markdown_preview.clear();
            self.markdown_preview_shutdown_requested = true;
            self.markdown_preview_worker = None;
            if unexpected {
                self.message = Some("Notes Markdown preview stopped unexpectedly".into());
            }
            cx.notify();
            return;
        }
        if event.project(&mut self.markdown_preview) {
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
        self.sync_markdown_preview(cx);
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
        if !self.request_markdown_preview_shutdown(cx) {
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

    fn request_markdown_preview_shutdown(&mut self, cx: &mut Context<Self>) -> bool {
        if self.markdown_preview_shutdown_requested {
            return true;
        }
        self.markdown_preview.clear();
        let result = self
            .markdown_preview_worker
            .as_ref()
            .ok_or(MarkdownPreviewWorkerSendError::Closed)
            .and_then(NotesMarkdownPreviewWorkerClient::try_shutdown);
        match result {
            Ok(()) | Err(MarkdownPreviewWorkerSendError::Closed) => {
                self.markdown_preview_shutdown_requested = true;
                true
            }
            Err(error) => {
                self.message = Some(
                    format!("Notes Markdown preview is still finishing: {error}. Try again.")
                        .into(),
                );
                cx.notify();
                false
            }
        }
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
            self.markdown_preview.clear();
            if let Some(markdown_preview_worker) = &self.markdown_preview_worker {
                if matches!(
                    markdown_preview_worker.try_shutdown(),
                    Err(MarkdownPreviewWorkerSendError::Full)
                ) {
                    let markdown_preview_worker = markdown_preview_worker.clone();
                    let _ = thread::Builder::new()
                        .name("rmac-notes-markdown-preview-close".into())
                        .spawn(move || {
                            let _ = markdown_preview_worker.shutdown_blocking();
                        });
                }
            }
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
    worker_bridge::bridge_worker_events(events, EVENT_CAPACITY)
}

fn bridge_search_events(
    events: NotesSearchWorkerEvents,
) -> io::Result<async_channel::Receiver<SearchWorkerEvent>> {
    worker_bridge::bridge_search_events(events, SEARCH_EVENT_CAPACITY)
}

fn bridge_preview_events(
    events: NotesPreviewWorkerEvents,
) -> io::Result<async_channel::Receiver<PreviewBridgeEvent>> {
    worker_bridge::bridge_preview_events(events, PREVIEW_EVENT_CAPACITY, render_preview_image)
}

fn bridge_markdown_preview_events(
    events: NotesMarkdownPreviewWorkerEvents,
) -> io::Result<async_channel::Receiver<MarkdownPreviewWorkerEvent>> {
    worker_bridge::bridge_markdown_preview_events(events, MARKDOWN_PREVIEW_EVENT_CAPACITY)
}

fn render_preview_image(preview: &DecodedImagePreview) -> Option<Arc<RenderImage>> {
    worker_bridge::render_preview_image(preview)
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

fn main() {
    rmac_ui::boot("Notes", 1080.0, 720.0, |window, cx| {
        NotesView::new(window, cx)
    });
}
