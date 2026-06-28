//! rmac Finder — a pixel-accurate macOS Finder (list view). See SPEC.md.

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    div, prelude::FluentBuilder as _, px, svg, AssetSource, ClickEvent, Context, Div, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement, Render, Result, SharedString,
    StatefulInteractiveElement as _, Stateful, Styled, Svg, Window,
};
use gpui_component::StyledExt as _;

// ---- bundled icons (app SVGs + gpui-component fallback) ----
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct AppAssets;

struct CombinedAssets;
impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(f) = AppAssets::get(path) {
            return Ok(Some(f.data));
        }
        gpui_component_assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut v: Vec<SharedString> = AppAssets::iter()
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(p.to_string()))
            .collect();
        if let Ok(mut o) = gpui_component_assets::Assets.list(path) {
            v.append(&mut o);
        }
        Ok(v)
    }
}

// ---- spec colors (light mode) ----
fn hsl(h: u32) -> Hsla {
    gpui::rgb(h).into()
}
fn list_bg() -> Hsla { hsl(0xffffff) }
fn toolbar_bg() -> Hsla { hsl(0xf6f6f6) }
fn sidebar_bg() -> Hsla { hsl(0xe9e9ed) }
fn alt_row() -> Hsla { hsl(0xf4f5f5) }
fn sel() -> Hsla { hsl(0x0063e1) }
fn accent() -> Hsla { hsl(0x007aff) }
fn sep() -> Hsla { hsl(0xe5e5e5) }
fn label() -> Hsla { hsl(0x272727) }
fn secondary() -> Hsla { hsl(0x808080) }
fn tertiary() -> Hsla { hsl(0xbfbfbf) }
fn drive_gray() -> Hsla { hsl(0x808080) }
fn white() -> Hsla { gpui::white() }

fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg().path(path).w(px(size)).h(px(size)).text_color(color).flex_none()
}

const SIDEBAR_W: f32 = 190.0;
const DATE_W: f32 = 184.0;
const SIZE_W: f32 = 80.0;
const KIND_W: f32 = 150.0;

#[derive(Clone)]
struct Entry {
    name: SharedString,
    path: PathBuf,
    is_dir: bool,
    size: SharedString,
    modified: SharedString,
    kind: SharedString,
}

#[derive(Clone)]
struct Place {
    name: SharedString,
    path: PathBuf,
    icon: &'static str,
    tint: Hsla,
}

struct Section {
    title: SharedString,
    places: Vec<Place>,
}

struct FinderView {
    cwd: PathBuf,
    entries: Vec<Entry>,
    selected: Option<usize>,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
    sections: Vec<Section>,
    dragging: bool,
}

impl FinderView {
    fn new(cx: &mut Context<Self>) -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
        let host = home
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Macintosh HD".to_string());
        let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");

        let p = |name: &str, path: PathBuf, icon: &'static str, tint: Hsla| Place {
            name: name.to_string().into(),
            path,
            icon,
            tint,
        };
        let sections = vec![
            Section {
                title: "Favorites".into(),
                places: vec![
                    p("Recents", home.clone(), "icons/clock.svg", accent()),
                    p("Applications", "/Applications".into(), "icons/layout-grid.svg", accent()),
                    p("Desktop", home.join("Desktop"), "icons/folder-fill.svg", accent()),
                    p("Documents", home.join("Documents"), "icons/folder-fill.svg", accent()),
                    p("Downloads", home.join("Downloads"), "icons/download.svg", accent()),
                ],
            },
            Section {
                title: "iCloud".into(),
                places: vec![p(
                    "iCloud Drive",
                    if icloud.is_dir() { icloud } else { home.clone() },
                    "icons/cloud.svg",
                    accent(),
                )],
            },
            Section {
                title: "Locations".into(),
                places: vec![
                    p("Macintosh HD", "/".into(), "icons/hard-drive.svg", drive_gray()),
                    p(&host, home.clone(), "icons/house.svg", drive_gray()),
                ],
            },
        ];

        let mut view = Self {
            cwd: home,
            entries: Vec::new(),
            selected: None,
            back: Vec::new(),
            fwd: Vec::new(),
            sections,
            dragging: false,
        };
        view.reload(cx);
        view
    }

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

    // ---- 52pt unified toolbar (draggable) ----
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let nav = |id: &'static str, glyph: &'static str, enabled: bool| {
            div()
                .id(id)
                .w(px(26.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .when(enabled, |el: Stateful<Div>| el.hover(|h| h.bg(hsl(0xe2e2e4))))
                .child(icon(
                    glyph,
                    17.0,
                    if enabled { hsl(0x3a3a3c) } else { tertiary() },
                ))
        };
        let seg = |glyph: &'static str, active: bool| {
            div()
                .w(px(34.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .when(active, |el: Div| el.bg(white()))
                .child(icon(glyph, 15.0, if active { label() } else { secondary() }))
        };
        let view_control = div()
            .flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded(px(7.0))
            .bg(hsl(0xe2e2e4))
            .child(seg("icons/layout-grid.svg", false))
            .child(seg("icons/list.svg", true))
            .child(seg("icons/columns-3.svg", false))
            .child(seg("icons/image.svg", false));

        let tool = |glyph: &'static str| {
            div()
                .w(px(30.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(glyph, 16.0, secondary()))
        };

        let search = div()
            .w(px(180.0))
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded(px(7.0))
            .bg(hsl(0xededef))
            .child(icon("icons/search.svg", 14.0, tertiary()))
            .child(div().text_size(px(13.0)).text_color(tertiary()).child("Search"));

        div()
            .id("toolbar")
            .h(px(52.0))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .pl(px(82.0))
            .pr_3()
            .bg(toolbar_bg())
            .border_b_1()
            .border_color(sep())
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = true))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = false))
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.dragging {
                    this.dragging = false;
                    window.start_window_move();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(nav("back", "icons/chevron-left.svg", !self.back.is_empty()).on_click(
                        cx.listener(|this, _, _, cx| this.go_back(cx)),
                    ))
                    .child(nav("fwd", "icons/chevron-right.svg", !self.fwd.is_empty()).on_click(
                        cx.listener(|this, _, _, cx| this.go_forward(cx)),
                    )),
            )
            .child(
                div()
                    .pl_1()
                    .text_size(px(15.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(label())
                    .child(self.title()),
            )
            .child(div().flex_1())
            .child(view_control)
            .child(tool("icons/share-2.svg"))
            .child(tool("icons/tag.svg"))
            .child(tool("icons/ellipsis.svg"))
            .child(search)
    }

    // ---- sidebar ----
    fn render_place(&self, p: &Place, cx: &Context<Self>) -> impl IntoElement {
        let selected = self.cwd == p.path;
        let path = p.path.clone();
        div()
            .id(SharedString::from(format!("place-{}-{}", p.name, p.path.display())))
            .flex()
            .items_center()
            .gap_2()
            .h(px(28.0))
            .px_2()
            .rounded(px(6.0))
            .when(selected, |el: Stateful<Div>| el.bg(hsl(0xd5d5da)))
            .when(!selected, |el: Stateful<Div>| el.hover(|h| h.bg(hsl(0x00000008))))
            .child(icon(p.icon, 17.0, p.tint))
            .child(div().text_size(px(13.0)).text_color(label()).child(p.name.clone()))
            .on_click(cx.listener(move |this, _, _, cx| this.navigate(path.clone(), cx)))
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut col = div()
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_2()
            .px_2()
            .gap_0p5()
            .bg(sidebar_bg())
            .border_r_1()
            .border_color(sep());
        for (si, section) in self.sections.iter().enumerate() {
            col = col.child(
                div()
                    .px_2()
                    .pt(px(if si == 0 { 2.0 } else { 12.0 }))
                    .pb_1()
                    .text_size(px(11.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child(section.title.clone()),
            );
            for p in &section.places {
                col = col.child(self.render_place(p, cx));
            }
        }
        col
    }

    // ---- list ----
    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .items_center()
            .h(px(26.0))
            .px_2()
            .border_b_1()
            .border_color(sep())
            .text_size(px(12.0))
            .text_color(secondary())
            .child(
                div()
                    .flex_1()
                    .pl(px(22.0))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child("Name")
                    .child(icon("icons/chevron-up.svg", 11.0, tertiary())),
            )
            .child(div().w(px(DATE_W)).child("Date Modified"))
            .child(div().w(px(SIZE_W)).flex().justify_end().child("Size"))
            .child(div().w(px(KIND_W)).pl_3().child("Kind"));

        let rows = self.entries.iter().enumerate().map(|(ix, e)| {
            let selected = self.selected == Some(ix);
            let primary = if selected { white() } else { label() };
            let sub = if selected { white() } else { secondary() };
            let glyph = if e.is_dir { "icons/folder-fill.svg" } else { "icons/file-fill.svg" };
            let icon_color = if selected {
                white()
            } else if e.is_dir {
                accent()
            } else {
                secondary()
            };

            div()
                .id(("row", ix))
                .flex()
                .items_center()
                .h(px(24.0))
                .px_2()
                .text_size(px(13.0))
                .when(selected, |el: Stateful<Div>| el.bg(sel()))
                .when(!selected && ix % 2 == 1, |el: Stateful<Div>| el.bg(alt_row()))
                .when(!selected, |el: Stateful<Div>| el.hover(|h| h.bg(hsl(0x0000000a))))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .min_w(px(0.0))
                        .child(
                            div().w(px(16.0)).flex().justify_center().when(e.is_dir, |el: Div| {
                                el.child(icon(
                                    "icons/chevron-right.svg",
                                    11.0,
                                    if selected { white() } else { tertiary() },
                                ))
                            }),
                        )
                        .child(icon(glyph, 16.0, icon_color))
                        .child(
                            div()
                                .pl(px(6.0))
                                .text_color(primary)
                                .truncate()
                                .child(e.name.clone()),
                        ),
                )
                .child(div().w(px(DATE_W)).text_color(sub).child(e.modified.clone()))
                .child(
                    div()
                        .w(px(SIZE_W))
                        .flex()
                        .justify_end()
                        .text_color(sub)
                        .child(e.size.clone()),
                )
                .child(div().w(px(KIND_W)).pl_3().text_color(sub).truncate().child(e.kind.clone()))
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
            .bg(list_bg())
            .child(header)
            .child(
                div()
                    .id("file-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(div().v_flex().children(rows)),
            )
    }
}

impl Render for FinderView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .bg(list_bg())
            .text_color(label())
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

// ---- helpers ----

fn read_entries(dir: &PathBuf) -> Vec<Entry> {
    let mut v: Vec<Entry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
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
        "md" => "Markdown Document".into(),
        "txt" => "Plain Text Document".into(),
        "json" => "JSON document".into(),
        "lock" => "Document".into(),
        "png" => "PNG image".into(),
        "jpg" | "jpeg" => "JPEG image".into(),
        "gif" => "GIF image".into(),
        "webp" => "WebP image".into(),
        "pdf" => "PDF document".into(),
        "zip" => "ZIP archive".into(),
        "gz" | "tar" => "Archive".into(),
        "app" => "Application".into(),
        "" => "Document".into(),
        other => format!("{} document", other.to_uppercase()),
    }
}

/// macOS Finder date: "Today at 11:12 AM", "Yesterday at 1:30 PM", "18 Apr 2026 at 2:42 PM".
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
        format!("Today at {time}")
    } else if days == 1 {
        format!("Yesterday at {time}")
    } else {
        format!("{} {} {} at {time}", dt.day(), dt.format("%b"), dt.year())
    }
}

fn main() {
    rmac_ui::boot_unified_with_assets(CombinedAssets, 1100.0, 720.0, |_window, cx| {
        FinderView::new(cx)
    });
}
