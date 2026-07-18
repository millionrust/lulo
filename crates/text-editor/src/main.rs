//! rmac Text Editor — a fast, native TextEdit-style editor.
//!
//! Rope-backed `InputState` body on a clean themed page, with a unified desktop
//! toolbar (New / Open / Save), a find/replace bar (⌘F / ⇧⌘F), dirty-state
//! tracking with a modified indicator and unsaved-changes prompts, basic
//! autosave to a recovery file, and a Format affordance (monospace + font
//! size). Shares the editing configuration with Notes via `rmac-editor`.

mod document;
mod rtf;
mod storage;

use std::{path::Path, path::PathBuf, time::Duration};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    actions, div, font, px, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement as _, Styled, StyledText, Subscription, TextRun, UnderlineStyle,
    Window,
};
use gpui_component::{Icon, IconName, Size, StyledExt as _};
use rmac_ui::{
    mac, Button, InputEvent, InputState, Position, RopeExt as _, SearchField, TextField,
};

const CTX: &str = "TextEditor";

actions!(
    text_editor,
    [
        NewFile,
        OpenFile,
        SaveFile,
        SaveFileAs,
        ToggleFind,
        ToggleReplace,
        FindNext,
        FindPrev,
        CloseBar,
        ToggleMono,
        IncreaseFont,
        DecreaseFont,
        CloseWindow,
    ]
);

/// A pending document switch that must wait on an unsaved-changes prompt.
#[derive(Clone, Copy)]
enum Pending {
    New,
    Open,
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
}

/// A modal alert awaiting the user, shown via the shared `rmac_ui::alert`.
#[derive(Clone)]
enum ActiveAlert {
    /// A recovery file was found — Restore (load it) or Discard.
    Recover(String),
    /// The buffer is dirty before `Pending` — Save / Don't Save / Cancel.
    ConfirmSave(Pending),
    /// The opened document no longer matches its retained exact revision.
    Conflict,
    /// A document open/save error — title + message + OK.
    Error {
        title: &'static str,
        message: String,
    },
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
}

impl std::fmt::Display for SaveFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
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
    /// Text as last saved (or opened/new) — the dirty baseline.
    saved_value: String,
    dirty: bool,
    file_busy: bool,

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
    recovery_path: PathBuf,
    legacy_recovery_path: Option<PathBuf>,
    recovery_clock: RecoveryClock,
    recovery_error: Option<SharedString>,
    /// The modal alert currently shown, if any (shared `rmac_ui::alert`).
    alert: Option<ActiveAlert>,
    _subscriptions: Vec<Subscription>,
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

fn recovery_failure_message() -> SharedString {
    "Text Editor could not safely update its private recovery data. The current buffer remains open; save the document before closing."
        .into()
}

impl EditorView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = rmac_editor::multiline("", window, cx);
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));

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

        let legacy_path = std::env::temp_dir().join("rmac-text-editor-recovery.txt");
        let (recovery_path, loaded) = match platform_recovery_path() {
            Ok(recovery_path) => {
                let loaded = storage::load_migrating_recovery(
                    &storage::RealStorage,
                    &recovery_path,
                    &legacy_path,
                );
                (recovery_path, loaded)
            }
            Err(failure) => {
                let content = storage::load_recovery(&storage::RealStorage, &legacy_path)
                    .ok()
                    .flatten();
                (
                    legacy_path.clone(),
                    storage::LoadedRecovery {
                        content,
                        legacy_path: None,
                        warning: Some(failure),
                    },
                )
            }
        };
        let alert = loaded.content.map(ActiveAlert::Recover);
        let recovery_error = loaded.warning.map(|_| recovery_failure_message());

        Self {
            alert,
            input,
            path: None,
            saved_bytes: None,
            text_format: document::TextFormat::default(),
            saved_value: String::new(),
            dirty: false,
            file_busy: false,
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
            recovery_path,
            legacy_recovery_path: loaded.legacy_path,
            recovery_clock: RecoveryClock::default(),
            recovery_error,
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

    // ── Dirty + autosave ────────────────────────────────────────────────

    fn on_buffer_changed(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        self.dirty = value != self.saved_value;
        if self.find_open {
            self.recompute_matches(cx);
        }
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
            let _ = this.update(cx, |this, cx| {
                if this.recovery_clock.should_write(generation, this.dirty) {
                    let content = this.input.read(cx).value().to_string();
                    match storage::save_recovery(
                        &storage::RealStorage,
                        storage::Operation::SaveRecovery,
                        &this.recovery_path,
                        content,
                    ) {
                        Ok(()) if this.legacy_recovery_path.is_none() => this.recovery_error = None,
                        Ok(()) => {}
                        Err(failure) => this.record_recovery_failure(failure, cx),
                    }
                }
            });
        })
        .detach();
    }

    fn record_recovery_failure(&mut self, _failure: storage::Failure, cx: &mut Context<Self>) {
        self.recovery_error = Some(recovery_failure_message());
        cx.notify();
    }

    fn clear_recovery(&mut self, cx: &mut Context<Self>) -> bool {
        self.recovery_clock.invalidate();
        match storage::remove_recoveries(
            &storage::RealStorage,
            &self.recovery_path,
            self.legacy_recovery_path.as_deref(),
        ) {
            Ok(()) => {
                self.legacy_recovery_path = None;
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
        self.dirty = false;
        self.clear_recovery(cx)
    }

    // ── File operations ─────────────────────────────────────────────────

    fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        self.guarded(Pending::New, window, cx);
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        self.guarded(Pending::Open, window, cx);
    }

    fn do_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.path = None;
        self.saved_bytes = None;
        self.text_format = document::TextFormat::default();
        self.rtf_runs = None;
        self.mark_clean(String::new(), cx);
        cx.notify();
    }

    /// Leave the read-only RTF preview and continue editing the extracted text
    /// as a new untitled plain-text document — the original `.rtf` is never
    /// overwritten.
    fn edit_as_plain_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.rtf_runs.take().is_some() {
            self.path = None;
            self.saved_bytes = None;
            self.text_format = document::TextFormat::default();
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
        cx.notify();
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
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
            let Some(path) = paths.into_iter().next() else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.file_busy = false;
                    cx.notify();
                });
                return;
            };
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
                        this.mark_clean(document.text, cx);
                    }
                    Ok(LoadedFile::RichText { text, runs }) => {
                        this.input
                            .update(cx, |state, cx| state.set_value(text.clone(), window, cx));
                        this.path = Some(path);
                        this.saved_bytes = None;
                        this.text_format = document::TextFormat::default();
                        this.rtf_runs = Some(runs);
                        this.mark_clean(text, cx);
                    }
                    Err(message) => {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Failed to open the file.",
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
        if self.file_busy {
            return;
        }
        self.save_with(None, window, cx);
    }

    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy || self.rtf_runs.is_some() {
            return;
        }
        let content = self.input.read(cx).value().to_string();
        self.save_to_new_path(content, self.text_format, None, window, cx);
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
        self.save_to_new_path(content, self.text_format, then, window, cx);
    }

    fn save_to_new_path(
        &mut self,
        content: String,
        format: document::TextFormat,
        then: Option<Pending>,
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
                    async move { save_document(&path, None, &content, format) }
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
                let recovery_cleared = self.mark_clean(saved.text, cx);
                if recovery_cleared {
                    if let Some(pending) = then {
                        self.perform(pending, window, cx);
                    }
                }
            }
            Err(error) => {
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

    /// If the buffer is dirty, ask before discarding; otherwise act immediately.
    fn guarded(&mut self, pending: Pending, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_busy {
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
            Some(ActiveAlert::Recover(content)) => {
                self.input
                    .update(cx, |s, cx| s.set_value(content, window, cx));
                // Recovered text is unsaved relative to the empty baseline, so
                // this marks the buffer dirty and re-arms autosave.
                self.on_buffer_changed(cx);
            }
            Some(ActiveAlert::ConfirmSave(pending)) => self.save_with(Some(pending), window, cx),
            Some(ActiveAlert::Conflict) => self.save_as(window, cx),
            Some(ActiveAlert::Error { .. }) | None => {}
        }
        cx.notify();
    }

    /// Secondary button: Discard (recover) / Don't Save (confirm).
    fn alert_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.alert.take() {
            Some(ActiveAlert::Recover(content)) => {
                if !self.clear_recovery(cx) {
                    self.alert = Some(ActiveAlert::Recover(content));
                }
            }
            Some(ActiveAlert::ConfirmSave(pending)) => {
                // Keep the draft recoverable while the Open picker is active:
                // cancelling the picker leaves the current document intact.
                if matches!(pending, Pending::Open) || self.clear_recovery(cx) {
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

    fn perform(&mut self, pending: Pending, window: &mut Window, cx: &mut Context<Self>) {
        match pending {
            Pending::New => self.do_new(window, cx),
            Pending::Open => self.do_open(window, cx),
            Pending::Close => cx.quit(),
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
                            .disabled(self.file_busy)
                            .tooltip("New")
                            .on_click(cx.listener(|this, _, window, cx| this.new_file(window, cx))),
                    )
                    .child(
                        Button::new("open", "")
                            .icon(Icon::new(IconName::FolderOpen).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(self.file_busy)
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
                    .child(
                        Button::new("save", "Save")
                            .primary()
                            .with_size(Size::Small)
                            .busy(self.file_busy)
                            .disabled(self.file_busy || self.rtf_runs.is_some())
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
                            .on_click(
                                cx.listener(|this, _, window, cx| this.replace_current(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("replace-all", "Replace All")
                            .ghost()
                            .with_size(Size::Small)
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
                    .child(cell(self.text_format.status()))
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
            ActiveAlert::Recover(_) => (
                "Recover unsaved changes?",
                "An autosaved document from a previous session was found.".into(),
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
                "Text Editor did not overwrite the external version. Save this buffer as a separate copy or cancel and inspect the other version."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-save-copy", "Save a Copy…", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
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
        let recovery_error = self.recovery_error.clone();

        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context(CTX)
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| this.save(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFileAs, window, cx| this.save_as(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleFind, window, cx| this.toggle_find(window, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleReplace, window, cx| this.toggle_replace(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindNext, window, cx| this.find_next(window, cx)))
            .on_action(cx.listener(|this, _: &FindPrev, window, cx| this.find_prev(window, cx)))
            .on_action(cx.listener(|this, _: &CloseBar, _, cx| this.close_bar(cx)))
            .on_action(cx.listener(|this, _: &ToggleMono, _, cx| this.toggle_mono(cx)))
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
                    .child(TextField::new(&self.input).h_full().appearance(false))
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
    rmac_ui::boot("Text Editor", 860.0, 640.0, |window, cx| {
        EditorView::new(window, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::{recovery_path_for_platform, RecoveryClock};
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
}
