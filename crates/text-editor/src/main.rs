//! rmac Text Editor — a fast, native TextEdit-style editor.
//!
//! Rope-backed `InputState` body on a clean white page, with a unified macOS
//! toolbar (New / Open / Save) and native file dialogs. Shares the editing
//! configuration with Notes via `rmac-editor`.

use std::path::PathBuf;

use gpui::{
    div, px, Context, Entity, IntoElement, ParentElement, PathPromptOptions, Render, SharedString,
    Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    Icon, IconName, Sizable as _, Size, StyledExt as _,
};
use rmac_editor::{Input, InputState};
use rmac_ui::mac;

struct EditorView {
    input: Entity<InputState>,
    path: Option<PathBuf>,
}

impl EditorView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = rmac_editor::multiline("", window, cx);
        Self { input, path: None }
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

    fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
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
        if let Some(path) = self.path.clone() {
            let _ = std::fs::write(&path, content);
            return;
        }
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

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(mac::MEDIUM)
                    .text_color(mac::text())
                    .child(self.filename()),
            )
            .child(
                Button::new("save")
                    .label("Save")
                    .primary()
                    .with_size(Size::Small)
                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
            );
        rmac_ui::toolbar(row)
    }
}

impl Render for EditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex_1()
                    .px(px(48.0))
                    .py(px(20.0))
                    .text_size(px(15.0))
                    .line_height(px(23.0))
                    .child(Input::new(&self.input).h_full().appearance(false)),
            )
    }
}

fn main() {
    rmac_ui::boot("Text Editor", 860.0, 640.0, |window, cx| {
        EditorView::new(window, cx)
    });
}
