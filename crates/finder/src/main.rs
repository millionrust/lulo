//! rmac Finder — a fast, native file manager.
//!
//! macOS Finder layout: Favorites sidebar │ columnar list (Name / Date Modified /
//! Size / Kind). Directories load on a background thread (yazi-style) so the UI
//! never blocks on large folders. Double-click opens folders (navigate) or files
//! (system default); back/forward history; single-click selects.

use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    div, prelude::FluentBuilder as _, px, ClickEvent, Context, Div, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Stateful,
    Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    Disableable as _, Icon, IconName, Sizable as _, Size, StyledExt as _,
};
use rmac_ui::mac;

const SIDEBAR_W: f32 = 184.0;
const DATE_W: f32 = 168.0;
const SIZE_W: f32 = 84.0;
const KIND_W: f32 = 132.0;

fn blue() -> gpui::Hsla {
    gpui::rgb(0x0a84ff).into()
}
fn folder_blue() -> gpui::Hsla {
    gpui::rgb(0x3b9eff).into()
}

#[derive(Clone)]
struct Entry {
    name: SharedString,
    path: PathBuf,
    is_dir: bool,
    size: SharedString,
    modified: SharedString,
    kind: SharedString,
}

struct FinderView {
    cwd: PathBuf,
    entries: Vec<Entry>,
    selected: Option<usize>,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
    favorites: Vec<(SharedString, PathBuf)>,
}

impl FinderView {
    fn new(cx: &mut Context<Self>) -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
        let fav = |name: &str, sub: &str| -> (SharedString, PathBuf) {
            (name.to_string().into(), home.join(sub))
        };
        let favorites = vec![
            ("Home".to_string().into(), home.clone()),
            fav("Desktop", "Desktop"),
            fav("Documents", "Documents"),
            fav("Downloads", "Downloads"),
            ("Applications".to_string().into(), PathBuf::from("/Applications")),
        ];

        let mut view = Self {
            cwd: home,
            entries: Vec::new(),
            selected: None,
            back: Vec::new(),
            fwd: Vec::new(),
            favorites,
        };
        view.reload(cx);
        view
    }

    /// Read the current directory off the main thread, then swap in the result.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.cwd.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let entries = cx
                .background_executor()
                .spawn(async move { read_entries(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.entries = entries;
                this.selected = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if path == self.cwd || !path.is_dir() {
            return;
        }
        self.back.push(self.cwd.clone());
        self.fwd.clear();
        self.cwd = path;
        self.reload(cx);
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.back.pop() {
            self.fwd.push(self.cwd.clone());
            self.cwd = p;
            self.reload(cx);
        }
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.fwd.pop() {
            self.back.push(self.cwd.clone());
            self.cwd = p;
            self.reload(cx);
        }
    }

    fn open(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(e) = self.entries.get(ix).cloned() else {
            return;
        };
        if e.is_dir {
            self.navigate(e.path, cx);
        } else {
            cx.open_with_system(&e.path);
        }
    }

    fn title(&self) -> SharedString {
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Macintosh HD".to_string())
            .into()
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let back_disabled = self.back.is_empty();
        let fwd_disabled = self.fwd.is_empty();
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
                        Button::new("back")
                            .icon(IconName::ChevronLeft)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(back_disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
                    )
                    .child(
                        Button::new("fwd")
                            .icon(IconName::ChevronRight)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(fwd_disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text())
                    .child(self.title()),
            )
            // right spacer balances the back/forward cluster so the title stays centered
            .child(div().w(px(64.0)));
        rmac_ui::toolbar(row)
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let rows = self.favorites.iter().cloned().map(|(name, path)| {
            let selected = self.cwd == path;
            div()
                .id(SharedString::from(format!("fav-{}", name)))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .when(selected, |el: Stateful<Div>| el.bg(mac::sidebar_selection()))
                .when(!selected, |el: Stateful<Div>| {
                    el.hover(|h| h.bg(mac::hover()))
                })
                .child(
                    Icon::new(IconName::Folder)
                        .text_color(folder_blue())
                        .with_size(Size::Small),
                )
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(mac::text())
                        .child(name),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.navigate(path.clone(), cx)))
        });

        div()
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_3()
            .px_2()
            .gap_0p5()
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
                    .child("Favorites"),
            )
            .children(rows)
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Column header.
        let header = div()
            .flex()
            .items_center()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(mac::separator())
            .text_size(px(11.0))
            .text_color(mac::text_secondary())
            .child(div().flex_1().child("Name"))
            .child(div().w(px(DATE_W)).child("Date Modified"))
            .child(div().w(px(SIZE_W)).flex().justify_end().child("Size"))
            .child(div().w(px(KIND_W)).child("Kind"));

        let rows = self.entries.iter().enumerate().map(|(ix, e)| {
            let selected = self.selected == Some(ix);
            let primary = if selected { gpui::white() } else { mac::text() };
            let secondary = if selected {
                gpui::white()
            } else {
                mac::text_secondary()
            };
            let icon = if e.is_dir {
                IconName::Folder
            } else {
                IconName::File
            };
            let icon_color = if selected {
                gpui::white()
            } else if e.is_dir {
                folder_blue()
            } else {
                mac::text_secondary()
            };

            div()
                .id(("row", ix))
                .flex()
                .items_center()
                .px_3()
                .py_1()
                .text_size(px(13.0))
                .when(selected, |el: Stateful<Div>| el.bg(blue()))
                .when(!selected, |el: Stateful<Div>| el.hover(|h| h.bg(mac::hover())))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w(px(0.0))
                        .child(Icon::new(icon).text_color(icon_color).with_size(Size::Small))
                        .child(div().text_color(primary).truncate().child(e.name.clone())),
                )
                .child(
                    div()
                        .w(px(DATE_W))
                        .text_size(px(12.0))
                        .text_color(secondary)
                        .child(e.modified.clone()),
                )
                .child(
                    div()
                        .w(px(SIZE_W))
                        .flex()
                        .justify_end()
                        .text_size(px(12.0))
                        .text_color(secondary)
                        .child(e.size.clone()),
                )
                .child(
                    div()
                        .w(px(KIND_W))
                        .pl_3()
                        .text_size(px(12.0))
                        .text_color(secondary)
                        .truncate()
                        .child(e.kind.clone()),
                )
                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                    if ev.click_count() >= 2 {
                        this.open(ix, cx);
                    } else {
                        this.selected = Some(ix);
                        cx.notify();
                    }
                }))
        });

        div()
            .flex_1()
            .v_flex()
            .bg(mac::window())
            .child(header)
            .child(
                div()
                    .id("file-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(div().v_flex().py_1().children(rows)),
            )
    }
}

impl Render for FinderView {
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
                    .child(self.render_sidebar(cx))
                    .child(self.render_list(cx)),
            )
    }
}

// ---- pure helpers ----

fn read_entries(dir: &PathBuf) -> Vec<Entry> {
    let mut v: Vec<Entry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue; // Finder hides dotfiles by default
            }
            let path = e.path();
            let md = e.metadata().ok();
            let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = if is_dir {
                "--".to_string()
            } else {
                human_size(md.as_ref().map(|m| m.len()).unwrap_or(0))
            };
            let modified = md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(date_label)
                .unwrap_or_default();
            let kind = kind_of(&path, is_dir);
            v.push(Entry {
                name: name.into(),
                path,
                is_dir,
                size: size.into(),
                modified: modified.into(),
                kind: kind.into(),
            });
        }
    }
    v.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

fn human_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K * K {
        format!("{:.2} GB", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.1} MB", b / (K * K))
    } else if b >= K {
        format!("{:.0} KB", b / K)
    } else {
        format!("{bytes} bytes")
    }
}

fn kind_of(path: &PathBuf, is_dir: bool) -> String {
    if is_dir {
        return "Folder".to_string();
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "rs" => "Rust Source".into(),
        "toml" => "TOML Document".into(),
        "md" => "Markdown".into(),
        "txt" => "Plain Text".into(),
        "json" => "JSON Document".into(),
        "lock" => "Lock File".into(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" => "Image".into(),
        "pdf" => "PDF Document".into(),
        "zip" | "gz" | "tar" => "Archive".into(),
        "" => "Document".into(),
        other => format!("{} File", other.to_uppercase()),
    }
}

/// macOS-style date: time today, "Yesterday", weekday this week, else M/D/YY h:mm.
fn date_label(t: SystemTime) -> String {
    let dt: DateTime<Local> = t.into();
    let now = Local::now();
    let (h12, ap) = {
        let h = dt.hour();
        if h == 0 {
            (12, "AM")
        } else if h < 12 {
            (h, "AM")
        } else if h == 12 {
            (12, "PM")
        } else {
            (h - 12, "PM")
        }
    };
    let time = format!("{}:{:02} {}", h12, dt.minute(), ap);
    let days = now.date_naive().signed_duration_since(dt.date_naive()).num_days();
    if days == 0 {
        format!("Today, {time}")
    } else if days == 1 {
        format!("Yesterday, {time}")
    } else if days < 7 {
        format!("{}, {time}", dt.format("%A"))
    } else {
        format!("{}/{}/{} {time}", dt.month(), dt.day(), dt.year() % 100)
    }
}

fn main() {
    rmac_ui::boot("Finder", 980.0, 640.0, |_window, cx| FinderView::new(cx));
}
