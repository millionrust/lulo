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
mod toolbar;
mod transfer_controller;
mod worker_bridge;
mod worker_event_controller;

use std::collections::BTreeSet;
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
    attachment_match_row, centered_state, date_label, folder_row, format_storage_bytes,
    styled_search_fragment, tag_pill,
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

        view.start_workers(window, cx);
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

impl Render for NotesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_root(None, cx)
    }
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
