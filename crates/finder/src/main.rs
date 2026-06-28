//! rmac Finder — a functional macOS-style file manager (list view). See SPEC.md.
//!
//! Beyond the pixel-accurate chrome: multi-selection, file operations
//! (new folder, rename, duplicate, copy/cut/paste, move-to-trash), keyboard
//! shortcuts + right-click context menus, live search, clickable sort headers,
//! hidden-file toggle, and live directory watching.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    actions, div, img, prelude::FluentBuilder as _, px, svg, AppContext as _, AssetSource, ClickEvent,
    Context, Div, FocusHandle, Focusable as _, Hsla, InteractiveElement as _, IntoElement,
    KeyBinding, KeyDownEvent, MouseButton, ParentElement, Render, Result, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Svg, Window,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, PopupMenu},
    StyledExt as _,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

actions!(
    finder,
    [
        NewFolder, RenameItem, Duplicate, MoveToTrash, DeleteItem, CopyItems, CutItems, PasteItems,
        SelectAll, GoUp, ToggleHidden, OpenItems, QuickLook, GetInfo, NewTab, CloseTab,
    ]
);

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Icon,
    List,
    Column,
    Gallery,
}

/// Drag payload: the file paths being dragged.
struct DraggedPaths(Vec<PathBuf>);

/// The little pill shown under the cursor while dragging.
struct DragPreview {
    count: usize,
}
impl Render for DragPreview {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.count;
        div()
            .px_2()
            .py_0p5()
            .rounded(px(6.0))
            .bg(hsl(0x0a84ff))
            .text_color(gpui::white())
            .text_size(px(12.0))
            .child(if n == 1 { "1 item".to_string() } else { format!("{n} items") })
    }
}

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
    size_bytes: u64,
    mtime: SystemTime,
}

#[derive(Clone, Copy, PartialEq)]
enum PlaceKind {
    Item,
    Volume,
    Tag,
}

#[derive(Clone)]
struct Place {
    name: SharedString,
    path: PathBuf,
    icon: &'static str,
    tint: Hsla,
    kind: PlaceKind,
}

struct Section {
    title: SharedString,
    places: Vec<Place>,
}

#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    Name,
    Date,
    Size,
    Kind,
}

struct FinderView {
    cwd: PathBuf,
    tabs: Vec<PathBuf>,
    active: usize,
    home: PathBuf,
    thumbs: std::collections::HashMap<PathBuf, PathBuf>,
    entries: Vec<Entry>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    clipboard: Vec<PathBuf>,
    clip_cut: bool,
    renaming: Option<(usize, gpui::Entity<InputState>)>,
    show_hidden: bool,
    view: ViewMode,
    sort_key: SortKey,
    sort_asc: bool,
    query: gpui::Entity<InputState>,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
    sections: Vec<Section>,
    info: Option<usize>,
    dragging: bool,
    focus: FocusHandle,
    watcher: Option<RecommendedWatcher>,
    watched: Option<PathBuf>,
    dirty: Arc<AtomicBool>,
}

impl FinderView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
        let host = home
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Macintosh HD".to_string());
        let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");

        let p = |name: &str, path: PathBuf, icon: &'static str, tint: Hsla, kind: PlaceKind| Place {
            name: name.to_string().into(),
            path,
            icon,
            tint,
            kind,
        };

        // Real mounted volumes.
        let mut locations = vec![
            p(&host, home.clone(), "icons/house.svg", drive_gray(), PlaceKind::Item),
            p("Macintosh HD", "/".into(), "icons/hard-drive.svg", drive_gray(), PlaceKind::Item),
        ];
        if let Ok(rd) = std::fs::read_dir("/Volumes") {
            for e in rd.flatten() {
                let vp = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name == "Macintosh HD" || name.starts_with('.') {
                    continue;
                }
                locations.push(p(&name, vp, "icons/hard-drive.svg", drive_gray(), PlaceKind::Volume));
            }
        }

        let tag = |name: &str, color: u32| p(name, PathBuf::new(), "", hsl(color), PlaceKind::Tag);
        let sections = vec![
            Section {
                title: "Favorites".into(),
                places: vec![
                    p("Recents", home.clone(), "icons/clock.svg", accent(), PlaceKind::Item),
                    p("Applications", "/Applications".into(), "icons/layout-grid.svg", accent(), PlaceKind::Item),
                    p("Desktop", home.join("Desktop"), "icons/folder-fill.svg", accent(), PlaceKind::Item),
                    p("Documents", home.join("Documents"), "icons/folder-fill.svg", accent(), PlaceKind::Item),
                    p("Downloads", home.join("Downloads"), "icons/download.svg", accent(), PlaceKind::Item),
                ],
            },
            Section {
                title: "iCloud".into(),
                places: vec![p(
                    "iCloud Drive",
                    if icloud.is_dir() { icloud } else { home.clone() },
                    "icons/cloud.svg",
                    accent(),
                    PlaceKind::Item,
                )],
            },
            Section {
                title: "Locations".into(),
                places: locations,
            },
            Section {
                title: "Tags".into(),
                places: vec![
                    tag("Red", 0xff3b30),
                    tag("Orange", 0xff9500),
                    tag("Yellow", 0xffcc00),
                    tag("Green", 0x34c759),
                    tag("Blue", 0x007aff),
                    tag("Purple", 0xaf52de),
                    tag("Gray", 0x8e8e93),
                ],
            },
        ];

        // Keyboard shortcuts → actions (handled on the focused list).
        cx.bind_keys([
            KeyBinding::new("cmd-a", SelectAll, Some("Finder")),
            KeyBinding::new("cmd-c", CopyItems, Some("Finder")),
            KeyBinding::new("cmd-x", CutItems, Some("Finder")),
            KeyBinding::new("cmd-v", PasteItems, Some("Finder")),
            KeyBinding::new("cmd-d", Duplicate, Some("Finder")),
            KeyBinding::new("cmd-backspace", MoveToTrash, Some("Finder")),
            KeyBinding::new("shift-cmd-n", NewFolder, Some("Finder")),
            KeyBinding::new("cmd-up", GoUp, Some("Finder")),
            KeyBinding::new("cmd-down", OpenItems, Some("Finder")),
            KeyBinding::new("enter", RenameItem, Some("Finder")),
            KeyBinding::new("shift-cmd-.", ToggleHidden, Some("Finder")),
            KeyBinding::new("space", QuickLook, Some("Finder")),
            KeyBinding::new("cmd-i", GetInfo, Some("Finder")),
            KeyBinding::new("cmd-t", NewTab, Some("Finder")),
            KeyBinding::new("cmd-w", CloseTab, Some("Finder")),
        ]);

        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        cx.observe(&query, |_, _, cx| cx.notify()).detach();

        let dirty = Arc::new(AtomicBool::new(false));
        let d2 = dirty.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                d2.store(true, Ordering::Relaxed);
            }
        })
        .ok();

        let focus = cx.focus_handle();
        window.focus(&focus);

        let mut view = Self {
            cwd: home.clone(),
            tabs: vec![home.clone()],
            active: 0,
            home,
            thumbs: std::collections::HashMap::new(),
            entries: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            clipboard: Vec::new(),
            clip_cut: false,
            renaming: None,
            show_hidden: false,
            view: ViewMode::List,
            sort_key: SortKey::Name,
            sort_asc: true,
            query,
            back: Vec::new(),
            fwd: Vec::new(),
            sections,
            info: None,
            dragging: false,
            focus,
            watcher,
            watched: None,
            dirty,
        };
        view.reload(cx);

        // Live directory watching → reload on filesystem changes.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor()
                .timer(Duration::from_millis(600))
                .await;
            let r = this.update(cx, |this: &mut FinderView, cx| {
                if this.dirty.swap(false, Ordering::Relaxed) {
                    this.reload(cx);
                }
            });
            if r.is_err() {
                break;
            }
        })
        .detach();

        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            *t = self.cwd.clone();
        }
        // (Re)watch the current directory.
        if let Some(w) = self.watcher.as_mut() {
            if let Some(old) = self.watched.take() {
                let _ = w.unwatch(&old);
            }
            if w.watch(&self.cwd, RecursiveMode::NonRecursive).is_ok() {
                self.watched = Some(self.cwd.clone());
            }
        }

        let path = self.cwd.clone();
        let show_hidden = self.show_hidden;
        let key = self.sort_key;
        let asc = self.sort_asc;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let entries = cx
                .background_executor()
                .spawn(async move {
                    let mut v = read_entries(&path, show_hidden);
                    sort_entries(&mut v, key, asc);
                    v
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.entries = entries;
                this.selected.clear();
                this.anchor = None;
                this.renaming = None;
                cx.notify();
                this.gen_thumbs(cx);
            });
        })
        .detach();
    }

    /// Generate image thumbnails (sips → cached PNG) off the main thread.
    fn gen_thumbs(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|e| !e.is_dir && is_image(&e.path) && !self.thumbs.contains_key(&e.path))
            .map(|e| e.path.clone())
            .collect();
        if targets.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    let cache = thumb_cache_dir();
                    targets
                        .into_iter()
                        .filter_map(|p| make_thumb(&p, &cache).map(|t| (p, t)))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                for (p, t) in results {
                    this.thumbs.insert(p, t);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn new_tab(&mut self, cx: &mut Context<Self>) {
        self.tabs.push(self.home.clone());
        self.active = self.tabs.len() - 1;
        self.cwd = self.home.clone();
        self.back.clear();
        self.fwd.clear();
        self.reload(cx);
    }

    fn close_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return;
        }
        self.tabs.remove(i);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > i {
            self.active -= 1;
        }
        self.cwd = self.tabs[self.active].clone();
        self.back.clear();
        self.fwd.clear();
        self.reload(cx);
    }

    fn select_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if i >= self.tabs.len() {
            return;
        }
        self.active = i;
        self.cwd = self.tabs[i].clone();
        self.back.clear();
        self.fwd.clear();
        self.reload(cx);
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

    fn go_up(&mut self, cx: &mut Context<Self>) {
        if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
            self.navigate(parent, cx);
        }
    }

    fn open_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(e) = self.entries.get(ix).cloned() else {
            return;
        };
        if e.is_dir {
            self.navigate(e.path, cx);
        } else {
            cx.open_with_system(&e.path);
        }
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<(bool, PathBuf)> = self
            .selected
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| (e.is_dir, e.path.clone()))
            .collect();
        // Open a single folder by navigating; otherwise system-open files.
        if let [(true, dir)] = paths.as_slice() {
            self.navigate(dir.clone(), cx);
        } else {
            for (_, p) in paths {
                cx.open_with_system(&p);
            }
        }
    }

    // ---- selection ----
    fn select_single(&mut self, ix: usize) {
        self.selected.clear();
        self.selected.insert(ix);
        self.anchor = Some(ix);
    }

    fn handle_click(&mut self, ix: usize, cmd: bool, shift: bool) {
        if cmd {
            if !self.selected.remove(&ix) {
                self.selected.insert(ix);
            }
            self.anchor = Some(ix);
        } else if shift {
            if let Some(a) = self.anchor {
                let (lo, hi) = if a <= ix { (a, ix) } else { (ix, a) };
                self.selected.clear();
                for i in lo..=hi {
                    self.selected.insert(i);
                }
            } else {
                self.select_single(ix);
            }
        } else {
            self.select_single(ix);
        }
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| e.path.clone())
            .collect()
    }

    // ---- operations ----
    fn new_folder(&mut self, cx: &mut Context<Self>) {
        let path = unique_path(self.cwd.join("untitled folder"));
        if std::fs::create_dir(&path).is_ok() {
            self.reload(cx);
        }
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        for src in self.selected_paths() {
            let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = src.extension().map(|e| e.to_string_lossy().into_owned());
            let copy_name = match &ext {
                Some(e) => format!("{stem} copy.{e}"),
                None => format!("{stem} copy"),
            };
            let dst = unique_path(self.cwd.join(copy_name));
            let _ = copy_recursive(&src, &dst);
        }
        self.reload(cx);
    }

    fn move_to_trash(&mut self, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            let _ = trash::delete_all(&paths);
            self.reload(cx);
        }
    }

    fn delete_immediately(&mut self, cx: &mut Context<Self>) {
        for p in self.selected_paths() {
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
        self.reload(cx);
    }

    fn copy(&mut self, _cx: &mut Context<Self>) {
        self.clipboard = self.selected_paths();
        self.clip_cut = false;
    }

    fn cut(&mut self, _cx: &mut Context<Self>) {
        self.clipboard = self.selected_paths();
        self.clip_cut = true;
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        for src in self.clipboard.clone() {
            let name = src.file_name().map(|n| n.to_owned()).unwrap_or_default();
            let dst = unique_path(self.cwd.join(name));
            if self.clip_cut {
                if std::fs::rename(&src, &dst).is_err() {
                    if copy_recursive(&src, &dst).is_ok() {
                        let _ = if src.is_dir() {
                            std::fs::remove_dir_all(&src)
                        } else {
                            std::fs::remove_file(&src)
                        };
                    }
                }
            } else {
                let _ = copy_recursive(&src, &dst);
            }
        }
        if self.clip_cut {
            self.clipboard.clear();
            self.clip_cut = false;
        }
        self.reload(cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected = (0..self.entries.len()).collect();
        cx.notify();
    }

    fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        self.reload(cx);
    }

    fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = true;
        }
        sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
        self.selected.clear();
        cx.notify();
    }

    // ---- rename ----
    fn rename_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(&ix) = self.selected.iter().next() else {
            return;
        };
        let Some(entry) = self.entries.get(ix) else {
            return;
        };
        let name = entry.name.to_string();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        cx.subscribe(&input, |this, _input, ev: &InputEvent, cx| match ev {
            InputEvent::PressEnter { .. } => this.rename_commit(cx),
            InputEvent::Blur => this.renaming = None,
            _ => {}
        })
        .detach();
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle);
        self.renaming = Some((ix, input));
        cx.notify();
    }

    fn rename_commit(&mut self, cx: &mut Context<Self>) {
        let Some((ix, input)) = self.renaming.take() else {
            return;
        };
        let new_name = input.read(cx).value().to_string();
        if let Some(entry) = self.entries.get(ix) {
            let new_name = new_name.trim();
            if !new_name.is_empty() && new_name != entry.name.as_ref() {
                let dst = self.cwd.join(new_name);
                let _ = std::fs::rename(&entry.path, &dst);
            }
        }
        self.reload(cx);
    }

    // ---- chrome (toolbar) ----
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
                .child(icon(glyph, 17.0, if enabled { hsl(0x3a3a3c) } else { tertiary() }))
        };
        let cur = self.view;
        let seg = |id: &'static str, glyph: &'static str, mode: ViewMode| {
            let active = cur == mode;
            div()
                .id(id)
                .w(px(34.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .when(active, |el: Stateful<Div>| el.bg(white()))
                .child(icon(glyph, 15.0, if active { label() } else { secondary() }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.view = mode;
                    cx.notify();
                }))
        };
        let view_control = div()
            .flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded(px(7.0))
            .bg(hsl(0xe2e2e4))
            .child(seg("v-icon", "icons/layout-grid.svg", ViewMode::Icon))
            .child(seg("v-list", "icons/list.svg", ViewMode::List))
            .child(seg("v-col", "icons/columns-3.svg", ViewMode::Column))
            .child(seg("v-gal", "icons/image.svg", ViewMode::Gallery));

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
            .w(px(200.0))
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded(px(7.0))
            .bg(hsl(0xededef))
            .child(icon("icons/search.svg", 14.0, tertiary()))
            .child(div().flex_1().child(Input::new(&self.query).appearance(false)));

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
            .on_mouse_down(MouseButton::Left, cx.listener(|t, _, _, _| t.dragging = true))
            .on_mouse_up(MouseButton::Left, cx.listener(|t, _, _, _| t.dragging = false))
            .on_mouse_move(cx.listener(|t, _, window, _| {
                if t.dragging {
                    t.dragging = false;
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

    fn title(&self) -> SharedString {
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Macintosh HD".to_string())
            .into()
    }

    // ---- sidebar ----
    fn render_place(&self, p: &Place, cx: &Context<Self>) -> impl IntoElement {
        let is_tag = p.kind == PlaceKind::Tag;
        let selected = !is_tag && self.cwd == p.path;
        let key = format!("{}-{}", p.name, p.path.display());

        let leading: gpui::AnyElement = if is_tag {
            div().w(px(12.0)).h(px(12.0)).flex_none().rounded_full().bg(p.tint).into_any_element()
        } else {
            icon(p.icon, 17.0, p.tint).into_any_element()
        };

        let np = p.path.clone();
        let main = div()
            .id(SharedString::from(format!("placemain-{key}")))
            .flex_1()
            .flex()
            .items_center()
            .gap_2()
            .min_w(px(0.0))
            .child(leading)
            .child(div().flex_1().text_size(px(13.0)).text_color(label()).truncate().child(p.name.clone()))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !is_tag {
                    this.navigate(np.clone(), cx);
                }
            }));

        let mut row = div()
            .id(SharedString::from(format!("place-{key}")))
            .flex()
            .items_center()
            .gap_2()
            .h(px(28.0))
            .px_2()
            .rounded(px(6.0))
            .when(selected, |el: Stateful<Div>| el.bg(hsl(0xd5d5da)))
            .when(!selected && !is_tag, |el: Stateful<Div>| el.hover(|h| h.bg(hsl(0x00000008))))
            .child(main);

        if p.kind == PlaceKind::Volume {
            let ep = p.path.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("eject-{key}")))
                    .w(px(18.0))
                    .h(px(18.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .hover(|h| h.bg(hsl(0x00000012)))
                    .child(icon("icons/eject.svg", 11.0, secondary()))
                    .on_click(cx.listener(move |_this, _, _, _| {
                        let _ = Command::new("diskutil").arg("eject").arg(&ep).spawn();
                    })),
            );
        }
        row
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

    fn context_menu(menu: PopupMenu, has_selection: bool, can_paste: bool) -> PopupMenu {
        let mut m = menu;
        if has_selection {
            m = m
                .menu("Open", Box::new(OpenItems))
                .menu("Rename", Box::new(RenameItem))
                .menu("Duplicate", Box::new(Duplicate))
                .separator()
                .menu("Copy", Box::new(CopyItems))
                .menu("Cut", Box::new(CutItems));
        }
        if can_paste {
            m = m.menu("Paste Item", Box::new(PasteItems));
        }
        m = m.separator().menu("New Folder", Box::new(NewFolder));
        if has_selection {
            m = m
                .separator()
                .menu("Move to Trash", Box::new(MoveToTrash))
                .menu("Delete Immediately", Box::new(DeleteItem));
        }
        m
    }

    // ---- list ----
    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let q = self.query.read(cx).value().to_lowercase();

        let sort_caret = |key: SortKey| -> Option<Svg> {
            if self.sort_key == key {
                Some(icon(
                    if self.sort_asc { "icons/chevron-up.svg" } else { "icons/chevron-down.svg" },
                    11.0,
                    tertiary(),
                ))
            } else {
                None
            }
        };
        let head = |w: Option<f32>, text: &'static str, key: SortKey, pl: bool| {
            let caret = sort_caret(key);
            let mut cell = div()
                .id(text)
                .flex()
                .items_center()
                .gap_1()
                .when(pl, |el: Stateful<Div>| el.pl_4())
                .when_some(w, |el, w| el.w(px(w)))
                .when(w.is_none(), |el| el.flex_1())
                .child(text)
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)));
            if let Some(c) = caret {
                cell = cell.child(c);
            }
            cell
        };

        let header = div()
            .flex()
            .items_center()
            .h(px(26.0))
            .px_2()
            .border_b_1()
            .border_color(sep())
            .text_size(px(12.0))
            .text_color(secondary())
            .child(head(None, "Name", SortKey::Name, true))
            .child(head(Some(DATE_W), "Date Modified", SortKey::Date, false))
            .child(head(Some(SIZE_W), "Size", SortKey::Size, false))
            .child(head(Some(KIND_W), "Kind", SortKey::Kind, true));

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for (ix, e) in self.entries.iter().enumerate() {
            if !q.is_empty() && !e.name.to_lowercase().contains(&q) {
                continue;
            }
            let selected = self.selected.contains(&ix);
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

            let drag_paths: Vec<PathBuf> = if selected {
                self.selected_paths()
            } else {
                vec![e.path.clone()]
            };
            let drag_count = drag_paths.len();
            let drop_dir = e.path.clone();
            let row_is_dir = e.is_dir;

            let name_cell: gpui::AnyElement = match &self.renaming {
                Some((ri, input)) if *ri == ix => div()
                    .pl(px(6.0))
                    .flex_1()
                    .child(Input::new(input).appearance(true))
                    .into_any_element(),
                _ => div()
                    .pl(px(6.0))
                    .text_color(primary)
                    .truncate()
                    .child(e.name.clone())
                    .into_any_element(),
            };

            rows.push(
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
                            .child(div().w(px(16.0)).flex().justify_center().when(e.is_dir, |el: Div| {
                                el.child(icon(
                                    "icons/chevron-right.svg",
                                    11.0,
                                    if selected { white() } else { tertiary() },
                                ))
                            }))
                            .child(icon(glyph, 16.0, icon_color))
                            .child(name_cell),
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
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, window, cx| {
                            if !this.selected.contains(&ix) {
                                this.select_single(ix);
                            }
                            window.focus(&this.focus);
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        if ev.click_count() >= 2 {
                            this.open_index(ix, cx);
                            return;
                        }
                        let m = ev.modifiers();
                        this.handle_click(ix, m.platform, m.shift);
                        window.focus(&this.focus);
                        cx.notify();
                    }))
                    .on_drag(DraggedPaths(drag_paths), move |_, _, _, cx| {
                        cx.new(|_| DragPreview { count: drag_count })
                    })
                    .when(row_is_dir, |el: Stateful<Div>| {
                        let dd = drop_dir.clone();
                        el.drag_over::<DraggedPaths>(|s, _, _, _| s.bg(hsl(0xcfe5ff)))
                            .on_drop(cx.listener(move |this, p: &DraggedPaths, _, cx| {
                                this.drop_into(dd.clone(), &p.0, cx)
                            }))
                    })
                    .into_any_element(),
            );
        }

        let has_sel = !self.selected.is_empty();
        let can_paste = !self.clipboard.is_empty();
        let is_list = matches!(self.view, ViewMode::List | ViewMode::Column);

        // Icon-grid tiles (Icon & Gallery modes).
        let mut tiles: Vec<gpui::AnyElement> = Vec::new();
        if !is_list {
            for (ix, e) in self.entries.iter().enumerate() {
                if !q.is_empty() && !e.name.to_lowercase().contains(&q) {
                    continue;
                }
                let selected = self.selected.contains(&ix);
                let glyph = if e.is_dir { "icons/folder-fill.svg" } else { "icons/file-fill.svg" };
                let icon_color = if e.is_dir { accent() } else { secondary() };
                let visual: gpui::AnyElement = match self.thumbs.get(&e.path) {
                    Some(t) => img(t.clone())
                        .max_w(px(56.0))
                        .max_h(px(50.0))
                        .rounded(px(3.0))
                        .into_any_element(),
                    None => icon(glyph, 52.0, icon_color).into_any_element(),
                };
                tiles.push(
                    div()
                        .id(("tile", ix))
                        .w(px(104.0))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .px_1()
                        .py_2()
                        .child(div().h(px(52.0)).flex().items_center().child(visual))
                        .child(
                            div()
                                .max_w(px(96.0))
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(4.0))
                                .when(selected, |el: Div| el.bg(sel()))
                                .text_size(px(12.0))
                                .text_center()
                                .truncate()
                                .text_color(if selected { white() } else { label() })
                                .child(e.name.clone()),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, window, cx| {
                                if !this.selected.contains(&ix) {
                                    this.select_single(ix);
                                }
                                window.focus(&this.focus);
                                cx.notify();
                            }),
                        )
                        .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                            if ev.click_count() >= 2 {
                                this.open_index(ix, cx);
                                return;
                            }
                            let m = ev.modifiers();
                            this.handle_click(ix, m.platform, m.shift);
                            window.focus(&this.focus);
                            cx.notify();
                        }))
                        .into_any_element(),
                );
            }
        }

        let content = if is_list {
            div()
                .id("file-list")
                .flex_1()
                .overflow_y_scroll()
                .child(div().v_flex().children(rows))
                .context_menu(move |menu, _, _| Self::context_menu(menu, has_sel, can_paste))
                .into_any_element()
        } else {
            div()
                .id("icon-grid")
                .flex_1()
                .overflow_y_scroll()
                .p_3()
                .child(div().flex().flex_wrap().gap_2().children(tiles))
                .context_menu(move |menu, _, _| Self::context_menu(menu, has_sel, can_paste))
                .into_any_element()
        };

        div()
            .track_focus(&self.focus)
            .key_context("Finder")
            .on_action(cx.listener(|this, _: &NewFolder, _, cx| this.new_folder(cx)))
            .on_action(cx.listener(|this, _: &RenameItem, window, cx| this.rename_start(window, cx)))
            .on_action(cx.listener(|this, _: &Duplicate, _, cx| this.duplicate(cx)))
            .on_action(cx.listener(|this, _: &MoveToTrash, _, cx| this.move_to_trash(cx)))
            .on_action(cx.listener(|this, _: &DeleteItem, _, cx| this.delete_immediately(cx)))
            .on_action(cx.listener(|this, _: &CopyItems, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &CutItems, _, cx| this.cut(cx)))
            .on_action(cx.listener(|this, _: &PasteItems, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &GoUp, _, cx| this.go_up(cx)))
            .on_action(cx.listener(|this, _: &OpenItems, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &ToggleHidden, _, cx| this.toggle_hidden(cx)))
            .on_action(cx.listener(|this, _: &QuickLook, _, cx| this.quick_look(cx)))
            .on_action(cx.listener(|this, _: &GetInfo, _, cx| this.get_info(cx)))
            .on_action(cx.listener(|this, _: &NewTab, _, cx| this.new_tab(cx)))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                let a = this.active;
                this.close_tab(a, cx);
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => {
                        if this.info.take().is_some() {
                            cx.notify();
                        }
                    }
                    "down" => {
                        let next = this.anchor.map(|a| a + 1).unwrap_or(0).min(this.entries.len().saturating_sub(1));
                        this.select_single(next);
                        cx.notify();
                    }
                    "up" => {
                        let prev = this.anchor.map(|a| a.saturating_sub(1)).unwrap_or(0);
                        this.select_single(prev);
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .flex_1()
            .v_flex()
            .bg(list_bg())
            .when(is_list, |el: Div| el.child(header))
            .child(content)
            .child(self.render_path_bar(cx))
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .h(px(30.0))
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(hsl(0xeeeeef))
            .border_b_1()
            .border_color(sep());
        for (i, path) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Macintosh HD".into());
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("tab-{i}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(22.0))
                    .px_2()
                    .rounded(px(5.0))
                    .when(active, |el: Stateful<Div>| el.bg(white()))
                    .when(!active, |el: Stateful<Div>| el.hover(|h| h.bg(hsl(0x00000008))))
                    .child(
                        div()
                            .id(SharedString::from(format!("tabname-{i}")))
                            .text_size(px(12.0))
                            .text_color(label())
                            .child(name)
                            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(i, cx))),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("tabclose-{i}")))
                            .w(px(14.0))
                            .h(px(14.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.0))
                            .text_size(px(12.0))
                            .text_color(secondary())
                            .hover(|h| h.bg(hsl(0x00000014)))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| this.close_tab(i, cx))),
                    ),
            );
        }
        bar.child(div().flex_1()).child(
            div()
                .id("newtab")
                .w(px(22.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .text_size(px(16.0))
                .text_color(secondary())
                .hover(|h| h.bg(hsl(0x00000008)))
                .child("+")
                .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
        )
    }

    fn render_path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut comps: Vec<(String, PathBuf)> = Vec::new();
        let mut acc = PathBuf::new();
        for c in self.cwd.components() {
            acc.push(c.as_os_str());
            let name = match c {
                Component::RootDir => "Macintosh HD".to_string(),
                Component::Normal(s) => s.to_string_lossy().into_owned(),
                _ => continue,
            };
            comps.push((name, acc.clone()));
        }
        let n = comps.len();
        let mut bar = div()
            .h(px(24.0))
            .flex_none()
            .flex()
            .items_center()
            .px_3()
            .gap_1()
            .bg(toolbar_bg())
            .border_t_1()
            .border_color(sep())
            .text_size(px(11.0))
            .text_color(secondary());
        for (i, (name, path)) in comps.into_iter().enumerate() {
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("crumb-{i}")))
                    .px_1()
                    .rounded(px(3.0))
                    .hover(|h| h.bg(hsl(0x00000010)))
                    .child(name)
                    .on_click(cx.listener(move |this, _, _, cx| this.navigate(path.clone(), cx))),
            );
            if i + 1 < n {
                bar = bar.child(icon("icons/chevron-right.svg", 9.0, tertiary()));
            }
        }
        bar
    }

    fn quick_look(&mut self, _cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            let _ = Command::new("qlmanage").arg("-p").args(&paths).spawn();
        }
    }

    fn drop_into(&mut self, dir: PathBuf, paths: &[PathBuf], cx: &mut Context<Self>) {
        for src in paths {
            if src == &dir || src.parent() == Some(dir.as_path()) {
                continue;
            }
            let Some(name) = src.file_name() else { continue };
            let dst = unique_path(dir.join(name));
            if std::fs::rename(src, &dst).is_err() && copy_recursive(src, &dst).is_ok() {
                if src.is_dir() {
                    let _ = std::fs::remove_dir_all(src);
                } else {
                    let _ = std::fs::remove_file(src);
                }
            }
        }
        self.reload(cx);
    }

    fn get_info(&mut self, cx: &mut Context<Self>) {
        self.info = self.selected.iter().next().copied();
        cx.notify();
    }

    fn render_info(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.entries.get(ix) else {
            return div();
        };
        let glyph = if e.is_dir { "icons/folder-fill.svg" } else { "icons/file-fill.svg" };
        let glyph_color = if e.is_dir { accent() } else { secondary() };

        let mut card = div()
            .w(px(300.0))
            .rounded(px(12.0))
            .bg(hsl(0xfbfbfd))
            .border_1()
            .border_color(sep())
            .shadow_lg()
            .child(
                // header bar with close
                div()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .px_2()
                    .child(
                        div()
                            .id("info-close")
                            .w(px(14.0))
                            .h(px(14.0))
                            .rounded_full()
                            .bg(hsl(0xff5f57))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.info = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                // title block
                div()
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .pb_3()
                    .px_4()
                    .border_b_1()
                    .border_color(sep())
                    .child(icon(glyph, 56.0, glyph_color))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .text_center()
                            .child(e.name.clone()),
                    ),
            );

        for (k, v) in file_info(e) {
            card = card.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_4()
                    .py_1()
                    .text_size(px(12.0))
                    .child(
                        div()
                            .w(px(96.0))
                            .flex_none()
                            .text_color(secondary())
                            .text_right()
                            .child(format!("{k}:")),
                    )
                    .child(div().flex_1().text_color(label()).child(v)),
            );
        }

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000026))
            .child(card.pb_3())
    }
}

impl Render for FinderView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let info = self.info;
        let multi = self.tabs.len() > 1;
        div()
            .size_full()
            .relative()
            .v_flex()
            .bg(list_bg())
            .text_color(label())
            .child(self.render_toolbar(cx))
            .when(multi, |el: Div| el.child(self.render_tabs(cx)))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .child(self.render_sidebar(cx))
                    .child(self.render_list(cx)),
            )
            .when_some(info, |el, ix| el.child(self.render_info(ix, cx)))
    }
}

// ---- helpers ----

fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 2..10_000 {
        let name = match &ext {
            Some(e) => format!("{stem} {n}.{e}"),
            None => format!("{stem} {n}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
    } else {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

fn read_entries(dir: &Path, show_hidden: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let path = e.path();
            let md = e.metadata().ok();
            let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size_bytes = if is_dir { 0 } else { md.as_ref().map(|m| m.len()).unwrap_or(0) };
            let mtime = md.as_ref().and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH);
            let size = if is_dir { "--".to_string() } else { human_size(size_bytes) };
            let kind = kind_of(&path, is_dir);
            v.push(Entry {
                name: name.into(),
                path,
                is_dir,
                size: size.into(),
                modified: date_label(mtime).into(),
                kind: kind.into(),
                size_bytes,
                mtime,
            });
        }
    }
    v
}

fn sort_entries(v: &mut [Entry], key: SortKey, asc: bool) {
    v.sort_by(|a, b| {
        let o = match key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Date => a.mtime.cmp(&b.mtime),
            SortKey::Size => a.size_bytes.cmp(&b.size_bytes),
            SortKey::Kind => a.kind.to_lowercase().cmp(&b.kind.to_lowercase()),
        };
        if asc { o } else { o.reverse() }
    });
}

fn is_image(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "bmp" | "tiff" | "tif"
    )
}

fn thumb_cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = PathBuf::from(home).join("Library/Caches/rmac-finder-thumbs");
    std::fs::create_dir_all(&dir).ok();
    dir
}

fn make_thumb(src: &Path, cache: &Path) -> Option<PathBuf> {
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    let out = cache.join(format!("{:x}.png", h.finish()));
    if out.exists() {
        return Some(out);
    }
    let ok = Command::new("sips")
        .args(["-s", "format", "png", "-Z", "96", src.to_str()?, "--out", out.to_str()?])
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok && out.exists() {
        Some(out)
    } else {
        None
    }
}

fn file_info(e: &Entry) -> Vec<(&'static str, String)> {
    let md = std::fs::metadata(&e.path).ok();
    let mut v: Vec<(&'static str, String)> = vec![("Kind", e.kind.to_string())];
    if !e.is_dir {
        v.push(("Size", format!("{} ({} bytes)", e.size, e.size_bytes)));
    }
    if let Some(parent) = e.path.parent() {
        v.push(("Where", parent.display().to_string()));
    }
    if let Some(md) = &md {
        if let Ok(created) = md.created() {
            v.push(("Created", date_label(created)));
        }
    }
    v.push(("Modified", e.modified.to_string()));
    if let Some(md) = &md {
        v.push(("Permissions", perm_string(md.permissions().mode())));
    }
    v
}

fn perm_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6u32, 3, 0] {
        let bits = (mode >> shift) & 0b111;
        s.push(if bits & 0b100 != 0 { 'r' } else { '-' });
        s.push(if bits & 0b010 != 0 { 'w' } else { '-' });
        s.push(if bits & 0b001 != 0 { 'x' } else { '-' });
    }
    s
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

fn kind_of(path: &Path, is_dir: bool) -> String {
    if is_dir {
        return "Folder".to_string();
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
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
    rmac_ui::boot_unified_with_assets(CombinedAssets, 1100.0, 720.0, |window, cx| {
        FinderView::new(window, cx)
    });
}
