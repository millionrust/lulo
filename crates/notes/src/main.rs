//! rmac Notes — an Apple Notes-style app, built for fidelity.
//!
//! Three columns: folders sidebar │ notes list │ editor. The editor splits each
//! note into a big bold title (first line) and a regular body, exactly like
//! macOS Notes. Notes are `.md` files under `~/Documents/rmac-notes`,
//! auto-saved on a 1.5s debounce.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    div, prelude::FluentBuilder as _, px, AppContext as _, Context, Div, Entity,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Stateful, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    Icon, IconName, Sizable as _, Size, StyledExt as _,
};
use rmac_editor::{Input, InputState};
use rmac_ui::mac;

const FOLDERS_W: f32 = 200.0;
const LIST_W: f32 = 292.0;

struct Note {
    path: PathBuf,
    title: SharedString,
    snippet: SharedString,
    date: SharedString,
}

struct NotesView {
    dir: PathBuf,
    notes: Vec<Note>,
    selected: Option<usize>,
    title: Entity<InputState>,
    body: Entity<InputState>,
    search: Entity<InputState>,
    last_saved: String,
}

impl NotesView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let dir = PathBuf::from(home).join("Documents").join("rmac-notes");
        std::fs::create_dir_all(&dir).ok();

        // Title is single-line; body is multi-line soft-wrapped.
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let body = rmac_editor::multiline("Note", window, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();

        let mut view = Self {
            notes: scan_notes(&dir),
            dir,
            selected: None,
            title,
            body,
            search,
            last_saved: String::new(),
        };

        if !view.notes.is_empty() {
            view.select(0, window, cx);
        }

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

    /// The full document text (title line + body) for the open note.
    fn doc(&self, cx: &Context<Self>) -> String {
        let t = self.title.read(cx).value().to_string();
        let b = self.body.read(cx).value().to_string();
        if t.is_empty() && b.is_empty() {
            String::new()
        } else {
            format!("{t}\n{b}")
        }
    }

    /// Split a stored document into the title field and the body field.
    fn load_doc(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut parts = text.splitn(2, '\n');
        let t = parts.next().unwrap_or("").to_string();
        let b = parts.next().unwrap_or("").to_string();
        self.title.update(cx, |s, cx| s.set_value(t, window, cx));
        self.body.update(cx, |s, cx| s.set_value(b, window, cx));
    }

    fn save_current(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        let doc = self.doc(cx);
        if doc == self.last_saved {
            return;
        }
        if let Some(note) = self.notes.get_mut(ix) {
            std::fs::write(&note.path, &doc).ok();
            note.title = title_of(&doc).into();
            note.snippet = snippet_of(&doc).into();
            note.date = date_label(SystemTime::now()).into();
            self.last_saved = doc;
            cx.notify();
        }
    }

    fn select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.save_current(cx);
        let Some(note) = self.notes.get(ix) else { return };
        let text = std::fs::read_to_string(&note.path).unwrap_or_default();
        self.load_doc(&text, window, cx);
        self.selected = Some(ix);
        self.last_saved = self.doc(cx);
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
                snippet: "No additional text".into(),
                date: date_label(SystemTime::now()).into(),
            },
        );
        self.select(0, window, cx);
    }

    fn delete_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else { return };
        if ix >= self.notes.len() {
            return;
        }
        let note = self.notes.remove(ix);
        std::fs::remove_file(&note.path).ok();
        self.selected = None;
        self.last_saved.clear();

        if self.notes.is_empty() {
            self.load_doc("", window, cx);
        } else {
            self.select(ix.min(self.notes.len() - 1), window, cx);
        }
        cx.notify();
    }

    // ---- chrome ----

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let row = div()
            .size_full()
            .flex()
            .items_center()
            .child(div().w(px(FOLDERS_W - 80.0)))
            .child(
                div()
                    .w(px(LIST_W))
                    .flex()
                    .items_center()
                    .justify_end()
                    .pr_3()
                    .child(
                        Button::new("compose")
                            .icon(IconName::Plus)
                            .ghost()
                            .with_size(Size::Medium)
                            .tooltip("New Note")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_note(window, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_end()
                    .pr_4()
                    .child(
                        Button::new("delete")
                            .icon(IconName::Delete)
                            .ghost()
                            .with_size(Size::Medium)
                            .tooltip("Delete Note")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.delete_current(window, cx)
                            })),
                    ),
            );
        rmac_ui::toolbar(row)
    }

    fn render_folders(&self, _cx: &Context<Self>) -> impl IntoElement {
        let count = self.notes.len();
        div()
            .w(px(FOLDERS_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_3()
            .px_2()
            .bg(mac::sidebar())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_size(px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("ICLOUD"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(px(6.0))
                    .bg(mac::sidebar_selection())
                    .child(
                        Icon::new(IconName::Folder)
                            .text_color(mac::notes_accent())
                            .with_size(Size::Small),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.0))
                            .text_color(mac::text())
                            .child("All Notes"),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(mac::text_tertiary())
                            .child(count.to_string()),
                    ),
            )
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let q = self.search.read(cx).value().to_lowercase();
        let mut items: Vec<gpui::AnyElement> = Vec::new();
        let n = self.notes.len();
        for (ix, note) in self.notes.iter().enumerate() {
            if !q.is_empty()
                && !note.title.to_lowercase().contains(&q)
                && !note.snippet.to_lowercase().contains(&q)
            {
                continue;
            }
            let selected = self.selected == Some(ix);
            items.push(
                div()
                    .id(("note", ix))
                    .mx_1()
                    .px_3()
                    .py_2()
                    .rounded(px(6.0))
                    .when(selected, |el: Stateful<Div>| el.bg(mac::notes_selection()))
                    .when(!selected, |el: Stateful<Div>| {
                        el.hover(|h| h.bg(mac::hover()))
                    })
                    .child(
                        div()
                            .v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(mac::SEMIBOLD)
                                    .text_color(mac::text())
                                    .truncate()
                                    .child(note.title.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(mac::MEDIUM)
                                            .text_color(mac::text())
                                            .child(note.date.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.0))
                                            .text_color(mac::text_secondary())
                                            .truncate()
                                            .child(note.snippet.clone()),
                                    ),
                            ),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.select(ix, window, cx)),
                    )
                    .into_any_element(),
            );
            let next_selected = self.selected == Some(ix + 1);
            if ix + 1 < n && !selected && !next_selected {
                items.push(
                    div()
                        .mx_4()
                        .h(px(1.0))
                        .bg(mac::separator())
                        .into_any_element(),
                );
            }
        }

        div()
            .w(px(LIST_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .bg(mac::list())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div().px_2().py_2().child(
                    div()
                        .h(px(28.0))
                        .flex()
                        .items_center()
                        .px_2()
                        .rounded(px(7.0))
                        .bg(gpui::rgb(0xededf0))
                        .child(div().flex_1().child(Input::new(&self.search).appearance(false))),
                ),
            )
            .child(
                div()
                    .id("notes-scroll")
                    .flex_1()
                    .py_1()
                    .overflow_y_scroll()
                    .child(div().v_flex().children(items)),
            )
    }

    fn render_editor(&self, _cx: &Context<Self>) -> impl IntoElement {
        if self.selected.is_some() {
            let meta = self
                .selected
                .and_then(|ix| self.notes.get(ix))
                .map(|n| n.date.clone())
                .unwrap_or_default();
            div()
                .size_full()
                .v_flex()
                .bg(mac::window())
                .child(
                    div()
                        .pt_3()
                        .pb_1()
                        .flex()
                        .justify_center()
                        .text_size(px(11.0))
                        .text_color(mac::text_secondary())
                        .child(meta),
                )
                // Big bold title (cascades into the single-line Input).
                .child(
                    div()
                        .px(px(44.0))
                        .pt_1()
                        .text_size(px(28.0))
                        .line_height(px(34.0))
                        .font_weight(mac::BOLD)
                        .text_color(mac::text())
                        .child(Input::new(&self.title).appearance(false)),
                )
                // Body.
                .child(
                    div()
                        .flex_1()
                        .px(px(44.0))
                        .pt_2()
                        .pb_4()
                        .text_size(px(16.0))
                        .line_height(px(24.0))
                        .text_color(mac::text())
                        .child(Input::new(&self.body).h_full().appearance(false)),
                )
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(mac::window())
                .text_size(px(15.0))
                .text_color(mac::text_tertiary())
                .child("No Note Selected")
                .into_any_element()
        }
    }
}

impl Render for NotesView {
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
                    .flex()
                    .child(self.render_folders(cx))
                    .child(self.render_list(cx))
                    .child(div().flex_1().child(self.render_editor(cx))),
            )
    }
}

// ---- pure helpers ----

fn title_of(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| {
            let t: String = l.trim_start_matches('#').trim().chars().take(60).collect();
            if t.is_empty() { "New Note".to_string() } else { t }
        })
        .unwrap_or_else(|| "New Note".to_string())
}

fn snippet_of(body: &str) -> String {
    let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty());
    let _title = lines.next();
    let rest: String = lines.collect::<Vec<_>>().join(" ");
    if rest.is_empty() {
        "No additional text".to_string()
    } else {
        rest.chars().take(80).collect()
    }
}

/// macOS-style relative date: time today, "Yesterday", weekday this week, else M/D/YY.
fn date_label(t: SystemTime) -> String {
    let dt: DateTime<Local> = t.into();
    let now = Local::now();
    let days = now.date_naive().signed_duration_since(dt.date_naive()).num_days();
    if days == 0 {
        let h = dt.hour();
        let (h12, ap) = if h == 0 {
            (12, "AM")
        } else if h < 12 {
            (h, "AM")
        } else if h == 12 {
            (12, "PM")
        } else {
            (h - 12, "PM")
        };
        format!("{}:{:02} {}", h12, dt.minute(), ap)
    } else if days == 1 {
        "Yesterday".to_string()
    } else if days < 7 {
        dt.format("%A").to_string()
    } else {
        format!("{}/{}/{:02}", dt.month(), dt.day(), dt.year() % 100)
    }
}

fn scan_notes(dir: &PathBuf) -> Vec<Note> {
    let mut entries: Vec<(PathBuf, SystemTime)> = std::fs::read_dir(dir)
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
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((path, mtime))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1));

    entries
        .into_iter()
        .map(|(path, mtime)| {
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            Note {
                title: title_of(&body).into(),
                snippet: snippet_of(&body).into(),
                date: date_label(mtime).into(),
                path,
            }
        })
        .collect()
}

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
    rmac_ui::boot("Notes", 1080.0, 720.0, |window, cx| {
        NotesView::new(window, cx)
    });
}
