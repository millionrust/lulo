//! rmac Notes live application.
//!
//! The GPUI view is a projection of `rmac-notes-runtime`: it never scans or
//! writes the library directly. Stable IDs, accepted snapshots, recovery, and
//! the single writer remain authoritative off the UI thread.

mod audio_recorder;
mod dialog_presentation;
mod edit_recovery_controller;
mod edit_text_assist_controller;
mod editor_presentation;
mod file_choosers;
mod glyphs;
mod input_support;
mod library_actions;
mod markdown_presentation;
mod note_context_menu;
mod note_find_controller;
mod note_format_controller;
mod note_navigation;
mod note_window;
mod notes_style;
mod presentation;
mod preview_controller;
mod print_controller;
mod quick_note;
mod recovery_presentation;
mod root_presentation;
mod runtime_controller;
mod search_controller;
mod search_highlight;
#[cfg(target_os = "linux")]
mod session_lock_watch;
mod settings_window;
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
    AppContext as _, Context, Div, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit, ParentElement,
    Pixels, Point, Render, RenderImage, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, StyledImage as _, Window, WindowHandle,
};
use gpui_component::{Icon, IconName, Size, StyledExt as _};
use rmac_editor::InputState;
use rmac_notes_runtime::{
    ActionRequest, ActionResult, BundleImportAcceptRequest, BundleImportReviewRequest,
    DraftRecoveryKind, EditGeneration, ExportRequest, LibraryAction, LockCredential, LockSecret,
    MarkdownPreviewState, MarkdownPreviewWorkerEvent, MarkdownPreviewWorkerSendError, NotesKeyring,
    NotesMarkdownPreviewSession, NotesMarkdownPreviewWorker, NotesMarkdownPreviewWorkerClient,
    NotesPreviewSession, NotesPreviewWorker, NotesPreviewWorkerClient, NotesSearchSession,
    NotesSearchWorker, NotesSearchWorkerClient, NotesSession, NotesWorker, NotesWorkerClient,
    PreviewState, PreviewWorkerEvent, PreviewWorkerSendError, ScheduledEdit, SearchField,
    SearchHit, SearchState, SearchWorkerEvent, SearchWorkerSendError, SessionPhase, WorkerCommand,
    WorkerEvent, WorkerFailure, WorkerSendError, EVENT_CAPACITY, MARKDOWN_PREVIEW_EVENT_CAPACITY,
    MAX_SEARCH_RESULTS, PREVIEW_EVENT_CAPACITY, SEARCH_EVENT_CAPACITY,
};
use rmac_notes_storage::{
    resolve_notes_paths, ExportFormat, ExportOutcome, MarkdownImportReview, PendingReason,
    PreviewSize,
};
use rmac_notes_store::{
    AttachmentId, BundleCollisionPolicy, BundleImportReview, ExportScope, FolderId, LockError,
    NewNote, NoteChanges, NoteId, NoteRecord, SmartFolderId, SortOrder,
};
use rmac_ui::{mac, AccessibleTextInput as _, Button, InputEvent, PopUpButton, TextField};

use glyphs::glyph;
use input_support::{
    display_title, now_unix_ms, parse_tags, safe_export_stem, take_counter, unique_folder_name,
};
use markdown_presentation::render_markdown_document;
use note_format_controller::{
    ChecklistBulkAction, CopiedStyle, ListMarker, ParagraphStyle, TextTransform,
};
use notes_style::*;
use presentation::{
    attachment_match_row, centered_state, date_label, date_section, folder_row,
    format_storage_bytes, full_date_label, styled_search_fragment_in, tag_pill,
};
use search_highlight::{
    matched_search_fragment, plain_search_fragment, MAX_SEARCH_DETAIL_FRAGMENT_CHARS,
    MAX_SEARCH_LABEL_FRAGMENT_CHARS, MAX_SEARCH_TITLE_FRAGMENT_CHARS,
};
use view_model::*;
use worker_bridge::PreviewBridgeEvent;

actions!(
    notes,
    [
        ComposeNote,
        CreateFolder,
        TrashOrRestore,
        DeleteSelectedNote,
        CloseAll,
        QuitAndKeepWindows,
        ShowSettings,
        FocusMainWindow,
        PreviousRecentNote,
        NextRecentNote,
        ClearRecentNotes,
        ToggleFullScreen,
        OpenRecentNote0,
        OpenRecentNote1,
        OpenRecentNote2,
        OpenRecentNote3,
        OpenRecentNote4,
        OpenRecentNote5,
        OpenRecentNote6,
        OpenRecentNote7,
        OpenRecentNote8,
        OpenRecentNote9,
        TogglePin,
        DuplicateNote,
        SortByEdited,
        SortByCreated,
        SortByTitle,
        FocusSearch,
        FindInNote,
        FindAndReplace,
        FindInNoteNext,
        FindInNotePrevious,
        UseSelectionForFind,
        JumpToSelection,
        PastePlainText,
        MakeUppercase,
        MakeLowercase,
        Capitalise,
        ExportNotes,
        RenameSelectedFolder,
        DeleteSelectedFolder,
        InsertChecklist,
        ToggleChecklistDone,
        TickAll,
        UntickAll,
        MoveTickedToBottom,
        DeleteTicked,
        MoveItemUp,
        MoveItemDown,
        InsertTable,
        ConvertToText,
        ToggleBold,
        ToggleItalic,
        ToggleStrikethrough,
        SetStyleTitle,
        SetStyleHeading,
        SetStyleSubheading,
        SetStyleBody,
        SetStyleMonospaced,
        InsertBulletedList,
        InsertNumberedList,
        InsertDashedList,
        InsertBlockQuote,
        InsertLink,
        IncreaseIndent,
        DecreaseIndent,
        MoveSelectedNote,
        DeleteNotePermanently,
        EmptyRecentlyDeleted,
        ToggleMarkdownPreview,
        ToggleLightBackground,
        ToggleFolders,
        ToggleNoteCount,
        ToggleToolbar,
        ShowListView,
        ShowGalleryView,
        ToggleAttachmentsBrowser,
        ShowAttachmentInNote,
        SetAllAttachmentsSmall,
        SetAllAttachmentsLarge,
        CollapseSection,
        CollapseAllSections,
        ExpandSection,
        ExpandAllSections,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ImportNote,
        ImportMarkdown,
        ImportNotesBundle,
        AddPhoto,
        PrintNote,
        ExportNotePdf,
        ExportNoteMarkdown,
        ShowSpellingAndGrammar,
        CheckDocumentNow,
        ToggleCheckSpellingWhileTyping,
        ToggleCheckGrammarWithSpelling,
        ToggleCorrectSpellingAutomatically,
        ShowSubstitutions,
        ToggleSmartCopyPaste,
        ToggleSmartQuotes,
        ToggleSmartLists,
        ToggleSmartDashes,
        ToggleSmartLinks,
        ToggleSmartTags,
        ToggleTextReplacement,
        StartSpeaking,
        StopSpeaking,
        ToggleUnderline,
        ToggleHighlight,
        FontBigger,
        FontSmaller,
        CopyStyle,
        PasteStyle,
        ToggleSuperscript,
        ToggleSubscript,
        BaselineUseDefault,
        RemoveStyle,
        AlignLeft,
        AlignCentre,
        AlignRight,
        MathsResultsOff,
        MathsResultsSuggest,
        MathsResultsInsert,
        CreateSmartFolder,
        CreateSmartFolderFromSelection,
        ToggleLockNote,
        CloseAllLockedNotes,
        ToggleShowHighlights,
        CustomiseToolbar,
        AttachFile,
        RenameAttachment,
        PasteAndRetainStyle,
        ShowSmartFoldersHelp,
        ShowTagsHelp,
        RecordAudio,
        OpenNoteInNewWindow,
        QuickNote
    ]
);

/// The note-list context menu's "Move to" submenu row for one destination
/// (`None` is "All Notes"). A payload action, like Files'
/// `GoToTitlePathAction`, since the plain `actions!` macro above only makes
/// zero-field markers.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = notes, no_json)]
struct MoveNoteToFolderAction {
    folder_id: Option<FolderId>,
}

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
    light_background_notes: BTreeSet<NoteId>,
    folders_visible: bool,
    show_note_count: bool,
    group_notes_by_date: bool,
    toolbar_visible: bool,
    gallery_view: bool,
    attachments_browser_visible: bool,
    attachment_view_large: bool,
    collapsed_sections: BTreeSet<String>,
    note_zoom: i8,
    preview_image: Option<Arc<RenderImage>>,
    selected_attachment: Option<AttachmentId>,
    search_query: Entity<InputState>,
    folder_name_input: Entity<InputState>,
    title: Entity<InputState>,
    tags: Entity<InputState>,
    body: Entity<InputState>,
    focus: FocusHandle,
    /// The note list's scroll position, which picks the rows it builds.
    note_list_scroll: gpui::ScrollHandle,
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
    /// The print dialog is open; the window stays until the portal answers.
    print_busy: bool,
    /// Bumped on every edit, so a print whose note changed while its dialog
    /// was open prints nothing.
    print_generation: Arc<std::sync::atomic::AtomicU64>,
    /// A press on a toolbar's empty area, turned into a window move by the
    /// next pointer motion (as in Files).
    dragging: Option<gpui::Point<gpui::Pixels>>,
    /// In-note Find (⌘F), separate from the note list's search (⌥⌘F).
    note_find_open: bool,
    note_replace_open: bool,
    note_find_input: Entity<InputState>,
    note_replace_input: Entity<InputState>,
    /// Byte offsets of every case-insensitive match of the query in the
    /// selected note's body.
    note_find_matches: Vec<usize>,
    note_find_current: usize,
    /// The note plain ⌫ most recently moved to Trash, so the status
    /// banner's Undo button can bring back exactly that one.
    pending_undo_trash: Option<(NoteId, u64)>,
    /// Most recently visited notes, newest first. Navigation leaves this
    /// order intact so Previous/Next can traverse it in both directions.
    recent_notes: Vec<NoteId>,
    recent_position: Option<usize>,
    last_editor_note: Option<NoteId>,
    /// Edit ▸ Spelling and Grammar / Substitutions for the note body.
    text_assist: rmac_ui::text_assist::TextAssistSettings,
    spell_checker: Arc<rmac_spelling::HunspellChecker>,
    /// Edit ▸ Substitutions ▸ Smart Lists and Smart Tags: checked,
    /// persisted for this session, with no live effect wired yet (see
    /// docs/parity.md NOTES-15).
    smart_lists: bool,
    smart_tags: bool,
    /// Format ▸ Font ▸ Copy Style/Paste Style.
    copied_style: Option<CopiedStyle>,
    /// Format ▸ Maths Results / the toolbar's maths-results button.
    maths_results_mode: MathsResultsMode,
    /// View ▸ Show Highlights: whether Format ▸ Font ▸ Highlight spans draw
    /// their yellow background. The `==marker==` stays in the body either
    /// way; this only hides the visual effect.
    show_highlights: bool,
    /// Notes ▸ Settings… ▸ Use dark backgrounds for note content: the
    /// baseline every note's own Format ▸ Show Note with Light Background
    /// choice (`light_background_notes`) flips from. `false` (dark) matches
    /// Lulo's behaviour before this setting existed.
    light_background_default: bool,
    /// The session's open locked-note keys, shared with the library and
    /// preview workers (never read here; see `view_model::LockDialog`).
    keyring: NotesKeyring,
    lock_dialog: Option<LockDialog>,
    lock_password_input: Entity<InputState>,
    lock_verify_input: Entity<InputState>,
    lock_hint_input: Entity<InputState>,
    lock_old_password_input: Entity<InputState>,
    /// The inline error under the password fields (wrong password + hint).
    lock_dialog_error: Option<SharedString>,
    /// The worker request a password dialog is waiting on.
    lock_request_id: Option<u64>,
    /// Last person-driven activity in Notes; open locked notes close after
    /// `LOCKED_NOTES_IDLE_TIMEOUT` without any.
    last_activity: std::time::Instant,
    idle_lock_timer: Option<gpui::Task<()>>,
    /// The selected Smart Folder (stored with the library), if any; narrows
    /// the note list by its tag on top of the ordinary folder selection.
    smart_folder_filter: Option<SmartFolderId>,
    /// `Some(prefill)` while the "name this Smart Folder" dialog is open.
    smart_folder_dialog: Option<String>,
    smart_folder_name_input: Entity<InputState>,
    /// View ▸ Customise Toolbar…
    hidden_toolbar_items: BTreeSet<ToolbarItem>,
    customise_toolbar_open: bool,
    /// Edit ▸ Attach File…: `(note id, chip filename) -> chosen path`. The
    /// chip text itself (`📎 filename`) is a plain line in the note's
    /// Markdown body, so it is durable; only the clickable "open this path"
    /// behaviour is session-only (see docs/parity.md).
    attachment_chip_paths:
        std::collections::BTreeMap<NoteId, std::collections::BTreeMap<String, std::path::PathBuf>>,
    /// Notes ▸ Settings… ▸ Automatically sort ticked items.
    auto_sort_ticked_items: bool,
    /// Help ▸ Using Smart Folders/Using Tags: `Some(body text)` while the
    /// local help alert is open (NOT-MENU-065/066).
    notes_help: Option<&'static str>,
    /// Edit ▸ Rename Attachment…: `Some((id, expected revision))` while the
    /// rename dialog is open for that attachment.
    attachment_rename: Option<(AttachmentId, u64)>,
    attachment_rename_input: Entity<InputState>,
    /// Edit ▸ Record Audio… (NOT-MENU-008): `Some` while `pw-record` is
    /// capturing to a temp file, awaiting Stop.
    recording: Option<audio_recorder::AudioRecording>,
    /// Window ▸ Open Note in New Window (NOT-MENU-064): a second window
    /// mirroring the shared editor fields (`title`/`body`), reused if
    /// already open.
    note_window: Option<WindowHandle<rmac_ui::Root>>,
    /// Quick Note (NOT-025): the small floating note window, and the two
    /// settings that govern it. Session-only, like every other Notes ▸
    /// Settings… control (NOTES-13) — not yet persisted across relaunch.
    quick_note_window: Option<WindowHandle<rmac_ui::Root>>,
    quick_note_id: Option<NoteId>,
    always_resume_quick_note: bool,
    /// The note-list / folder-sidebar right-click menu (NOTES-xx: right-
    /// clicking a note or folder did nothing). Opened on right mouse-down,
    /// closed by `rmac_ui::DismissMenu` (Escape or a click outside).
    /// `context_menu_target` says which content `render_context_menu`
    /// builds for it.
    context_menu: Option<rmac_ui::ContextMenuState>,
    context_menu_target: note_context_menu::ContextMenuTarget,
}

impl NotesView {
    /// Format ▸ Show Note with Light Background flips this note's own
    /// membership in `light_background_notes` regardless of the global
    /// default, so the set always means "notes that differ from the
    /// current Notes ▸ Settings… ▸ Use dark backgrounds for note content
    /// baseline" rather than "notes forced light": changing the global
    /// default elsewhere never needs to rewrite this set.
    fn note_has_light_background(&self) -> bool {
        self.session.selected_note().is_some_and(|note| {
            self.light_background_default ^ self.light_background_notes.contains(&note.id)
        })
    }

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
            light_background_notes: BTreeSet::new(),
            folders_visible: true,
            show_note_count: true,
            group_notes_by_date: true,
            toolbar_visible: true,
            gallery_view: false,
            attachments_browser_visible: false,
            attachment_view_large: true,
            collapsed_sections: BTreeSet::new(),
            note_zoom: 0,
            preview_image: None,
            selected_attachment: None,
            search_query: inputs.search_query,
            folder_name_input: inputs.folder_name_input,
            title: inputs.title,
            tags: inputs.tags,
            body: inputs.body,
            focus: inputs.focus,
            note_list_scroll: gpui::ScrollHandle::new(),
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
            print_busy: false,
            print_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            dragging: None,
            note_find_open: false,
            note_replace_open: false,
            note_find_input: inputs.note_find,
            note_replace_input: inputs.note_replace,
            note_find_matches: Vec::new(),
            note_find_current: 0,
            pending_undo_trash: None,
            recent_notes: Vec::new(),
            recent_position: None,
            last_editor_note: None,
            text_assist: rmac_ui::text_assist::TextAssistSettings::default(),
            spell_checker: rmac_spelling::shared(),
            smart_lists: false,
            smart_tags: false,
            copied_style: None,
            maths_results_mode: MathsResultsMode::default(),
            show_highlights: true,
            light_background_default: false,
            keyring: NotesKeyring::new(),
            lock_dialog: None,
            lock_password_input: inputs.lock_password,
            lock_verify_input: inputs.lock_verify,
            lock_hint_input: inputs.lock_hint,
            lock_old_password_input: inputs.lock_old_password,
            lock_dialog_error: None,
            lock_request_id: None,
            last_activity: std::time::Instant::now(),
            idle_lock_timer: None,
            smart_folder_filter: None,
            smart_folder_dialog: None,
            smart_folder_name_input: inputs.smart_folder_name,
            hidden_toolbar_items: BTreeSet::new(),
            customise_toolbar_open: false,
            attachment_chip_paths: std::collections::BTreeMap::new(),
            auto_sort_ticked_items: false,
            notes_help: None,
            attachment_rename: None,
            attachment_rename_input: inputs.attachment_rename,
            recording: None,
            note_window: None,
            quick_note_window: None,
            quick_note_id: None,
            always_resume_quick_note: false,
            context_menu: None,
            context_menu_target: note_context_menu::ContextMenuTarget::Note,
        };
        rmac_ui::set_menu_checked(
            "notes::ToggleCheckSpellingWhileTyping",
            view.text_assist.check_spelling_while_typing,
            cx,
        );
        rmac_ui::set_menu_checked(
            "notes::ToggleCheckGrammarWithSpelling",
            view.text_assist.check_grammar_with_spelling,
            cx,
        );
        rmac_ui::set_menu_checked(
            "notes::ToggleCorrectSpellingAutomatically",
            view.text_assist.correct_spelling_automatically,
            cx,
        );
        rmac_ui::set_menu_checked(
            "notes::ToggleSmartCopyPaste",
            view.text_assist.smart_copy_paste,
            cx,
        );
        rmac_ui::set_menu_checked(
            "notes::ToggleSmartQuotes",
            view.text_assist.smart_quotes,
            cx,
        );
        rmac_ui::set_menu_checked("notes::ToggleSmartLists", view.smart_lists, cx);
        rmac_ui::set_menu_checked(
            "notes::ToggleSmartDashes",
            view.text_assist.smart_dashes,
            cx,
        );
        rmac_ui::set_menu_checked("notes::ToggleSmartLinks", view.text_assist.smart_links, cx);
        rmac_ui::set_menu_checked("notes::ToggleSmartTags", view.smart_tags, cx);
        rmac_ui::set_menu_checked(
            "notes::ToggleTextReplacement",
            view.text_assist.text_replacement,
            cx,
        );
        rmac_ui::set_menu_enabled("notes::StopSpeaking", false, cx);

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

/// Open locked notes close after this long without activity in Notes (they
/// also close on sleep, on the lock screen and with Close All Locked Notes).
const LOCKED_NOTES_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8 * 60);

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
        WorkerFailure::Lock(LockError::WrongPassword) => "The password is incorrect.".into(),
        WorkerFailure::Lock(error) => error.to_string(),
        WorkerFailure::LockedNoteClosed => "Enter the locked notes password first".into(),
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
    // Notes is one window over one library: a second launch brings the
    // running Notes forward rather than opening a second, menu-less copy.
    rmac_ui::boot_single_window_app_with_assets(
        rmac_ui::app_id::NOTES,
        rmac_ui::layered_assets(glyphs::NotesAssets),
        "Notes",
        1080.0,
        720.0,
        NotesView::new,
        |cx: &mut gpui::App| {
            cx.on_action(|_: &ShowSettings, cx| {
                if cx.windows().is_empty() {
                    rmac_ui::open_another_window(Vec::new(), cx);
                    rmac_ui::dispatch_to_app_window(Box::new(ShowSettings), cx);
                }
            });
            cx.on_action(|_: &ComposeNote, cx| {
                if cx.windows().is_empty() {
                    rmac_ui::open_another_window(Vec::new(), cx);
                    rmac_ui::dispatch_to_app_window(Box::new(ComposeNote), cx);
                }
            });
            cx.on_action(|_: &FocusMainWindow, cx| {
                if cx.windows().is_empty() {
                    rmac_ui::open_another_window(Vec::new(), cx);
                }
            });
        },
    );
}
