//! rmac Text Editor — a fast, native TextEdit-style editor.
//!
//! Built on gpui-component's rope-backed `InputState` (multi-line + soft-wrap)
//! with native Open/Save dialogs. The editing core here is what the Notes app
//! will reuse (to be extracted into `rmac-editor`).

use std::path::PathBuf;

use gpui::{
    div, AppContext as _, Context, Entity, IntoElement, ParentElement, PathPromptOptions, Render,
    SharedString, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    ActiveTheme as _, StyledExt as _,
};

struct EditorView {
    input: Entity<InputState>,
    path: Option<PathBuf>,
}

impl EditorView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .soft_wrap(true)
                .placeholder("Start typing…")
        });
        Self { input, path: None }
    }

    /// Display name for the title strip.
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

    fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.path = None;
        cx.notify();
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(content) = std::fs::read_to_string(&path) else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.input
                    .update(cx, |s, cx| s.set_value(content, window, cx));
                this.path = Some(path);
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.input.read(cx).value().to_string();

        // Known path → write directly.
        if let Some(path) = self.path.clone() {
            let _ = std::fs::write(&path, content);
            return;
        }

        // Otherwise prompt for a destination.
        let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some("Untitled.txt"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let _ = std::fs::write(&path, content);
            let _ = this.update_in(cx, |this, _window, cx| {
                this.path = Some(path);
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toolbar = div()
            .h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("new")
                    .label("New")
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| this.new_file(window, cx))),
            )
            .child(
                Button::new("open")
                    .label("Open")
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| this.open(window, cx))),
            )
            .child(
                Button::new("save")
                    .label("Save")
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.filename()),
            );

        div()
            .size_full()
            .v_flex()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(rmac_ui::title_bar("Text Editor"))
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .child(Input::new(&self.input).h_full().appearance(false)),
            )
    }
}

fn main() {
    rmac_ui::boot("Text Editor", 900.0, 640.0, |window, cx| {
        EditorView::new(window, cx)
    });
}
