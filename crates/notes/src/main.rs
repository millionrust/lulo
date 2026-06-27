//! rmac Notes — an Apple Notes-style app.
//!
//! Two-pane: a sidebar list of notes and an editor. Notes are plain `.md`
//! files under `~/Documents/rmac-notes`, auto-saved on a 1.5s debounce. The
//! editor body is the shared `rmac-editor` core (also used by Text Editor).

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    div, prelude::FluentBuilder as _, AppContext as _, Context, Div, Entity,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Stateful, Styled, Window,
};
use gpui_component::{button::Button, ActiveTheme as _, StyledExt as _};
use rmac_editor::{Input, InputState};

struct Note {
    path: PathBuf,
    title: SharedString,
}

struct NotesView {
    dir: PathBuf,
    notes: Vec<Note>,
    selected: Option<usize>,
    input: Entity<InputState>,
    last_saved: String,
}

impl NotesView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let dir = PathBuf::from(home).join("Documents").join("rmac-notes");
        std::fs::create_dir_all(&dir).ok();

        let input = rmac_editor::multiline("Write a note…", window, cx);
        let mut view = Self {
            notes: scan_notes(&dir),
            dir,
            selected: None,
            input,
            last_saved: String::new(),
        };

        if !view.notes.is_empty() {
            view.select(0, window, cx);
        }

        // Auto-save loop — persists the open note on a debounce.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(1500))
                    .await;
                let Some(this) = this.upgrade() else { break };
                if cx
                    .update_entity(&this, |view: &mut NotesView, cx| view.save_current(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        view
    }

    /// Persist the open note if its text changed since the last save.
    fn save_current(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let body = rmac_editor::value(&self.input, cx);
        if body == self.last_saved {
            return;
        }
        if let Some(note) = self.notes.get_mut(ix) {
            std::fs::write(&note.path, &body).ok();
            note.title = rmac_editor::title_from_body(&body, "New Note").into();
            self.last_saved = body;
            cx.notify();
        }
    }

    /// Switch to the note at `ix`, saving the current one first.
    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.save_current(cx);
        let Some(note) = self.notes.get(ix) else { return };
        let body = std::fs::read_to_string(&note.path).unwrap_or_default();
        self.input
            .update(cx, |s, cx| s.set_value(body.clone(), window, cx));
        self.last_saved = body;
        self.selected = Some(ix);
        cx.notify();
    }

    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_current(cx);
        let path = unique_path(&self.dir);
        std::fs::write(&path, "").ok();
        self.notes.insert(
            0,
            Note {
                path,
                title: "New Note".into(),
            },
        );
        self.select(0, window, cx);
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let rows = self.notes.iter().enumerate().map(|(ix, note)| {
            let selected = self.selected == Some(ix);
            div()
                .id(("note", ix))
                .px_3()
                .py_2()
                .rounded(gpui::px(6.0))
                .text_sm()
                .truncate()
                .when(selected, |el: Stateful<Div>| el.bg(cx.theme().secondary))
                .when(!selected, |el: Stateful<Div>| {
                    el.hover(|h| h.bg(cx.theme().muted))
                })
                .child(note.title.clone())
                .on_click(cx.listener(move |this, _, window, cx| this.select(ix, window, cx)))
        });

        div()
            .v_flex()
            .w(gpui::px(240.0))
            .h_full()
            .gap_1()
            .p_2()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                Button::new("new-note")
                    .label("New Note")
                    .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
            )
            .child(div().v_flex().gap_1().children(rows))
    }
}

impl Render for NotesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor: gpui::AnyElement = if self.selected.is_some() {
            div()
                .size_full()
                .p_3()
                .child(Input::new(&self.input).h_full().appearance(false))
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("No note selected")
                .into_any_element()
        };

        div()
            .size_full()
            .v_flex()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(rmac_ui::title_bar("Notes"))
            .child(
                div()
                    .flex_1()
                    .h_flex()
                    .child(self.render_sidebar(cx))
                    .child(div().flex_1().child(editor)),
            )
    }
}

/// Scan a directory for `.md` notes, newest first, titled by first line.
fn scan_notes(dir: &PathBuf) -> Vec<Note> {
    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("md") {
                return None;
            }
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            Some((path, mtime))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));

    entries
        .into_iter()
        .map(|(path, _)| {
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            let fallback = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Note".to_string());
            Note {
                title: rmac_editor::title_from_body(&body, &fallback).into(),
                path,
            }
        })
        .collect()
}

/// A non-colliding `note-<n>.md` path in `dir`.
fn unique_path(dir: &PathBuf) -> PathBuf {
    let mut n = 1;
    loop {
        let path = dir.join(format!("note-{n}.md"));
        if !path.exists() {
            return path;
        }
        n += 1;
    }
}

fn main() {
    rmac_ui::boot("Notes", 1000.0, 680.0, |window, cx| {
        NotesView::new(window, cx)
    });
}
