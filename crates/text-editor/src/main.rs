//! rmac Text Editor — a fast, native TextEdit-style editor.
//!
//! Rope-backed `InputState` body on a clean themed page, with a unified desktop
//! toolbar (New / Open / Save), a find/replace bar (⌘F / ⇧⌘F), dirty-state
//! tracking with a modified indicator and unsaved-changes prompts, basic
//! autosave to a recovery file, and a Format affordance (monospace + font
//! size). Shares the editing configuration with Notes via `rmac-editor`.

mod document;
mod recovery;
mod rtf;
mod storage;

use std::{
    ffi::OsString,
    path::Path,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    actions, div, font, px, App, AppContext as _, Application, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement as _, Styled, StyledText, Subscription, TextRun,
    UnderlineStyle, Window,
};
use gpui_component::{Icon, IconName, Root, Size, StyledExt as _};
use notify::Watcher as _;
use rmac_ui::{
    mac, Button, InputEvent, InputState, Position, RopeExt as _, SearchField, TextField,
};

const CTX: &str = "TextEditor";
const WINDOW_WIDTH: f32 = 860.0;
const WINDOW_HEIGHT: f32 = 640.0;
const MAX_STARTUP_DOCUMENTS: usize = 32;
static STARTUP_RECOVERY_LOCK: Mutex<()> = Mutex::new(());
static NEXT_WINDOW_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Eq, PartialEq)]
struct StartupRequest {
    open_untitled: bool,
    paths: Vec<PathBuf>,
}

fn parse_startup_request(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<StartupRequest, &'static str> {
    let mut paths = Vec::new();
    let mut open_untitled = false;
    let mut options = true;
    for argument in arguments {
        if options && argument == "--" {
            options = false;
            continue;
        }
        if options && argument == "--new-document" {
            open_untitled = true;
            continue;
        }
        if options
            && argument
                .to_str()
                .is_some_and(|value| value.starts_with('-'))
        {
            return Err("unsupported Text Editor launch option");
        }
        let path = PathBuf::from(argument);
        if path.as_os_str().is_empty() {
            return Err("empty Text Editor launch path");
        }
        if paths.len() == MAX_STARTUP_DOCUMENTS {
            return Err("too many Text Editor startup documents");
        }
        paths.push(path);
    }
    if paths.is_empty() && !open_untitled {
        open_untitled = true;
    }
    Ok(StartupRequest {
        open_untitled,
        paths,
    })
}

actions!(
    text_editor,
    [
        NewFile,
        OpenFile,
        SaveFile,
        SaveFileAs,
        PrintFile,
        ToggleFind,
        ToggleReplace,
        FindNext,
        FindPrev,
        CloseBar,
        ToggleMono,
        SetEncodingUtf8,
        SetEncodingUtf8Bom,
        SetEncodingUtf16Le,
        SetEncodingUtf16Be,
        SetLineEndingLf,
        SetLineEndingCrLf,
        SetLineEndingCr,
        IncreaseFont,
        DecreaseFont,
        CloseWindow,
    ]
);

/// A pending document switch that must wait on an unsaved-changes prompt.
#[derive(Clone, Copy)]
enum Pending {
    Close,
}

#[derive(Default)]
struct RecoveryClock {
    generation: u64,
}

impl RecoveryClock {
    fn arm(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn should_write(&self, generation: u64, dirty: bool) -> bool {
        dirty && self.generation == generation
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }
}

/// A modal alert awaiting the user, shown via the shared `rmac_ui::alert`.
#[derive(Clone)]
enum ActiveAlert {
    /// A recovery file was found — Restore (load it) or Discard.
    Recover(RecoveryPrompt),
    /// The buffer is dirty before `Pending` — Save / Don't Save / Cancel.
    ConfirmSave(Pending),
    /// The opened document no longer matches its retained exact revision.
    Conflict,
    /// The external bytes reviewed immediately before an explicit overwrite.
    ConfirmOverwrite { reviewed_revision: Vec<u8> },
    /// A document open/save error — title + message + OK.
    Error {
        title: &'static str,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalChange {
    Modified,
    Missing,
    Unreadable,
}

enum DocumentWatchEvent {
    Changed,
    Unavailable,
}

#[derive(Clone)]
struct RecoveryPrompt {
    content: String,
    document_label: String,
    format: document::TextFormat,
    additional_drafts: usize,
}

enum LoadedFile {
    Plain(document::DecodedDocument),
    RichText {
        text: String,
        runs: Vec<rtf::RtfRun>,
    },
}

#[derive(Debug)]
enum SaveFailure {
    Codec(document::CodecError),
    Storage(storage::SaveDocumentError),
    ConflictingCopyDestination,
}

impl std::fmt::Display for SaveFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
            Self::ConflictingCopyDestination => formatter.write_str(
                "Save a Copy requires a different file; the conflicting source was not changed",
            ),
        }
    }
}

struct EditorView {
    input: Entity<InputState>,
    path: Option<PathBuf>,
    /// Complete bytes read at open or exact successful save readback. Existing
    /// document saves must still match this revision immediately before write.
    saved_bytes: Option<Vec<u8>>,
    text_format: document::TextFormat,
    /// Format at the last exact successful read or write. A format-only
    /// conversion is an unsaved document change just like a text edit.
    saved_format: document::TextFormat,
    /// Text as last saved (or opened/new) — the dirty baseline.
    saved_value: String,
    dirty: bool,
    file_busy: bool,
    print_busy: bool,

    // Find / replace bar
    find_open: bool,
    replace_mode: bool,
    find_input: Entity<InputState>,
    replace_input: Entity<InputState>,
    /// Byte offsets of every match of the current query in the buffer.
    matches: Vec<usize>,
    /// Index into `matches` of the active match.
    current: usize,

    // Format
    mono: bool,
    font_size: f32,

    /// When an `.rtf` is opened, its parsed styled runs for the formatted
    /// preview. `Some` puts the editor in read-only RTF-viewer mode.
    rtf_runs: Option<Vec<rtf::RtfRun>>,

    // Infra
    focus: FocusHandle,
    recovery_directory: PathBuf,
    recovery_path: PathBuf,
    recovery_cleanup_paths: Vec<PathBuf>,
    recovery_clock: RecoveryClock,
    recovery_loading: bool,
    recovery_error: Option<SharedString>,
    status_notice: Option<SharedString>,
    window_generation: u64,
    document_generation: u64,
    current_document_generation: Arc<AtomicU64>,
    external_change: Option<ExternalChange>,
    document_watch_warning: bool,
    watched_directory: Option<PathBuf>,
    document_watcher: Option<notify::RecommendedWatcher>,
    pending_startup_path: Option<PathBuf>,
    /// The modal alert currently shown, if any (shared `rmac_ui::alert`).
    alert: Option<ActiveAlert>,
    _subscriptions: Vec<Subscription>,
}

fn open_editor_window(cx: &mut App, initial_path: Option<PathBuf>) -> Result<(), ()> {
    cx.open_window(
        rmac_ui::window_options_for_app(rmac_ui::app_id::TEXT_EDITOR, WINDOW_WIDTH, WINDOW_HEIGHT),
        |window, cx| {
            rmac_ui::prepare_surface_window(window, cx);
            let view = cx.new(|cx| EditorView::new_with_path(initial_path, window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .map(|_| ())
    .map_err(|_| ())
}

fn platform_recovery_path() -> Result<PathBuf, storage::Failure> {
    recovery_path_for_platform(
        cfg!(target_os = "macos"),
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn recovery_path_for_platform(
    macos: bool,
    xdg_state_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<PathBuf, storage::Failure> {
    if !macos {
        if let Some(path) = xdg_state_home.filter(|path| path.is_absolute()) {
            return Ok(path.join("rmac-text-editor/recovery.txt"));
        }
    }

    let home = home.ok_or_else(|| {
        storage::Failure::message(
            storage::Operation::ResolveRecoveryPath,
            Path::new("recovery.txt"),
            "HOME is not set and XDG_STATE_HOME is unavailable",
        )
    })?;
    if macos {
        Ok(home.join("Library/Application Support/rmac-text-editor/recovery.txt"))
    } else {
        Ok(home.join(".local/state/rmac-text-editor/recovery.txt"))
    }
}

struct StartupRecovery {
    directory: PathBuf,
    active_path: PathBuf,
    cleanup_paths: Vec<PathBuf>,
    prompt: Option<RecoveryPrompt>,
    warning: bool,
}

fn startup_recovery() -> StartupRecovery {
    // Multiple windows can launch concurrently. Serializing only this
    // background discovery/migration boundary prevents two windows from
    // importing the same legacy raw draft before either removes it.
    let _guard = STARTUP_RECOVERY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temporary_legacy = std::env::temp_dir().join("rmac-text-editor-recovery.txt");
    let (legacy_primary, mut warning) = match platform_recovery_path() {
        Ok(path) => (path, false),
        Err(_) => (temporary_legacy.clone(), true),
    };
    let directory = legacy_primary
        .parent()
        .map(|parent| parent.join("recovery"))
        .unwrap_or_else(|| std::env::temp_dir().join("rmac-text-editor-recovery"));
    let mut discovery = recovery::discover(&directory);
    warning |= discovery.unavailable || discovery.excessive || discovery.malformed > 0;

    let loaded_legacy = storage::load_legacy_recovery_drafts(
        &storage::RealStorage,
        &legacy_primary,
        &temporary_legacy,
    );
    warning |= loaded_legacy.warning.is_some();
    let mut cleanup_paths = Vec::new();
    let mut unmigrated = Vec::new();
    let legacy_count = loaded_legacy.drafts.len();
    for (index, draft) in loaded_legacy.drafts.into_iter().enumerate() {
        let record = recovery::RecoveryRecord::for_document(
            None,
            document::TextFormat::default(),
            draft.content,
        );
        let migrated_path = recovery::fresh_record_path(&directory);
        if recovery::save(&storage::RealStorage, &migrated_path, &record).is_ok() {
            if storage::remove_recovery_paths(&storage::RealStorage, &draft.paths).is_err() {
                warning = true;
                cleanup_paths.extend(draft.paths);
            }
            discovery.candidates.push(recovery::Candidate {
                path: migrated_path,
                record,
            });
        } else {
            warning = true;
            unmigrated.push((
                RecoveryPrompt {
                    content: record.content,
                    document_label: if legacy_count == 1 {
                        "Legacy unsaved document".into()
                    } else {
                        format!("Legacy unsaved document {}", index + 1)
                    },
                    format: record.format,
                    additional_drafts: 0,
                },
                draft.paths,
            ));
        }
    }

    discovery.candidates.sort_by(|left, right| {
        right
            .record
            .created_unix_ms
            .cmp(&left.record.created_unix_ms)
            .then_with(|| left.path.cmp(&right.path))
    });
    if !unmigrated.is_empty() {
        let additional_drafts = discovery
            .candidates
            .len()
            .saturating_add(unmigrated.len().saturating_sub(1));
        let (mut prompt, selected_paths) = unmigrated.remove(0);
        prompt.additional_drafts = additional_drafts;
        cleanup_paths.extend(selected_paths);
        let active_path = recovery::fresh_record_path(&directory);
        return StartupRecovery {
            directory,
            active_path,
            cleanup_paths,
            prompt: Some(prompt),
            warning,
        };
    }
    let mut selected = None;
    let mut remaining = discovery.candidates.len();
    for candidate in discovery.candidates {
        remaining = remaining.saturating_sub(1);
        match recovery::claim(&directory, candidate) {
            Ok(claimed) => {
                selected = Some(claimed);
                break;
            }
            Err(_) => warning = true,
        }
    }
    let active_path = selected
        .as_ref()
        .map(|candidate| candidate.path.clone())
        .unwrap_or_else(|| recovery::fresh_record_path(&directory));
    let prompt = selected.map(|candidate| RecoveryPrompt {
        content: candidate.record.content,
        document_label: candidate.record.document_label,
        format: candidate.record.format,
        additional_drafts: remaining,
    });
    StartupRecovery {
        directory,
        active_path,
        cleanup_paths,
        prompt,
        warning,
    }
}

fn load_selected_document(path: &Path) -> Result<LoadedFile, String> {
    let bytes = storage::read_bounded(
        &storage::RealStorage,
        storage::Operation::LoadDocument,
        path,
        document::MAX_DOCUMENT_BYTES,
    )
    .map_err(|_| "Text Editor could not read the selected document".to_string())?;
    let is_rtf = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rtf"));
    if is_rtf {
        let runs = rtf::parse_rtf(&bytes)
            .ok_or_else(|| "the RTF document could not be decoded safely".to_string())?;
        let text = runs.iter().map(|run| run.text.as_str()).collect();
        Ok(LoadedFile::RichText { text, runs })
    } else {
        document::decode(bytes)
            .map(LoadedFile::Plain)
            .map_err(|error| error.to_string())
    }
}

fn save_document(
    path: &Path,
    expected: Option<&[u8]>,
    text: &str,
    format: document::TextFormat,
) -> Result<document::DecodedDocument, SaveFailure> {
    let encoded = document::encode(text, format).map_err(SaveFailure::Codec)?;
    storage::write_document_if_unchanged(&storage::RealStorage, path, expected, &encoded)
        .map_err(SaveFailure::Storage)?;
    // Encoding a valid Rust string through a supported format is guaranteed to
    // decode. Keeping this fallible preserves the invariant without panicking.
    document::decode(encoded).map_err(SaveFailure::Codec)
}

fn same_file_identity(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    let (Ok(left_metadata), Ok(right_metadata)) =
        (std::fs::metadata(left), std::fs::metadata(right))
    else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        left_metadata.dev() == right_metadata.dev() && left_metadata.ino() == right_metadata.ino()
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    }
}

fn save_document_copy(
    path: &Path,
    forbidden_destination: Option<&Path>,
    text: &str,
    format: document::TextFormat,
) -> Result<document::DecodedDocument, SaveFailure> {
    if forbidden_destination.is_some_and(|source| same_file_identity(source, path)) {
        Err(SaveFailure::ConflictingCopyDestination)
    } else {
        save_document(path, None, text, format)
    }
}

fn inspect_external_revision(path: &Path, expected: &[u8]) -> Option<ExternalChange> {
    match storage::read_bounded(
        &storage::RealStorage,
        storage::Operation::ValidateDocumentRevision,
        path,
        document::MAX_DOCUMENT_BYTES,
    ) {
        Ok(current) if current == expected => None,
        Ok(_) => Some(ExternalChange::Modified),
        Err(failure) if failure.error_kind == std::io::ErrorKind::NotFound => {
            Some(ExternalChange::Missing)
        }
        Err(_) => Some(ExternalChange::Unreadable),
    }
}

fn should_reuse_untitled_window(
    dirty: bool,
    has_path: bool,
    rich_text_preview: bool,
    empty: bool,
) -> bool {
    !dirty && !has_path && !rich_text_preview && empty
}

fn can_begin_print(
    file_busy: bool,
    print_busy: bool,
    recovery_loading: bool,
    alert_open: bool,
    rich_text_preview: bool,
) -> bool {
    !file_busy && !print_busy && !recovery_loading && !alert_open && !rich_text_preview
}

fn recovery_failure_message() -> SharedString {
    "Text Editor could not safely update its private recovery data. The current buffer remains open; save the document before closing."
        .into()
}

impl EditorView {
    fn new_with_path(
        initial_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = rmac_editor::multiline("", window, cx);
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
        let (document_events, document_event_rx) = async_channel::bounded(4);
        let document_watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let event = match result {
                    Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => return,
                    Ok(_) => DocumentWatchEvent::Changed,
                    Err(_) => DocumentWatchEvent::Unavailable,
                };
                let _ = document_events.try_send(event);
            })
            .ok();

        // Dirty tracking + live match refresh + autosave on every edit.
        let sub_main = cx.subscribe(&input, |this, _input, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.on_buffer_changed(cx);
            }
        });
        // Re-render the parent whenever the buffer notifies — InputEvent has no
        // cursor-move variant, but `observe` fires on every `notify()` the input
        // makes (including caret movement), keeping the line:col status live.
        cx.observe(&input, |_, _, cx| cx.notify()).detach();

        // Live match recompute as the query is edited.
        let sub_find = cx.subscribe(&find_input, |this, _input, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.current = 0;
                this.recompute_matches(cx);
                cx.notify();
            }
        });

        cx.bind_keys([
            KeyBinding::new("cmd-n", NewFile, Some(CTX)),
            KeyBinding::new("cmd-o", OpenFile, Some(CTX)),
            KeyBinding::new("cmd-s", SaveFile, Some(CTX)),
            KeyBinding::new("cmd-shift-s", SaveFileAs, Some(CTX)),
            KeyBinding::new("cmd-f", ToggleFind, Some(CTX)),
            KeyBinding::new("cmd-shift-f", ToggleReplace, Some(CTX)),
            KeyBinding::new("cmd-g", FindNext, Some(CTX)),
            KeyBinding::new("cmd-shift-g", FindPrev, Some(CTX)),
            KeyBinding::new("escape", CloseBar, Some(CTX)),
            KeyBinding::new("cmd-=", IncreaseFont, Some(CTX)),
            KeyBinding::new("cmd-+", IncreaseFont, Some(CTX)),
            KeyBinding::new("cmd--", DecreaseFont, Some(CTX)),
            KeyBinding::new("cmd-shift-m", ToggleMono, Some(CTX)),
            KeyBinding::new("cmd-w", CloseWindow, Some(CTX)),
        ]);
        #[cfg(target_os = "linux")]
        cx.bind_keys([KeyBinding::new("cmd-p", PrintFile, Some(CTX))]);

        // Recovery discovery can inspect bounded records totaling up to 128
        // MiB. Present the first frame immediately and keep the document gated
        // until the background result establishes this window's recovery
        // identity and any required Restore/Discard decision.
        cx.spawn_in(window, async move |this, cx| {
            let recovery = cx
                .background_executor()
                .spawn(async { startup_recovery() })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.recovery_directory = recovery.directory;
                this.recovery_path = recovery.active_path;
                this.recovery_cleanup_paths = recovery.cleanup_paths;
                this.recovery_loading = false;
                this.recovery_error = recovery.warning.then(recovery_failure_message);
                this.alert = recovery.prompt.map(ActiveAlert::Recover);
                if this.alert.is_none() {
                    if let Some(path) = this.pending_startup_path.take() {
                        this.load_document_path(path, "Failed to open the file.", window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();

        // Native events are hints only. Coalesce bursts from atomic rename and
        // metadata activity, then compare the complete bounded file bytes with
        // the exact revision retained at open/last-save.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = document_event_rx.recv().await {
                if matches!(event, DocumentWatchEvent::Unavailable) {
                    if this
                        .update(cx, |this, cx| {
                            this.document_watch_warning = this.path.is_some();
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let mut unavailable = false;
                while let Ok(event) = document_event_rx.try_recv() {
                    unavailable |= matches!(event, DocumentWatchEvent::Unavailable);
                }
                if unavailable
                    && this
                        .update(cx, |this, cx| {
                            this.document_watch_warning = this.path.is_some();
                            cx.notify();
                        })
                        .is_err()
                {
                    break;
                }
                let snapshot = this
                    .update(cx, |this, _| {
                        this.path
                            .clone()
                            .zip(this.saved_bytes.clone())
                            .map(|(path, expected)| (path, expected, this.document_generation))
                    })
                    .ok()
                    .flatten();
                let Some((path, expected, generation)) = snapshot else {
                    continue;
                };
                let checked_path = path.clone();
                let state = cx
                    .background_executor()
                    .spawn(async move { inspect_external_revision(&checked_path, &expected) })
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if this.document_generation == generation
                            && this.path.as_deref() == Some(path.as_path())
                        {
                            this.external_change = state;
                            if !unavailable {
                                this.document_watch_warning = false;
                            }
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let recovery_directory = std::env::temp_dir().join("rmac-text-editor-recovery-pending");
        let recovery_path = recovery::fresh_record_path(&recovery_directory);

        Self {
            alert: None,
            input,
            path: None,
            saved_bytes: None,
            text_format: document::TextFormat::default(),
            saved_format: document::TextFormat::default(),
            saved_value: String::new(),
            dirty: false,
            file_busy: false,
            print_busy: false,
            find_open: false,
            replace_mode: false,
            find_input,
            replace_input,
            matches: Vec::new(),
            current: 0,
            mono: false,
            font_size: 15.0,
            rtf_runs: None,
            focus: cx.focus_handle(),
            recovery_directory,
            recovery_path,
            recovery_cleanup_paths: Vec::new(),
            recovery_clock: RecoveryClock::default(),
            recovery_loading: true,
            recovery_error: None,
            status_notice: None,
            window_generation: NEXT_WINDOW_GENERATION
                .fetch_add(1, Ordering::Relaxed)
                .max(1),
            document_generation: 0,
            current_document_generation: Arc::new(AtomicU64::new(0)),
            external_change: None,
            document_watch_warning: false,
            watched_directory: None,
            document_watcher,
            pending_startup_path: initial_path,
            _subscriptions: vec![sub_main, sub_find],
        }
    }

    fn filename(&self) -> SharedString {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".to_string())
                .into(),
            None => "Untitled".into(),
        }
    }

    fn file_action_blocked(&self) -> bool {
        self.recovery_loading || self.alert.is_some() || self.print_busy
    }

    fn advance_document_generation(&mut self) {
        self.document_generation = self.document_generation.wrapping_add(1);
        self.current_document_generation
            .store(self.document_generation, Ordering::Release);
    }

    fn reset_document_watch(&mut self) {
        self.advance_document_generation();
        self.external_change = None;
        let next_directory = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        if self.watched_directory != next_directory {
            if let (Some(watcher), Some(directory)) = (
                self.document_watcher.as_mut(),
                self.watched_directory.take(),
            ) {
                let _ = watcher.unwatch(&directory);
            }
            if let (Some(watcher), Some(directory)) =
                (self.document_watcher.as_mut(), next_directory.as_ref())
            {
                if watcher
                    .watch(directory, notify::RecursiveMode::NonRecursive)
                    .is_ok()
                {
                    self.watched_directory = Some(directory.clone());
                }
            }
        }
        self.document_watch_warning =
            self.path.is_some() && self.watched_directory != next_directory;
    }

    // ── Dirty + autosave ────────────────────────────────────────────────

    fn on_buffer_changed(&mut self, cx: &mut Context<Self>) {
        self.advance_document_generation();
        if self.find_open {
            self.recompute_matches(cx);
        }
        self.refresh_dirty_state(cx);
    }

    fn refresh_dirty_state(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        self.dirty = document::has_unsaved_changes(
            &value,
            &self.saved_value,
            self.text_format,
            self.saved_format,
        );
        if self.dirty {
            self.schedule_autosave(cx);
        } else {
            self.clear_recovery(cx);
        }
        cx.notify();
    }

    /// Debounced autosave: each edit bumps a generation token and arms a timer;
    /// only the most recent timer actually writes the recovery file.
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        let generation = self.recovery_clock.arm();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let Ok(Some((path, record, cleanup_paths))) = this.update(cx, |this, cx| {
                this.recovery_clock
                    .should_write(generation, this.dirty)
                    .then(|| {
                        let content = this.input.read(cx).value().to_string();
                        (
                            this.recovery_path.clone(),
                            recovery::RecoveryRecord::for_document(
                                this.path.as_deref(),
                                this.text_format,
                                content,
                            ),
                            this.recovery_cleanup_paths.clone(),
                        )
                    })
            }) else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move {
                        recovery::save(&storage::RealStorage, &path, &record)?;
                        storage::remove_recovery_paths(&storage::RealStorage, &cleanup_paths)
                    }
                })
                .await;
            let stale = this
                .update(cx, |this, cx| {
                    let current = this.recovery_clock.is_current(generation)
                        && this.dirty
                        && this.recovery_path == path;
                    if current {
                        match result {
                            Ok(()) => {
                                this.recovery_cleanup_paths.clear();
                                this.recovery_error = None;
                            }
                            Err(failure) => this.record_recovery_failure(failure, cx),
                        }
                    }
                    !current
                })
                .unwrap_or(false);
            if stale {
                let _ = cx
                    .background_executor()
                    .spawn(async move { storage::remove_recovery(&storage::RealStorage, &path) })
                    .await;
            }
        })
        .detach();
    }

    fn record_recovery_failure(&mut self, _failure: storage::Failure, cx: &mut Context<Self>) {
        self.recovery_error = Some(recovery_failure_message());
        cx.notify();
    }

    fn clear_recovery(&mut self, cx: &mut Context<Self>) -> bool {
        self.recovery_clock.invalidate();
        let mut paths = vec![self.recovery_path.clone()];
        paths.extend(self.recovery_cleanup_paths.iter().cloned());
        match storage::remove_recovery_paths(&storage::RealStorage, &paths) {
            Ok(()) => {
                self.recovery_cleanup_paths.clear();
                self.recovery_path = recovery::fresh_record_path(&self.recovery_directory);
                self.recovery_error = None;
                true
            }
            Err(failure) => {
                self.record_recovery_failure(failure, cx);
                false
            }
        }
    }

    fn mark_clean(&mut self, value: String, cx: &mut Context<Self>) -> bool {
        self.saved_value = value;
        self.saved_format = self.text_format;
        self.dirty = false;
        self.clear_recovery(cx)
    }

    fn record_current_document(&self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move {
                        rmac_recent_documents::Store::from_environment()
                            .and_then(|store| store.record(&path))
                    }
                })
                .await;
            if result.is_err() {
                let _ = this.update(cx, |this, cx| {
                    if this.path.as_deref() == Some(path.as_path()) {
                        this.status_notice =
                            Some("The document is open, but Recents could not be updated.".into());
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    // ── File operations ─────────────────────────────────────────────────

    fn new_file(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        if open_editor_window(cx, None).is_err() {
            self.alert = Some(ActiveAlert::Error {
                title: "Could not open a new document window.",
                message: "Text Editor could not create another window. This document remains open."
                    .into(),
            });
            cx.notify();
        }
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        self.do_open(window, cx);
    }

    /// Leave the read-only RTF preview and continue editing the extracted text
    /// as a new untitled plain-text document — the original `.rtf` is never
    /// overwritten.
    fn edit_as_plain_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.rtf_runs.take().is_some() {
            self.path = None;
            self.saved_bytes = None;
            self.text_format = document::TextFormat::default();
            self.reset_document_watch();
            self.dirty = true; // an unsaved derived document
            self.schedule_autosave(cx);
            cx.notify();
        }
    }

    fn do_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        self.file_busy = true;
        let reuse_current = should_reuse_untitled_window(
            self.dirty,
            self.path.is_some(),
            self.rtf_runs.is_some(),
            self.input.read(cx).value().is_empty(),
        );
        cx.notify();
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let picker = rx.await;
            let Ok(Ok(Some(paths))) = picker else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.file_busy = false;
                    if !matches!(picker, Ok(Ok(None))) {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not open the file chooser.",
                            message: "The desktop file chooser is temporarily unavailable.".into(),
                        });
                    }
                    cx.notify();
                });
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_busy = false;
                let mut paths = paths.into_iter();
                if reuse_current {
                    if let Some(path) = paths.next() {
                        this.load_document_path(path, "Failed to open the file.", window, cx);
                    }
                }
                let mut failed_windows = 0_usize;
                for path in paths {
                    if open_editor_window(cx, Some(path)).is_err() {
                        failed_windows += 1;
                    }
                }
                if failed_windows > 0 {
                    this.status_notice = Some(
                        format!(
                            "Text Editor could not create {} selected document {}.",
                            failed_windows,
                            if failed_windows == 1 {
                                "window"
                            } else {
                                "windows"
                            }
                        )
                        .into(),
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_document_path(
        &mut self,
        path: PathBuf,
        error_title: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy {
            return;
        }
        self.file_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { load_selected_document(&path) }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_busy = false;
                match loaded {
                    Ok(LoadedFile::Plain(document)) => {
                        this.input.update(cx, |state, cx| {
                            state.set_value(document.text.clone(), window, cx)
                        });
                        this.path = Some(path);
                        this.saved_bytes = Some(document.original_bytes);
                        this.text_format = document.format;
                        this.rtf_runs = None;
                        this.reset_document_watch();
                        this.mark_clean(document.text, cx);
                        this.record_current_document(cx);
                    }
                    Ok(LoadedFile::RichText { text, runs }) => {
                        this.input
                            .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
                        this.path = Some(path);
                        this.saved_bytes = None;
                        this.text_format = document::TextFormat::default();
                        this.rtf_runs = Some(runs);
                        this.reset_document_watch();
                        this.mark_clean(text, cx);
                        this.record_current_document(cx);
                    }
                    Err(message) => {
                        this.alert = Some(ActiveAlert::Error {
                            title: error_title,
                            message,
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        self.save_with(None, window, cx);
    }

    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked() {
            return;
        }
        let content = self.input.read(cx).value().to_string();
        self.save_to_new_path(content, self.text_format, None, None, window, cx);
    }

    fn print_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !can_begin_print(
            self.file_busy,
            self.print_busy,
            self.recovery_loading,
            self.alert.is_some(),
            false,
        ) {
            return;
        }
        if self.rtf_runs.is_some() {
            self.alert = Some(ActiveAlert::Error {
                title: "Could not print the document.",
                message: "Printing the formatted RTF preview is not supported yet. Continue as plain text to print without implying the original formatting is preserved."
                    .into(),
            });
            cx.notify();
            return;
        }
        #[cfg(target_os = "linux")]
        {
            let raw_window = raw_window_handle::HasWindowHandle::window_handle(window)
                .map(|handle| handle.as_raw());
            let raw_display = raw_window_handle::HasDisplayHandle::display_handle(window)
                .map(|handle| handle.as_raw());
            let (raw_window, raw_display) = match (raw_window, raw_display) {
                (Ok(raw_window), Ok(raw_display)) => (raw_window, raw_display),
                (Err(_), _) | (_, Err(_)) => {
                    self.alert = Some(ActiveAlert::Error {
                        title: "Could not open the print dialog.",
                        message:
                            "Printing requires the current exported Wayland application window."
                                .into(),
                    });
                    cx.notify();
                    return;
                }
            };
            let request = rmac_print_linux::PrintDocument {
                window: raw_window,
                display: raw_display,
                window_generation: self.window_generation,
                document_generation: self.document_generation,
                current_document_generation: self.current_document_generation.clone(),
                title: self.filename().to_string(),
                text: self.input.read(cx).value().to_string(),
            };
            self.print_busy = true;
            self.status_notice = None;
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let result = rmac_print_linux::print_document(request).await;
                let _ = this.update_in(cx, |this, _, cx| {
                    this.print_busy = false;
                    match result {
                        Ok(rmac_print_linux::Outcome::Printed) => {
                            this.status_notice =
                                Some("The desktop print service accepted the document.".into());
                        }
                        Ok(rmac_print_linux::Outcome::Cancelled) => {}
                        Err(error) => {
                            this.alert = Some(ActiveAlert::Error {
                                title: "Could not print the document.",
                                message: error.to_string(),
                            });
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = window;
            let _ = (self.window_generation, &self.current_document_generation);
            self.alert = Some(ActiveAlert::Error {
                title: "Printing is unavailable.",
                message: "Printing is implemented for the supported Linux session.".into(),
            });
            cx.notify();
        }
    }

    /// Save the buffer; if `then` is set, run that pending action only **after**
    /// the save has actually succeeded (important for the async Save-As path so
    /// the destructive action never runs before the file is written).
    fn save_with(&mut self, then: Option<Pending>, window: &mut Window, cx: &mut Context<Self>) {
        // The RTF preview is read-only — never write plain text over the .rtf.
        if self.rtf_runs.is_some() {
            return;
        }
        if self.file_busy {
            return;
        }
        if !self.dirty && self.path.is_some() {
            if let Some(pending) = then {
                self.perform(pending, window, cx);
            }
            return;
        }
        let content = self.input.read(cx).value().to_string();
        if let Some(path) = self.path.clone() {
            let Some(expected) = self.saved_bytes.clone() else {
                self.alert = Some(ActiveAlert::Error {
                    title: "Failed to save the file.",
                    message: "Text Editor could not validate the opened document revision. Save a copy instead."
                        .into(),
                });
                cx.notify();
                return;
            };
            let format = self.text_format;
            self.file_busy = true;
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let save_content = content.clone();
                let result = cx
                    .background_executor()
                    .spawn(
                        async move { save_document(&path, Some(&expected), &save_content, format) },
                    )
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.finish_document_save(result, content, then, window, cx);
                });
            })
            .detach();
            return;
        }
        self.save_to_new_path(content, self.text_format, then, None, window, cx);
    }

    fn save_to_new_path(
        &mut self,
        content: String,
        format: document::TextFormat,
        then: Option<Pending>,
        forbidden_destination: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let suggested_name = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled.txt");
        self.file_busy = true;
        cx.notify();
        let rx = cx.prompt_for_new_path(&dir, Some(suggested_name));
        cx.spawn_in(window, async move |this, cx| {
            // Save-As was cancelled or failed: do NOT run the pending action,
            // so unsaved changes are preserved instead of silently discarded.
            let picker = rx.await;
            let Ok(Ok(Some(path))) = picker else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.file_busy = false;
                    if !matches!(picker, Ok(Ok(None))) {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not open the save dialog.",
                            message: "The desktop file chooser is temporarily unavailable.".into(),
                        });
                    }
                    cx.notify();
                });
                return;
            };
            let result = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    let content = content.clone();
                    async move {
                        save_document_copy(
                            &path,
                            forbidden_destination.as_deref(),
                            &content,
                            format,
                        )
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if result.is_ok() {
                    this.path = Some(path);
                }
                this.finish_document_save(result, content, then, window, cx);
            });
        })
        .detach();
    }

    fn finish_document_save(
        &mut self,
        result: Result<document::DecodedDocument, SaveFailure>,
        requested_text: String,
        then: Option<Pending>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_busy = false;
        match result {
            Ok(saved) => {
                if saved.text != requested_text {
                    self.input.update(cx, |state, cx| {
                        state.set_value(saved.text.clone(), window, cx)
                    });
                }
                self.saved_bytes = Some(saved.original_bytes);
                self.text_format = saved.format;
                self.reset_document_watch();
                let recovery_cleared = self.mark_clean(saved.text, cx);
                self.record_current_document(cx);
                if recovery_cleared {
                    if let Some(pending) = then {
                        self.perform(pending, window, cx);
                    }
                }
            }
            Err(error) => {
                if matches!(
                    &error,
                    SaveFailure::Storage(storage::SaveDocumentError::Conflict)
                ) {
                    self.external_change = Some(ExternalChange::Modified);
                }
                self.alert = Some(
                    if matches!(
                        &error,
                        SaveFailure::Storage(storage::SaveDocumentError::Conflict)
                    ) {
                        ActiveAlert::Conflict
                    } else {
                        ActiveAlert::Error {
                            title: "Failed to save the file.",
                            message: error.to_string(),
                        }
                    },
                );
            }
        }
        cx.notify();
    }

    fn show_external_conflict(&mut self, cx: &mut Context<Self>) {
        if !self.file_busy && !self.file_action_blocked() && self.external_change.is_some() {
            self.alert = Some(ActiveAlert::Conflict);
            cx.notify();
        }
    }

    fn reload_conflicting_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.alert = None;
            return;
        };
        self.alert = None;
        self.file_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { load_selected_document(&path) }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.file_busy = false;
                match loaded {
                    Ok(LoadedFile::Plain(document)) => {
                        this.input.update(cx, |state, cx| {
                            state.set_value(document.text.clone(), window, cx)
                        });
                        this.saved_bytes = Some(document.original_bytes);
                        this.text_format = document.format;
                        this.rtf_runs = None;
                        this.reset_document_watch();
                        this.mark_clean(document.text, cx);
                    }
                    Ok(LoadedFile::RichText { text, runs }) => {
                        this.input
                            .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
                        this.saved_bytes = None;
                        this.text_format = document::TextFormat::default();
                        this.rtf_runs = Some(runs);
                        this.reset_document_watch();
                        this.mark_clean(text, cx);
                    }
                    Err(message) => {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not reload the document.",
                            message,
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_conflicting_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.rtf_runs.is_some() {
            return;
        }
        self.alert = None;
        let content = self.input.read(cx).value().to_string();
        self.save_to_new_path(
            content,
            self.text_format,
            None,
            self.path.clone(),
            window,
            cx,
        );
    }

    fn review_conflict_overwrite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.alert = None;
            return;
        };
        self.alert = None;
        self.file_busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let reviewed = cx
                .background_executor()
                .spawn(async move {
                    storage::read_bounded(
                        &storage::RealStorage,
                        storage::Operation::ValidateDocumentRevision,
                        &path,
                        document::MAX_DOCUMENT_BYTES,
                    )
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.file_busy = false;
                this.alert = Some(match reviewed {
                    Ok(reviewed_revision) => ActiveAlert::ConfirmOverwrite { reviewed_revision },
                    Err(_) => ActiveAlert::Error {
                        title: "Could not review the external document.",
                        message: "The document is missing, inaccessible, or no longer within Text Editor’s safety limit. Your local buffer remains open; save a copy instead."
                            .into(),
                    },
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn overwrite_conflicting_document(
        &mut self,
        reviewed_revision: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy || self.rtf_runs.is_some() {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.alert = None;
            return;
        };
        self.alert = None;
        self.file_busy = true;
        let content = self.input.read(cx).value().to_string();
        let format = self.text_format;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let save_content = content.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    save_document(&path, Some(&reviewed_revision), &save_content, format)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_document_save(result, content, None, window, cx);
            });
        })
        .detach();
    }

    /// If the buffer is dirty, ask before discarding; otherwise act immediately.
    fn guarded(&mut self, pending: Pending, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        if !self.dirty {
            if self.clear_recovery(cx) {
                self.perform(pending, window, cx);
            }
            return;
        }
        self.alert = Some(ActiveAlert::ConfirmSave(pending));
        cx.notify();
    }

    /// Primary (default) button of the active alert: Restore / Save / OK.
    fn alert_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.alert.take() {
            Some(ActiveAlert::Recover(prompt)) => {
                self.text_format = prompt.format;
                self.saved_format = prompt.format;
                self.path = None;
                self.saved_bytes = None;
                self.input
                    .update(cx, |state, cx| state.set_value(prompt.content, window, cx));
                if prompt.additional_drafts > 0 {
                    self.status_notice = Some(
                        format!(
                            "{} additional recovered {} remain available on the next launch.",
                            prompt.additional_drafts,
                            if prompt.additional_drafts == 1 {
                                "draft"
                            } else {
                                "drafts"
                            }
                        )
                        .into(),
                    );
                }
                // Recovered text is unsaved relative to the empty baseline, so
                // this marks the buffer dirty and re-arms autosave.
                self.on_buffer_changed(cx);
                if let Some(path) = self.pending_startup_path.take() {
                    if open_editor_window(cx, Some(path)).is_err() {
                        self.status_notice = Some(
                            "The recovered draft is safe, but Text Editor could not open the requested document window."
                                .into(),
                        );
                    }
                }
            }
            Some(ActiveAlert::ConfirmSave(pending)) => self.save_with(Some(pending), window, cx),
            Some(ActiveAlert::Conflict) => self.save_conflicting_copy(window, cx),
            Some(ActiveAlert::ConfirmOverwrite { reviewed_revision }) => {
                self.overwrite_conflicting_document(reviewed_revision, window, cx);
            }
            Some(ActiveAlert::Error { .. }) | None => {}
        }
        cx.notify();
    }

    /// Secondary button: Discard (recover) / Don't Save (confirm).
    fn alert_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.alert.take() {
            Some(ActiveAlert::Recover(prompt)) => {
                if !self.clear_recovery(cx) {
                    self.alert = Some(ActiveAlert::Recover(prompt));
                } else if let Some(path) = self.pending_startup_path.take() {
                    self.load_document_path(path, "Failed to open the file.", window, cx);
                }
            }
            Some(ActiveAlert::ConfirmSave(pending)) => {
                if self.clear_recovery(cx) {
                    self.perform(pending, window, cx);
                } else {
                    self.alert = Some(ActiveAlert::ConfirmSave(pending));
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Cancel / dismiss the alert without acting.
    fn alert_cancel(&mut self, cx: &mut Context<Self>) {
        self.alert = None;
        cx.notify();
    }

    fn perform(&mut self, pending: Pending, window: &mut Window, _cx: &mut Context<Self>) {
        match pending {
            Pending::Close => window.remove_window(),
        }
    }

    // ── Find / replace ──────────────────────────────────────────────────

    fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open && !self.replace_mode {
            self.close_bar(cx);
        } else {
            self.find_open = true;
            self.replace_mode = false;
            self.current = 0;
            self.recompute_matches(cx);
            self.find_input.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
        }
    }

    fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open && self.replace_mode {
            self.close_bar(cx);
        } else {
            self.find_open = true;
            self.replace_mode = true;
            self.current = 0;
            self.recompute_matches(cx);
            self.find_input.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
        }
    }

    fn close_bar(&mut self, cx: &mut Context<Self>) {
        self.find_open = false;
        self.replace_mode = false;
        cx.notify();
    }

    /// Case-sensitive scan of the buffer for the current query, recording the
    /// byte offset of every match.
    fn recompute_matches(&mut self, cx: &Context<Self>) {
        let needle = self.find_input.read(cx).value().to_string();
        let hay = self.input.read(cx).value().to_string();
        let mut v = Vec::new();
        if !needle.is_empty() {
            let mut start = 0;
            while let Some(pos) = hay[start..].find(&needle) {
                let abs = start + pos;
                v.push(abs);
                start = abs + needle.len();
            }
        }
        if self.current >= v.len() {
            self.current = 0;
        }
        self.matches = v;
    }

    fn scroll_to_current(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(&off) = self.matches.get(self.current) {
            let pos: Position = self.input.read(cx).text().offset_to_position(off);
            self.input
                .update(cx, |s, cx| s.set_cursor_position(pos, window, cx));
        }
    }

    fn find_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.scroll_to_current(window, cx);
        cx.notify();
    }

    fn find_prev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        self.current = (self.current + n - 1) % n;
        self.scroll_to_current(window, cx);
        cx.notify();
    }

    fn replace_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy {
            return;
        }
        self.recompute_matches(cx);
        if self.matches.is_empty() {
            return;
        }
        let off = self.matches[self.current];
        let needle = self.find_input.read(cx).value().to_string();
        let repl = self.replace_input.read(cx).value().to_string();
        let mut hay = self.input.read(cx).value().to_string();
        if off + needle.len() <= hay.len() && &hay[off..off + needle.len()] == needle.as_str() {
            hay.replace_range(off..off + needle.len(), &repl);
            self.input.update(cx, |s, cx| s.set_value(hay, window, cx));
            self.on_buffer_changed(cx);
            self.scroll_to_current(window, cx);
            cx.notify();
        }
    }

    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy {
            return;
        }
        let needle = self.find_input.read(cx).value().to_string();
        if needle.is_empty() {
            return;
        }
        let repl = self.replace_input.read(cx).value().to_string();
        let hay = self.input.read(cx).value().to_string();
        if !hay.contains(&needle) {
            return;
        }
        let newv = hay.replace(&needle, &repl);
        self.input.update(cx, |s, cx| s.set_value(newv, window, cx));
        self.current = 0;
        self.on_buffer_changed(cx);
        cx.notify();
    }

    // ── Format ──────────────────────────────────────────────────────────

    fn toggle_mono(&mut self, cx: &mut Context<Self>) {
        self.mono = !self.mono;
        cx.notify();
    }

    fn increase_font(&mut self, cx: &mut Context<Self>) {
        self.font_size = (self.font_size + 1.0).min(48.0);
        cx.notify();
    }

    fn decrease_font(&mut self, cx: &mut Context<Self>) {
        self.font_size = (self.font_size - 1.0).max(9.0);
        cx.notify();
    }

    fn set_encoding(&mut self, encoding: document::TextEncoding, cx: &mut Context<Self>) {
        if self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked() {
            return;
        }
        if self.text_format.encoding != encoding {
            self.text_format.encoding = encoding;
            self.refresh_dirty_state(cx);
        }
    }

    fn set_line_ending(&mut self, line_ending: document::LineEnding, cx: &mut Context<Self>) {
        if self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked() {
            return;
        }
        if !matches!(
            line_ending,
            document::LineEnding::Lf | document::LineEnding::CrLf | document::LineEnding::Cr
        ) {
            return;
        }
        if self.text_format.save_line_ending != line_ending {
            self.text_format.save_line_ending = line_ending;
            self.refresh_dirty_state(cx);
        }
    }

    // ── Rendering ───────────────────────────────────────────────────────

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.filename();
        let dirty = self.dirty;
        let row = div()
            .size_full()
            .flex()
            .items_center()
            .px_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("new", "")
                            .icon(Icon::new(IconName::File).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(self.file_busy || self.file_action_blocked())
                            .tooltip("New Window")
                            .on_click(cx.listener(|this, _, window, cx| this.new_file(window, cx))),
                    )
                    .child(
                        Button::new("open", "")
                            .icon(Icon::new(IconName::FolderOpen).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(self.file_busy || self.file_action_blocked())
                            .tooltip("Open")
                            .on_click(cx.listener(|this, _, window, cx| this.open(window, cx))),
                    )
                    .child(
                        Button::new("find", "")
                            .icon(Icon::new(IconName::Search).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .selected(self.find_open)
                            .tooltip("Find")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_find(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(mac::MEDIUM)
                    .text_color(mac::text())
                    .child(title)
                    .when(dirty, |d| {
                        d.child(
                            div()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(mac::text_secondary())
                                .child("— Edited"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("mono", "Mono")
                            .ghost()
                            .with_size(Size::Small)
                            .selected(self.mono)
                            .tooltip("Monospace font")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_mono(cx))),
                    )
                    .child(
                        Button::new("font-dec", "")
                            .icon(Icon::new(IconName::Minus).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Small)
                            .tooltip("Smaller text")
                            .on_click(cx.listener(|this, _, _, cx| this.decrease_font(cx))),
                    )
                    .child(
                        Button::new("font-inc", "")
                            .icon(Icon::new(IconName::Plus).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Small)
                            .tooltip("Larger text")
                            .on_click(cx.listener(|this, _, _, cx| this.increase_font(cx))),
                    )
                    .when(cfg!(target_os = "linux"), |actions| {
                        actions.child(
                            Button::new("print", "Print…")
                                .ghost()
                                .with_size(Size::Small)
                                .busy(self.print_busy)
                                .disabled(!can_begin_print(
                                    self.file_busy,
                                    self.print_busy,
                                    self.recovery_loading,
                                    self.alert.is_some(),
                                    self.rtf_runs.is_some(),
                                ))
                                .tooltip("Print Document")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.print_document(window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("save", "Save")
                            .primary()
                            .with_size(Size::Small)
                            .busy(self.file_busy)
                            .disabled(
                                self.file_busy
                                    || self.rtf_runs.is_some()
                                    || self.file_action_blocked(),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            );
        rmac_ui::toolbar(row)
    }

    fn render_find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query_empty = self.find_input.read(cx).value().is_empty();
        let status: SharedString = if query_empty {
            "".into()
        } else if self.matches.is_empty() {
            "Not found".into()
        } else {
            format!("{} of {}", self.current + 1, self.matches.len()).into()
        };

        let find_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(220.0))
                    .child(SearchField::new(&self.find_input).appearance(true)),
            )
            .child(
                Button::new("find-prev", "")
                    .icon(Icon::new(IconName::ChevronUp).text_color(mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("Previous match")
                    .on_click(cx.listener(|this, _, window, cx| this.find_prev(window, cx))),
            )
            .child(
                Button::new("find-next", "")
                    .icon(Icon::new(IconName::ChevronDown).text_color(mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("Next match")
                    .on_click(cx.listener(|this, _, window, cx| this.find_next(window, cx))),
            )
            .child(
                div()
                    .min_w(px(64.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(status),
            )
            .child(div().flex_1())
            .child(
                Button::new("find-close", "")
                    .icon(Icon::new(IconName::Close).text_color(mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("Done")
                    .on_click(cx.listener(|this, _, _, cx| this.close_bar(cx))),
            );

        let mut col = div()
            .v_flex()
            .gap_2()
            .w_full()
            .px(px(48.0))
            .py(px(8.0))
            .bg(mac::chrome())
            .border_b_1()
            .border_color(mac::separator())
            .child(find_row);

        if self.replace_mode {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(220.0))
                            .child(TextField::new(&self.replace_input).appearance(true)),
                    )
                    .child(
                        Button::new("replace-one", "Replace")
                            .ghost()
                            .with_size(Size::Small)
                            .disabled(self.print_busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.replace_current(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("replace-all", "Replace All")
                            .ghost()
                            .with_size(Size::Small)
                            .disabled(self.print_busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.replace_all(window, cx)),
                            ),
                    ),
            );
        }

        col
    }

    /// The read-only formatted RTF preview: a banner plus styled text built from
    /// the parsed runs (weight / italic / underline / color preserved).
    fn render_rtf_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let base = if self.mono {
            rmac_ui::MONO_FONT
        } else {
            rmac_ui::UI_FONT
        };
        let size = self.font_size;
        let runs = self.rtf_runs.as_deref().unwrap_or(&[]);

        let mut text = String::new();
        let mut text_runs: Vec<TextRun> = Vec::new();
        for r in runs {
            if r.text.is_empty() {
                continue;
            }
            let family = r.family.clone().unwrap_or_else(|| base.to_string());
            let mut f = font(family);
            if r.bold {
                f = f.bold();
            }
            if r.italic {
                f = f.italic();
            }
            let color = r
                .color
                .map(|(rr, gg, bb)| {
                    gpui::rgb(((rr as u32) << 16) | ((gg as u32) << 8) | bb as u32).into()
                })
                .unwrap_or_else(mac::text);
            text_runs.push(TextRun {
                len: r.text.len(),
                font: f,
                color,
                background_color: None,
                underline: r.underline.then(|| UnderlineStyle {
                    thickness: px(1.0),
                    color: None,
                    wavy: false,
                }),
                strikethrough: None,
            });
            text.push_str(&r.text);
        }

        let banner = div()
            .flex_none()
            .h_flex()
            .items_center()
            .justify_between()
            .mb_4()
            .px_3()
            .py_2()
            .rounded(px(8.0))
            .bg(mac::chrome())
            .border_1()
            .border_color(mac::separator())
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child("Read-only RTF preview — formatting shown as in the document."),
            )
            .child(
                Button::new("edit-plain", "Edit as Plain Text")
                    .small()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.edit_as_plain_text(window, cx)),
                    ),
            );

        div()
            .id("rtf-preview")
            .flex_1()
            .overflow_y_scroll()
            .px(px(48.0))
            .py(px(20.0))
            .text_size(px(size))
            .line_height(px(size * 1.5))
            .child(banner)
            .child(StyledText::new(text).with_runs(text_runs))
    }

    /// Bottom status bar: live cursor line:column (1-based) on the left, and the
    /// document's word + character counts on the right — like a real editor.
    fn render_status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let st = self.input.read(cx);
        let pos = st.cursor_position();
        let value = st.value();
        let chars = value.chars().count();
        let words = value.split_whitespace().count();

        let cell = |s: String| {
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(mac::text_secondary())
                .child(s)
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(24.0))
            .px_3()
            .border_t_1()
            .border_color(mac::separator())
            .bg(mac::chrome())
            .child(cell(format!(
                "Ln {}, Col {}",
                pos.line + 1,
                pos.character + 1
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new(
                            "document-format",
                            self.text_format.status_against(self.saved_format),
                        )
                        .ghost()
                        .xsmall()
                        .disabled(
                            self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked(),
                        )
                        .tooltip("Text encoding and line endings")
                        .dropdown_menu({
                            let format = self.text_format;
                            move |menu, _, _| {
                                menu.menu_with_check(
                                    "UTF-8",
                                    format.encoding == document::TextEncoding::Utf8,
                                    Box::new(SetEncodingUtf8),
                                )
                                .menu_with_check(
                                    "UTF-8 with BOM",
                                    format.encoding == document::TextEncoding::Utf8Bom,
                                    Box::new(SetEncodingUtf8Bom),
                                )
                                .menu_with_check(
                                    "UTF-16 Little Endian",
                                    format.encoding == document::TextEncoding::Utf16Le,
                                    Box::new(SetEncodingUtf16Le),
                                )
                                .menu_with_check(
                                    "UTF-16 Big Endian",
                                    format.encoding == document::TextEncoding::Utf16Be,
                                    Box::new(SetEncodingUtf16Be),
                                )
                                .separator()
                                .menu_with_check(
                                    "Unix (LF)",
                                    format.save_line_ending == document::LineEnding::Lf,
                                    Box::new(SetLineEndingLf),
                                )
                                .menu_with_check(
                                    "Windows (CRLF)",
                                    format.save_line_ending == document::LineEnding::CrLf,
                                    Box::new(SetLineEndingCrLf),
                                )
                                .menu_with_check(
                                    "Classic Mac (CR)",
                                    format.save_line_ending == document::LineEnding::Cr,
                                    Box::new(SetLineEndingCr),
                                )
                            }
                        }),
                    )
                    .child(cell(format!(
                        "{} {}",
                        words,
                        if words == 1 { "word" } else { "words" }
                    )))
                    .child(cell(format!(
                        "{} {}",
                        chars,
                        if chars == 1 { "char" } else { "chars" }
                    ))),
            )
    }

    /// Build the shared modal alert for the current `ActiveAlert`.
    fn render_alert(&self, alert: ActiveAlert, cx: &mut Context<Self>) -> impl IntoElement {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};
        let (title, message, buttons): (&str, String, Vec<gpui::AnyElement>) = match alert {
            ActiveAlert::Recover(prompt) => (
                "Recover unsaved changes?",
                format!(
                    "An autosaved draft for “{}” was found.{}",
                    prompt.document_label,
                    if prompt.additional_drafts == 0 {
                        String::new()
                    } else {
                        format!(
                            " {} additional {} will remain available for a later launch.",
                            prompt.additional_drafts,
                            if prompt.additional_drafts == 1 {
                                "draft"
                            } else {
                                "drafts"
                            }
                        )
                    }
                ),
                vec![
                    rmac_ui::dialog_button("alert-discard", "Discard", Normal)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.alert_secondary(window, cx)),
                        )
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-restore", "Restore", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::ConfirmSave(_) => (
                "Do you want to save the changes you made?",
                "Your changes will be lost if you don't save them.".into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-dontsave", "Don't Save", Destructive)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.alert_secondary(window, cx)),
                        )
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-save", "Save", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::Conflict => (
                "The document changed in another application.",
                "Text Editor did not overwrite the external version. Reload discards this local buffer, Save a Copy preserves it at a new location, and Overwrite requires a fresh review plus another exact preflight."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-reload", "Discard & Reload", Destructive)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.reload_conflicting_document(window, cx);
                        }))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-save-copy", "Save a Copy…", Primary)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.save_conflicting_copy(window, cx);
                        }))
                        .into_any_element(),
                    rmac_ui::dialog_button(
                        "alert-review-overwrite",
                        "Overwrite Anyway…",
                        Destructive,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.review_conflict_overwrite(window, cx);
                    }))
                    .into_any_element(),
                ],
            ),
            ActiveAlert::ConfirmOverwrite { .. } => (
                "Overwrite the external document?",
                "Text Editor reread the complete external revision. Overwrite will run a second exact preflight and stop if the document changes again. This cannot preserve the external edits."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel-overwrite", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button(
                        "alert-confirm-overwrite",
                        "Overwrite Anyway",
                        Destructive,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.alert_confirm(window, cx);
                    }))
                    .into_any_element(),
                ],
            ),
            ActiveAlert::Error { title, message } => (
                title,
                message,
                vec![rmac_ui::dialog_button("alert-ok", "OK", Primary)
                    .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                    .into_any_element()],
            ),
        };
        rmac_ui::alert(title, message, buttons)
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let font_family = if self.mono {
            rmac_ui::MONO_FONT
        } else {
            rmac_ui::UI_FONT
        };
        let size = self.font_size;
        let recovery_loading = self.recovery_loading;
        let recovery_error = self.recovery_error.clone();
        let status_notice = self.status_notice.clone();
        let external_change = self.external_change;
        let document_watch_warning = self.document_watch_warning;

        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context(CTX)
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| this.save(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFileAs, window, cx| this.save_as(window, cx)))
            .on_action(
                cx.listener(|this, _: &PrintFile, window, cx| this.print_document(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFind, window, cx| this.toggle_find(window, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleReplace, window, cx| this.toggle_replace(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindNext, window, cx| this.find_next(window, cx)))
            .on_action(cx.listener(|this, _: &FindPrev, window, cx| this.find_prev(window, cx)))
            .on_action(cx.listener(|this, _: &CloseBar, _, cx| this.close_bar(cx)))
            .on_action(cx.listener(|this, _: &ToggleMono, _, cx| this.toggle_mono(cx)))
            .on_action(cx.listener(|this, _: &SetEncodingUtf8, _, cx| {
                this.set_encoding(document::TextEncoding::Utf8, cx)
            }))
            .on_action(cx.listener(|this, _: &SetEncodingUtf8Bom, _, cx| {
                this.set_encoding(document::TextEncoding::Utf8Bom, cx)
            }))
            .on_action(cx.listener(|this, _: &SetEncodingUtf16Le, _, cx| {
                this.set_encoding(document::TextEncoding::Utf16Le, cx)
            }))
            .on_action(cx.listener(|this, _: &SetEncodingUtf16Be, _, cx| {
                this.set_encoding(document::TextEncoding::Utf16Be, cx)
            }))
            .on_action(cx.listener(|this, _: &SetLineEndingLf, _, cx| {
                this.set_line_ending(document::LineEnding::Lf, cx)
            }))
            .on_action(cx.listener(|this, _: &SetLineEndingCrLf, _, cx| {
                this.set_line_ending(document::LineEnding::CrLf, cx)
            }))
            .on_action(cx.listener(|this, _: &SetLineEndingCr, _, cx| {
                this.set_line_ending(document::LineEnding::Cr, cx)
            }))
            .on_action(cx.listener(|this, _: &IncreaseFont, _, cx| this.increase_font(cx)))
            .on_action(cx.listener(|this, _: &DecreaseFont, _, cx| this.decrease_font(cx)))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                this.guarded(Pending::Close, window, cx)
            }))
            // The custom red traffic light dispatches RequestClose — route it
            // through the same unsaved-changes guard so closes aren't silent.
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.guarded(Pending::Close, window, cx)
            }))
            .bg(mac::window())
            .text_color(mac::text())
            .child(self.render_toolbar(cx))
            .when(recovery_loading, |editor| {
                editor.child(
                    div()
                        .id("recovery-loading")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px_3()
                        .bg(mac::chrome())
                        .border_b_1()
                        .border_color(mac::separator())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child("Checking for unsaved drafts…"),
                )
            })
            .when_some(recovery_error, |editor, message| {
                editor.child(
                    div()
                        .id("recovery-error")
                        .h(px(34.0))
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
                        .cursor_pointer()
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.recovery_error = None;
                            cx.notify();
                        })),
                )
            })
            .when_some(status_notice, |editor, message| {
                editor.child(
                    div()
                        .id("recovery-notice")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(mac::chrome())
                        .border_b_1()
                        .border_color(mac::separator())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .cursor_pointer()
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.status_notice = None;
                            cx.notify();
                        })),
                )
            })
            .when_some(external_change, |editor, change| {
                let message = match change {
                    ExternalChange::Modified => {
                        "This document changed outside Text Editor. Your buffer was not replaced."
                    }
                    ExternalChange::Missing => {
                        "This document was moved or deleted outside Text Editor. Your buffer remains open."
                    }
                    ExternalChange::Unreadable => {
                        "Text Editor can no longer verify the external document. Your buffer remains open."
                    }
                };
                editor.child(
                    div()
                        .id("external-change")
                        .h(px(40.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(mac::warning_background())
                        .border_b_1()
                        .border_color(mac::warning_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text())
                        .child(div().flex_1().child(message))
                        .child(
                            Button::new("external-change-review", "Review…")
                                .with_size(Size::Small)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_external_conflict(cx);
                                })),
                        ),
                )
            })
            .when(document_watch_warning && external_change.is_none(), |editor| {
                editor.child(
                    div()
                        .id("document-watch-warning")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px_3()
                        .bg(mac::chrome())
                        .border_b_1()
                        .border_color(mac::separator())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(
                            "Live document monitoring is unavailable. Saves still recheck the complete file before writing.",
                        ),
                )
            })
            .when(self.find_open, |d| d.child(self.render_find_bar(cx)))
            .child(if self.rtf_runs.is_some() {
                self.render_rtf_preview(cx).into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .px(px(48.0))
                    .py(px(20.0))
                    .font_family(font_family)
                    .text_size(px(size))
                    .line_height(px(size * 1.5))
                    .child(
                        TextField::new(&self.input)
                            .h_full()
                            .appearance(false)
                            .disabled(recovery_loading || self.print_busy),
                    )
                    .into_any_element()
            })
            .when(self.rtf_runs.is_none(), |d| {
                d.child(self.render_status_bar(cx))
            })
            .when_some(self.alert.clone(), |d, alert| {
                d.child(self.render_alert(alert, cx))
            })
    }
}

fn main() {
    let request = match parse_startup_request(std::env::args_os().skip(1)) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            if request.open_untitled && open_editor_window(cx, None).is_err() {
                eprintln!("Text Editor could not open a document window");
            }
            for path in request.paths {
                if open_editor_window(cx, Some(path)).is_err() {
                    eprintln!("Text Editor could not open a document window");
                }
            }
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::{
        can_begin_print, document, parse_startup_request, recovery_path_for_platform,
        same_file_identity, save_document_copy, should_reuse_untitled_window, RecoveryClock,
        SaveFailure, StartupRequest, MAX_STARTUP_DOCUMENTS,
    };
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn clean_transition_invalidates_a_pending_recovery_write() {
        let mut clock = RecoveryClock::default();
        let pending = clock.arm();

        assert!(clock.should_write(pending, true));
        assert!(!clock.should_write(pending, false));

        clock.invalidate();
        assert!(!clock.should_write(pending, true));
    }

    #[test]
    fn newer_edit_invalidates_an_older_recovery_write() {
        let mut clock = RecoveryClock::default();
        let older = clock.arm();
        let newer = clock.arm();

        assert!(!clock.should_write(older, true));
        assert!(clock.should_write(newer, true));
    }

    #[test]
    fn recovery_paths_follow_xdg_and_macos_conventions() {
        let linux_xdg = recovery_path_for_platform(
            false,
            Some(PathBuf::from("/var/state")),
            Some(PathBuf::from("/home/user")),
        )
        .unwrap();
        let linux_fallback = recovery_path_for_platform(
            false,
            Some(PathBuf::from("relative-state")),
            Some(PathBuf::from("/home/user")),
        )
        .unwrap();
        let macos = recovery_path_for_platform(
            true,
            Some(PathBuf::from("/ignored")),
            Some(PathBuf::from("/Users/user")),
        )
        .unwrap();

        assert_eq!(
            linux_xdg,
            PathBuf::from("/var/state/rmac-text-editor/recovery.txt")
        );
        assert_eq!(
            linux_fallback,
            PathBuf::from("/home/user/.local/state/rmac-text-editor/recovery.txt")
        );
        assert_eq!(
            macos,
            PathBuf::from("/Users/user/Library/Application Support/rmac-text-editor/recovery.txt")
        );
    }

    #[test]
    fn identical_copy_destination_is_rejected_even_before_it_exists() {
        let path = PathBuf::from("document.txt");
        assert!(same_file_identity(&path, &path));
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_copy_destination_is_the_same_file_identity() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-text-editor-copy-identity-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("source.txt");
        let link = directory.join("link.txt");
        std::fs::write(&source, b"external revision").unwrap();
        std::fs::hard_link(&source, &link).unwrap();

        assert!(same_file_identity(&source, &link));

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_copy_never_replaces_its_forbidden_source() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-text-editor-copy-protection-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("source.txt");
        std::fs::write(&source, b"external revision").unwrap();

        let error = save_document_copy(
            &source,
            Some(&source),
            "local buffer",
            document::TextFormat::default(),
        )
        .unwrap_err();

        assert!(matches!(error, SaveFailure::ConflictingCopyDestination));
        assert_eq!(std::fs::read(&source).unwrap(), b"external revision");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_reuses_only_a_clean_empty_untitled_window() {
        assert!(should_reuse_untitled_window(false, false, false, true));
        assert!(!should_reuse_untitled_window(true, false, false, false));
        assert!(!should_reuse_untitled_window(false, true, false, false));
        assert!(!should_reuse_untitled_window(false, false, true, false));
        assert!(!should_reuse_untitled_window(false, false, false, false));
    }

    #[test]
    fn desktop_launch_paths_open_independent_windows() {
        assert_eq!(
            parse_startup_request([
                OsString::from("/home/user/one.txt"),
                OsString::from("/home/user/two.rtf"),
            ])
            .unwrap(),
            StartupRequest {
                open_untitled: false,
                paths: vec![
                    PathBuf::from("/home/user/one.txt"),
                    PathBuf::from("/home/user/two.rtf"),
                ],
            }
        );
        assert_eq!(
            parse_startup_request([OsString::from("--new-document")]).unwrap(),
            StartupRequest {
                open_untitled: true,
                paths: Vec::new(),
            }
        );
    }

    #[test]
    fn startup_arguments_are_bounded_and_options_fail_closed() {
        assert!(parse_startup_request([OsString::from("--unknown")]).is_err());
        assert_eq!(
            parse_startup_request([OsString::from("--"), OsString::from("-literal-name.txt"),])
                .unwrap()
                .paths,
            vec![PathBuf::from("-literal-name.txt")]
        );
        assert!(parse_startup_request(
            (0..=MAX_STARTUP_DOCUMENTS).map(|index| OsString::from(format!("/tmp/{index}")))
        )
        .is_err());
    }

    #[test]
    fn printing_requires_one_stable_plain_text_window_state() {
        assert!(can_begin_print(false, false, false, false, false));
        assert!(!can_begin_print(true, false, false, false, false));
        assert!(!can_begin_print(false, true, false, false, false));
        assert!(!can_begin_print(false, false, true, false, false));
        assert!(!can_begin_print(false, false, false, true, false));
        assert!(!can_begin_print(false, false, false, false, true));
    }
}
