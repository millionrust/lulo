//! rmac Notes live application.
//!
//! The GPUI view is a projection of `rmac-notes-runtime`: it never scans or
//! writes the library directly. Stable IDs, accepted snapshots, recovery, and
//! the single writer remain authoritative off the UI thread.

mod dialog_presentation;
mod edit_recovery_controller;
mod editor_presentation;
mod input_support;
mod library_actions;
mod markdown_presentation;
mod note_navigation;
mod presentation;
mod preview_controller;
mod recovery_presentation;
mod root_presentation;
mod runtime_controller;
mod search_controller;
mod search_highlight;
mod startup_controller;
mod status_presentation;
mod toolbar;
mod transfer_controller;
mod view_model;
mod worker_bridge;
mod worker_event_controller;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::thread;

use gpui::{
    accesskit, actions, div, img, prelude::FluentBuilder as _, px, AccessibleAction, AnyElement,
    AppContext as _, Context, Div, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ObjectFit, ParentElement, Render, RenderImage, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, StyledImage as _, Window,
};
use gpui_component::{Icon, IconName, Sizable as _, Size, StyledExt as _};
use rmac_editor::InputState;
use rmac_notes_runtime::{
    ActionRequest, ActionResult, BundleImportAcceptRequest, BundleImportReviewRequest,
    DraftRecoveryKind, EditGeneration, ExportRequest, LibraryAction, MarkdownPreviewState,
    MarkdownPreviewWorkerEvent, MarkdownPreviewWorkerSendError, NotesMarkdownPreviewSession,
    NotesMarkdownPreviewWorker, NotesMarkdownPreviewWorkerClient, NotesPreviewSession,
    NotesPreviewWorker, NotesPreviewWorkerClient, NotesSearchSession, NotesSearchWorker,
    NotesSearchWorkerClient, NotesSession, NotesWorker, NotesWorkerClient, PreviewState,
    PreviewWorkerEvent, PreviewWorkerSendError, ScheduledEdit, SearchField, SearchHit, SearchState,
    SearchWorkerEvent, SearchWorkerSendError, SessionPhase, WorkerCommand, WorkerEvent,
    WorkerFailure, WorkerSendError, EVENT_CAPACITY, MARKDOWN_PREVIEW_EVENT_CAPACITY,
    MAX_SEARCH_RESULTS, PREVIEW_EVENT_CAPACITY, SEARCH_EVENT_CAPACITY,
};
use rmac_notes_storage::{
    resolve_notes_paths, ExportFormat, ExportOutcome, MarkdownImportReview, PendingReason,
    PreviewSize,
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
    attachment_match_row, centered_state, date_label, date_section, folder_row,
    format_storage_bytes, full_date_label, styled_search_fragment, tag_pill,
};
use search_highlight::{
    matched_search_fragment, plain_search_fragment, MAX_SEARCH_DETAIL_FRAGMENT_CHARS,
    MAX_SEARCH_LABEL_FRAGMENT_CHARS, MAX_SEARCH_TITLE_FRAGMENT_CHARS,
};
use view_model::*;
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
        DeleteSelectedFolder,
        InsertChecklist
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

impl NotesView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let inputs = Self::initialize_inputs(window, cx);
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
            search_query: inputs.search_query,
            folder_name_input: inputs.folder_name_input,
            title: inputs.title,
            tags: inputs.tags,
            body: inputs.body,
            focus: inputs.focus,
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

        view.start_workers(window, cx);

        // The Dock's or the menu bar's Quit, ⌘Q, ⌘Tab's Q and logging out
        // close the window through the compositor; route them through the
        // same review as ⌘W so a pending change or a running import is never
        // cut off. The view removes the window itself once it may close.
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |view, cx| {
                if view.closing {
                    return true;
                }
                view.request_close(window, cx);
                if !view.closing {
                    window.activate_window();
                }
                false
            })
            .unwrap_or(true)
        });
        view
    }

    fn apply_worker_event(
        &mut self,
        event: WorkerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_worker_event_with(event, window, cx, |_| {});
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
        self.continue_close(window, cx);
    }
    fn render_status_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.render_status_banner_with(None, cx)
    }
}

impl Render for NotesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_root(None, window, cx)
    }
}

fn worker_failure_message(failure: WorkerFailure) -> String {
    match failure {
        WorkerFailure::WrongPhase => "Notes is not ready for that action".into(),
        WorkerFailure::CommitPending => "Resolve the pending Notes change first".into(),
        WorkerFailure::PendingConflict(error) => error.to_string(),
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
    // Notes' first frame is a cheap "Opening Notes…" placeholder while the
    // library worker starts (see SessionPhase::Starting); the performance
    // harness must time launch-to-interactive against the frame that shows
    // the real library list and selected note, not that placeholder. See
    // render_root's Ready/Maintenance/Pending/Stopped arm, which calls
    // rmac_ui::mark_content_ready.
    rmac_ui::defer_content_ready();
    rmac_ui::boot_app(
        rmac_ui::app_id::NOTES,
        "Notes",
        1080.0,
        720.0,
        NotesView::new,
    );
}
