//! rmac Text Editor — a fast, native TextEdit-style editor.
//!
//! Rope-backed `InputState` body on a clean white page, with a unified macOS
//! toolbar (New / Open / Save), a find/replace bar (⌘F / ⇧⌘F), dirty-state
//! tracking with a modified indicator and unsaved-changes prompts, basic
//! autosave to a recovery file, and a Format affordance (monospace + font
//! size). Shares the editing configuration with Notes via `rmac-editor`.

mod rtf;

use std::{path::PathBuf, time::Duration};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    actions, div, font, px, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement as _, Styled, StyledText, Subscription, TextRun, UnderlineStyle,
    Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    input::{InputEvent, InputState, Position, RopeExt as _},
    Icon, IconName, Selectable as _, Sizable as _, Size, StyledExt as _,
};
use rmac_editor::Input;
use rmac_ui::mac;

const CTX: &str = "TextEditor";

actions!(
    text_editor,
    [
        NewFile,
        OpenFile,
        SaveFile,
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

/// A modal alert awaiting the user, shown via the shared `rmac_ui::alert`.
#[derive(Clone)]
enum ActiveAlert {
    /// A recovery file was found — Restore (load it) or Discard.
    Recover(String),
    /// The buffer is dirty before `Pending` — Save / Don't Save / Cancel.
    ConfirmSave(Pending),
    /// A save error — message + OK.
    Error(String),
}

struct EditorView {
    input: Entity<InputState>,
    path: Option<PathBuf>,
    /// Text as last saved (or opened/new) — the dirty baseline.
    saved_value: String,
    dirty: bool,

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
    autosave_gen: u64,
    /// The modal alert currently shown, if any (shared `rmac_ui::alert`).
    alert: Option<ActiveAlert>,
    _subscriptions: Vec<Subscription>,
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

        let recovery_path = std::env::temp_dir().join("rmac-text-editor-recovery.txt");

        // If an autosaved recovery file from a previous (crashed) session exists,
        // open the shared Recover alert once the view is live.
        let alert = std::fs::read_to_string(&recovery_path)
            .ok()
            .filter(|c| !c.is_empty())
            .map(ActiveAlert::Recover);

        Self {
            alert,
            input,
            path: None,
            saved_value: String::new(),
            dirty: false,
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
            autosave_gen: 0,
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
        self.schedule_autosave(cx);
        cx.notify();
    }

    /// Debounced autosave: each edit bumps a generation token and arms a timer;
    /// only the most recent timer actually writes the recovery file.
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        self.autosave_gen += 1;
        let gen = self.autosave_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                if this.autosave_gen == gen {
                    let content = this.input.read(cx).value().to_string();
                    let _ = std::fs::write(&this.recovery_path, content);
                }
            });
        })
        .detach();
    }

    fn mark_clean(&mut self, value: String) {
        self.saved_value = value;
        self.dirty = false;
        let _ = std::fs::remove_file(&self.recovery_path);
    }

    // ── File operations ─────────────────────────────────────────────────

    fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.guarded(Pending::New, window, cx);
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.guarded(Pending::Open, window, cx);
    }

    fn do_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.path = None;
        self.rtf_runs = None;
        self.mark_clean(String::new());
        cx.notify();
    }

    /// Leave the read-only RTF preview and continue editing the extracted text
    /// as a new untitled plain-text document — the original `.rtf` is never
    /// overwritten.
    fn edit_as_plain_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.rtf_runs.take().is_some() {
            self.path = None;
            self.dirty = true; // an unsaved derived document
            cx.notify();
        }
    }

    fn do_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                return;
            };
            // `.rtf` files open as a read-only formatted preview; the editable
            // body holds the extracted plain text.
            let is_rtf = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("rtf"));
            let rtf_runs = if is_rtf { rtf::parse_rtf(&bytes) } else { None };
            let content = match &rtf_runs {
                Some(runs) => runs.iter().map(|r| r.text.as_str()).collect::<String>(),
                None => String::from_utf8_lossy(&bytes).into_owned(),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.input
                    .update(cx, |s, cx| s.set_value(content.clone(), window, cx));
                this.path = Some(path);
                this.rtf_runs = rtf_runs;
                this.mark_clean(content);
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_with(None, window, cx);
    }

    /// Save the buffer; if `then` is set, run that pending action only **after**
    /// the save has actually succeeded (important for the async Save-As path so
    /// the destructive action never runs before the file is written).
    fn save_with(&mut self, then: Option<Pending>, window: &mut Window, cx: &mut Context<Self>) {
        // The RTF preview is read-only — never write plain text over the .rtf.
        if self.rtf_runs.is_some() {
            return;
        }
        let content = self.input.read(cx).value().to_string();
        if let Some(path) = self.path.clone() {
            match std::fs::write(&path, &content) {
                Ok(()) => {
                    self.mark_clean(content);
                    cx.notify();
                    if let Some(pending) = then {
                        self.perform(pending, window, cx);
                    }
                }
                Err(err) => {
                    // Write failed: keep dirty state and do NOT run the pending
                    // (destructive) action, so unsaved changes are preserved.
                    self.alert = Some(ActiveAlert::Error(format!("{}: {}", path.display(), err)));
                    cx.notify();
                }
            }
            return;
        }
        let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some("Untitled.txt"));
        cx.spawn_in(window, async move |this, cx| {
            // Save-As was cancelled or failed: do NOT run the pending action,
            // so unsaved changes are preserved instead of silently discarded.
            let Ok(Ok(Some(path))) = rx.await else { return };
            let write_result = std::fs::write(&path, &content);
            let _ = this.update_in(cx, |this, window, cx| match write_result {
                Ok(()) => {
                    this.path = Some(path);
                    this.mark_clean(content);
                    cx.notify();
                    if let Some(pending) = then {
                        this.perform(pending, window, cx);
                    }
                }
                Err(err) => {
                    // Write failed: keep dirty state and do NOT run the pending
                    // (destructive) action, so unsaved changes are preserved.
                    this.alert = Some(ActiveAlert::Error(format!("{}: {}", path.display(), err)));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// If the buffer is dirty, ask before discarding; otherwise act immediately.
    fn guarded(&mut self, pending: Pending, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dirty {
            self.perform(pending, window, cx);
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
            Some(ActiveAlert::Error(_)) | None => {}
        }
        cx.notify();
    }

    /// Secondary button: Discard (recover) / Don't Save (confirm).
    fn alert_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.alert.take() {
            Some(ActiveAlert::Recover(_)) => {
                let _ = std::fs::remove_file(&self.recovery_path);
            }
            Some(ActiveAlert::ConfirmSave(pending)) => self.perform(pending, window, cx),
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
                        Button::new("new")
                            .icon(Icon::new(IconName::File).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .tooltip("New")
                            .on_click(cx.listener(|this, _, window, cx| this.new_file(window, cx))),
                    )
                    .child(
                        Button::new("open")
                            .icon(Icon::new(IconName::FolderOpen).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Medium)
                            .tooltip("Open")
                            .on_click(cx.listener(|this, _, window, cx| this.open(window, cx))),
                    )
                    .child(
                        Button::new("find")
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
                    .text_size(px(13.0))
                    .font_weight(mac::MEDIUM)
                    .text_color(mac::text())
                    .child(title)
                    .when(dirty, |d| {
                        d.child(
                            div()
                                .text_size(px(12.0))
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
                        Button::new("mono")
                            .label("Mono")
                            .ghost()
                            .with_size(Size::Small)
                            .selected(self.mono)
                            .tooltip("Monospace font")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_mono(cx))),
                    )
                    .child(
                        Button::new("font-dec")
                            .icon(Icon::new(IconName::Minus).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Small)
                            .tooltip("Smaller text")
                            .on_click(cx.listener(|this, _, _, cx| this.decrease_font(cx))),
                    )
                    .child(
                        Button::new("font-inc")
                            .icon(Icon::new(IconName::Plus).text_color(mac::text()))
                            .ghost()
                            .with_size(Size::Small)
                            .tooltip("Larger text")
                            .on_click(cx.listener(|this, _, _, cx| this.increase_font(cx))),
                    )
                    .child(
                        Button::new("save")
                            .label("Save")
                            .primary()
                            .with_size(Size::Small)
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
                    .child(Input::new(&self.find_input).appearance(true)),
            )
            .child(
                Button::new("find-prev")
                    .icon(Icon::new(IconName::ChevronUp).text_color(mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("Previous match")
                    .on_click(cx.listener(|this, _, window, cx| this.find_prev(window, cx))),
            )
            .child(
                Button::new("find-next")
                    .icon(Icon::new(IconName::ChevronDown).text_color(mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("Next match")
                    .on_click(cx.listener(|this, _, window, cx| this.find_next(window, cx))),
            )
            .child(
                div()
                    .min_w(px(64.0))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child(status),
            )
            .child(div().flex_1())
            .child(
                Button::new("find-close")
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
                            .child(Input::new(&self.replace_input).appearance(true)),
                    )
                    .child(
                        Button::new("replace-one")
                            .label("Replace")
                            .ghost()
                            .with_size(Size::Small)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.replace_current(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("replace-all")
                            .label("Replace All")
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
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child("Read-only RTF preview — formatting shown as in the document."),
            )
            .child(
                Button::new("edit-plain")
                    .label("Edit as Plain Text")
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
                .text_size(px(11.0))
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
            ActiveAlert::Error(msg) => (
                "Failed to save the file.",
                msg,
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

        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context(CTX)
            .on_action(cx.listener(|this, _: &NewFile, window, cx| this.new_file(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| this.save(window, cx)))
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
                    .child(Input::new(&self.input).h_full().appearance(false))
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
