//! rmac Finder — a functional macOS-style file manager (list view). See SPEC.md.
//!
//! Beyond the pixel-accurate chrome: multi-selection, file operations
//! (new folder, rename, duplicate, copy/cut/paste, move-to-trash), keyboard
//! shortcuts + right-click context menus, live search, clickable sort headers,
//! hidden-file toggle, and live directory watching.

mod file_ops;
mod pasteboard;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Datelike, Local, Timelike};
use gpui::{
    actions, div, img, prelude::FluentBuilder as _, px, svg, AppContext as _, AssetSource,
    ClickEvent, ClipboardItem, Context, Div, ExternalPaths, FocusHandle, Focusable as _, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, Render, Result, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Svg, Window,
};
use gpui_component::StyledExt as _;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rmac_ui::{InputEvent, InputState, SearchField, TextField};

actions!(
    finder,
    [
        NewFolder,
        RenameItem,
        Duplicate,
        MoveToTrash,
        DeleteItem,
        CopyItems,
        CutItems,
        PasteItems,
        SelectAll,
        GoUp,
        ToggleHidden,
        OpenItems,
        QuickLook,
        GetInfo,
        NewTab,
        CloseTab,
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

enum TransferEvent {
    Progress { processed: usize, total: usize },
    Finished(file_ops::TransferReport),
}

#[derive(Clone)]
struct ActiveTransfer {
    label: SharedString,
    processed: usize,
    total: usize,
    cancel: Arc<AtomicBool>,
    cancelling: bool,
    keep_unfinished_in_clipboard: bool,
}
impl Render for DragPreview {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.count;
        div()
            .px_2()
            .py_0p5()
            .rounded(px(6.0))
            .bg(rmac_ui::mac::accent())
            .text_color(rmac_ui::mac::on_accent())
            .text_size(rmac_ui::text_px(12.0))
            .child(if n == 1 {
                "1 item".to_string()
            } else {
                format!("{n} items")
            })
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
fn list_bg() -> Hsla {
    rmac_ui::mac::list()
}
fn toolbar_bg() -> Hsla {
    rmac_ui::mac::chrome()
}
fn sidebar_bg() -> Hsla {
    rmac_ui::mac::sidebar()
}
fn alt_row() -> Hsla {
    rmac_ui::mac::row_alternate()
}
fn sel() -> Hsla {
    rmac_ui::mac::accent()
}
fn accent() -> Hsla {
    rmac_ui::mac::accent()
}
fn sep() -> Hsla {
    rmac_ui::mac::separator()
}
fn label() -> Hsla {
    rmac_ui::mac::text()
}
fn secondary() -> Hsla {
    rmac_ui::mac::text_secondary()
}
fn tertiary() -> Hsla {
    rmac_ui::mac::text_tertiary()
}
fn drive_gray() -> Hsla {
    rmac_ui::mac::text_secondary()
}
fn white() -> Hsla {
    rmac_ui::mac::on_accent()
}

fn icon(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .w(px(size))
        .h(px(size))
        .text_color(color)
        .flex_none()
}

const SIDEBAR_W: f32 = 190.0;
const DATE_W: f32 = 184.0;
const SIZE_W: f32 = 80.0;
const KIND_W: f32 = 150.0;

fn root_volume_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Macintosh HD"
    } else {
        "Computer"
    }
}

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
    /// Recently-used files from the platform search provider, not a folder.
    Recents,
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

/// One browser tab — its own directory and navigation history.
#[derive(Clone)]
struct Tab {
    cwd: PathBuf,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
}

struct FinderView {
    cwd: PathBuf,
    tabs: Vec<Tab>,
    active: usize,
    home: PathBuf,
    thumbs: std::collections::HashMap<PathBuf, PathBuf>,
    entries: Vec<Entry>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    clipboard: Vec<PathBuf>,
    clip_cut: bool,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<Point<Pixels>>,
    renaming: Option<(usize, gpui::Entity<InputState>)>,
    show_hidden: bool,
    view: ViewMode,
    col_stack: Vec<PathBuf>,
    sort_key: SortKey,
    sort_asc: bool,
    query: gpui::Entity<InputState>,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
    sections: Vec<Section>,
    info: Option<usize>,
    result_title: Option<SharedString>,
    operation_error: Option<SharedString>,
    transfer: Option<ActiveTransfer>,
    /// Free space on the current volume (bytes), read once per navigation.
    free_bytes: Option<u64>,
    dragging: bool,
    focus: FocusHandle,
    watcher: Option<RecommendedWatcher>,
    watched: Option<PathBuf>,
    search_generation: u64,
    search_cancel: Option<Arc<AtomicBool>>,
}

impl Drop for FinderView {
    fn drop(&mut self) {
        if let Some(transfer) = &self.transfer {
            transfer.cancel.store(true, Ordering::Release);
        }
        if let Some(cancel) = &self.search_cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}

impl FinderView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
        let host = home
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_volume_name().to_string());
        let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");

        let p =
            |name: &str, path: PathBuf, icon: &'static str, tint: Hsla, kind: PlaceKind| Place {
                name: name.to_string().into(),
                path,
                icon,
                tint,
                kind,
            };

        // Real mounted volumes.
        let mut locations = vec![
            p(
                &host,
                home.clone(),
                "icons/house.svg",
                drive_gray(),
                PlaceKind::Item,
            ),
            p(
                root_volume_name(),
                "/".into(),
                "icons/hard-drive.svg",
                drive_gray(),
                PlaceKind::Item,
            ),
        ];
        let (mounts, mount_error) = match rmac_mounts::discover() {
            Ok(mounts) => (mounts, None),
            Err(error) => (
                Vec::new(),
                Some(format!("Could not load mounted volumes: {error}").into()),
            ),
        };
        locations.extend(mounts.into_iter().map(|mount| {
            p(
                &mount.name,
                mount.path,
                "icons/hard-drive.svg",
                drive_gray(),
                if mount.ejectable {
                    PlaceKind::Volume
                } else {
                    PlaceKind::Item
                },
            )
        }));

        #[cfg(target_os = "macos")]
        let tag = |name: &str, color: u32| p(name, PathBuf::new(), "", hsl(color), PlaceKind::Tag);
        let mut favorites = vec![p(
            "Recents",
            PathBuf::new(),
            "icons/clock.svg",
            accent(),
            PlaceKind::Recents,
        )];
        #[cfg(target_os = "macos")]
        favorites.push(p(
            "Applications",
            "/Applications".into(),
            "icons/layout-grid.svg",
            accent(),
            PlaceKind::Item,
        ));
        favorites.extend([
            p(
                "Desktop",
                home.join("Desktop"),
                "icons/folder-fill.svg",
                accent(),
                PlaceKind::Item,
            ),
            p(
                "Documents",
                home.join("Documents"),
                "icons/folder-fill.svg",
                accent(),
                PlaceKind::Item,
            ),
            p(
                "Downloads",
                home.join("Downloads"),
                "icons/download.svg",
                accent(),
                PlaceKind::Item,
            ),
        ]);
        let mut sections = vec![Section {
            title: "Favorites".into(),
            places: favorites,
        }];
        // Only show iCloud Drive when the real CloudDocs folder exists.
        if icloud.is_dir() {
            sections.push(Section {
                title: "iCloud".into(),
                places: vec![p(
                    "iCloud Drive",
                    icloud,
                    "icons/cloud.svg",
                    accent(),
                    PlaceKind::Item,
                )],
            });
        }
        sections.push(Section {
            title: "Locations".into(),
            places: locations,
        });
        #[cfg(target_os = "macos")]
        sections.push(Section {
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
        });

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
        // Pressing Return runs a recursive Spotlight search of the whole folder tree.
        cx.subscribe(&query, |this, _input, ev: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.recursive_search(cx);
            }
        })
        .detach();

        // The bounded channel bridges notify's callback thread to GPUI. A
        // capacity of one coalesces filesystem-event bursts into one reload.
        let (fs_events, fs_event_rx) = async_channel::bounded(1);
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = fs_events.try_send(());
            }
        })
        .ok();

        let focus = cx.focus_handle();
        window.focus(&focus);

        let mut view = Self {
            cwd: home.clone(),
            tabs: vec![Tab {
                cwd: home.clone(),
                back: Vec::new(),
                fwd: Vec::new(),
            }],
            active: 0,
            home: home.clone(),
            thumbs: std::collections::HashMap::new(),
            entries: Vec::new(),
            selected: BTreeSet::new(),
            menu_at: None,
            anchor: None,
            clipboard: Vec::new(),
            clip_cut: false,
            renaming: None,
            show_hidden: false,
            view: ViewMode::List,
            col_stack: vec![home.clone()],
            sort_key: SortKey::Name,
            sort_asc: true,
            query,
            back: Vec::new(),
            fwd: Vec::new(),
            sections,
            info: None,
            result_title: None,
            operation_error: mount_error,
            transfer: None,
            free_bytes: None,
            dragging: false,
            focus,
            watcher,
            watched: None,
            search_generation: 0,
            search_cancel: None,
        };
        view.reload(cx);

        // Live directory watching → reload on filesystem changes.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while fs_event_rx.recv().await.is_ok() {
                // FSEvents can deliver a rapid sequence for one logical
                // operation. Wait for 200 ms of quiet, but cap continuous
                // churn at two seconds so the view cannot remain stale.
                for _ in 0..10 {
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    if fs_event_rx.try_recv().is_err() {
                        break;
                    }
                }
                while fs_event_rx.try_recv().is_ok() {}
                if this
                    .update(cx, |this: &mut FinderView, cx| this.reload_after_event(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_inner(cx, true);
    }

    /// Refresh after a watcher event without spawning `df`; free space changes
    /// slowly and is refreshed on navigation and explicit file operations.
    fn reload_after_event(&mut self, cx: &mut Context<Self>) {
        self.reload_inner(cx, false);
    }

    fn reload_inner(&mut self, cx: &mut Context<Self>, refresh_free_space: bool) {
        self.cancel_search();
        self.result_title = None;
        self.col_stack = vec![self.cwd.clone()];
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.cwd = self.cwd.clone();
        }
        // Reconfigure the watcher only after navigation. Re-watching the same
        // directory in response to its own event can create a reload storm.
        if self.watched.as_ref() != Some(&self.cwd) {
            if let Some(w) = self.watcher.as_mut() {
                if let Some(old) = self.watched.take() {
                    let _ = w.unwatch(&old);
                }
                if w.watch(&self.cwd, RecursiveMode::NonRecursive).is_ok() {
                    self.watched = Some(self.cwd.clone());
                }
            }
        }

        let path = self.cwd.clone();
        // Compared on completion so a slow read for a directory we've since
        // navigated away from doesn't clobber the current listing.
        let read_path = path.clone();
        let show_hidden = self.show_hidden;
        let key = self.sort_key;
        let asc = self.sort_asc;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (entries, free) = cx
                .background_executor()
                .spawn(async move {
                    let mut v = read_entries(&path, show_hidden);
                    sort_entries(&mut v, key, asc);
                    let free = refresh_free_space.then(|| free_space(&path));
                    (v, free)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Drop stale results from a superseded navigation.
                if this.cwd != read_path {
                    return;
                }
                this.entries = entries;
                let entry_paths = this
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect::<BTreeSet<_>>();
                this.thumbs.retain(|source, thumbnail| {
                    entry_paths.contains(source) && rmac_thumbnails::is_current(source, thumbnail)
                });
                if let Some(free) = free {
                    this.free_bytes = free;
                }
                this.selected.clear();
                this.anchor = None;
                this.renaming = None;
                cx.notify();
                this.gen_thumbs(cx);
            });
        })
        .detach();
    }

    /// Generate platform thumbnails into the persistent cache off the main thread.
    fn gen_thumbs(&mut self, cx: &mut Context<Self>) {
        let directory = self.cwd.clone();
        let targets: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| {
                !entry.is_dir
                    && rmac_thumbnails::is_supported(&entry.path)
                    && !self.thumbs.get(&entry.path).is_some_and(|thumbnail| {
                        rmac_thumbnails::is_current(&entry.path, thumbnail)
                    })
            })
            .map(|e| e.path.clone())
            .collect();
        if targets.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    let mut generated = Vec::new();
                    let mut first_error = None;
                    let mut failure_count = 0;
                    for path in targets {
                        match rmac_thumbnails::generate(&path) {
                            Ok(thumbnail) => generated.push((path, thumbnail)),
                            Err(error) => {
                                failure_count += 1;
                                first_error.get_or_insert(error);
                            }
                        }
                    }
                    (generated, first_error, failure_count)
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                for (p, t) in results.0 {
                    if this.entries.iter().any(|entry| entry.path == p)
                        && rmac_thumbnails::is_current(&p, &t)
                    {
                        this.thumbs.insert(p, t);
                    }
                }
                if this.cwd == directory && this.operation_error.is_none() {
                    this.operation_error = results.1.map(|error| {
                        if results.2 == 1 {
                            format!("Could not generate thumbnail: {error}").into()
                        } else {
                            format!(
                                "Could not generate thumbnail: {error} (and {} more failures)",
                                results.2 - 1
                            )
                            .into()
                        }
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Persist the active tab's live navigation state into the tab list.
    fn save_tab(&mut self) {
        if let Some(t) = self.tabs.get_mut(self.active) {
            t.cwd = self.cwd.clone();
            t.back = self.back.clone();
            t.fwd = self.fwd.clone();
        }
    }

    /// Load tab `i`'s state into the live fields.
    fn load_tab(&mut self, i: usize) {
        if let Some(t) = self.tabs.get(i) {
            self.cwd = t.cwd.clone();
            self.back = t.back.clone();
            self.fwd = t.fwd.clone();
        }
    }

    fn new_tab(&mut self, cx: &mut Context<Self>) {
        self.save_tab();
        self.tabs.push(Tab {
            cwd: self.home.clone(),
            back: Vec::new(),
            fwd: Vec::new(),
        });
        self.active = self.tabs.len() - 1;
        self.load_tab(self.active);
        self.reload(cx);
    }

    fn close_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return;
        }
        let was_active = i == self.active;
        if was_active {
            self.save_tab();
        }
        self.tabs.remove(i);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > i {
            self.active -= 1;
        }
        if was_active {
            self.load_tab(self.active);
            self.reload(cx);
        } else {
            cx.notify();
        }
    }

    fn select_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if i >= self.tabs.len() || i == self.active {
            return;
        }
        self.save_tab();
        self.active = i;
        self.load_tab(i);
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

    fn record_operation_failures(
        &mut self,
        failures: Vec<file_ops::Failure>,
        cx: &mut Context<Self>,
    ) {
        self.operation_error = failures.first().map(|first| {
            if failures.len() == 1 {
                first.to_string().into()
            } else {
                format!("{} (and {} more failures)", first, failures.len() - 1).into()
            }
        });
        cx.notify();
    }

    fn finish_file_operations(&mut self, failures: Vec<file_ops::Failure>, cx: &mut Context<Self>) {
        self.record_operation_failures(failures, cx);
        self.reload(cx);
    }

    fn begin_search(&mut self) -> (u64, Arc<AtomicBool>) {
        self.cancel_search();
        self.search_generation = self.search_generation.wrapping_add(1);
        self.operation_error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());
        (self.search_generation, cancel)
    }

    fn cancel_search(&mut self) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.search_generation = self.search_generation.wrapping_add(1);
    }

    fn eject_volume(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let unmount_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { rmac_mounts::unmount(&unmount_path) })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                match result {
                    Ok(()) => {
                        this.operation_error = None;
                        for section in &mut this.sections {
                            section.places.retain(|place| place.path != path);
                        }
                        if this.cwd.starts_with(&path) {
                            this.navigate(this.home.clone(), cx);
                            return;
                        }
                    }
                    Err(error) => {
                        this.operation_error =
                            Some(format!("Could not eject volume: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn block_mutation_during_transfer(&mut self, cx: &mut Context<Self>) -> bool {
        if self.transfer.is_none() {
            return false;
        }
        self.operation_error = Some("Wait for the current file operation to finish".into());
        cx.notify();
        true
    }

    fn start_transfer(
        &mut self,
        label: &'static str,
        tasks: Vec<file_ops::TransferTask>,
        keep_unfinished_in_clipboard: bool,
        cx: &mut Context<Self>,
    ) {
        if tasks.is_empty() {
            return;
        }
        if self.block_mutation_during_transfer(cx) {
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.operation_error = None;
        self.transfer = Some(ActiveTransfer {
            label: label.into(),
            processed: 0,
            total: tasks.len(),
            cancel: cancel.clone(),
            cancelling: false,
            keep_unfinished_in_clipboard,
        });
        cx.notify();

        let (events, event_rx) = async_channel::unbounded();
        cx.background_executor()
            .spawn(async move {
                let progress_events = events.clone();
                let report = file_ops::execute_transfers(
                    &file_ops::RealFileSystem,
                    &tasks,
                    &cancel,
                    move |processed, total| {
                        let _ = progress_events
                            .send_blocking(TransferEvent::Progress { processed, total });
                    },
                );
                let _ = events.send_blocking(TransferEvent::Finished(report));
            })
            .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = event_rx.recv().await {
                let finished = matches!(event, TransferEvent::Finished(_));
                if this
                    .update(cx, |this: &mut FinderView, cx| match event {
                        TransferEvent::Progress { processed, total } => {
                            if let Some(transfer) = this.transfer.as_mut() {
                                transfer.processed = processed;
                                transfer.total = total;
                            }
                            cx.notify();
                        }
                        TransferEvent::Finished(report) => {
                            let keep_clipboard = this
                                .transfer
                                .as_ref()
                                .is_some_and(|transfer| transfer.keep_unfinished_in_clipboard);
                            this.transfer = None;
                            if keep_clipboard {
                                this.clipboard = report.unfinished_moves;
                                this.clip_cut = !this.clipboard.is_empty();
                                if this.clip_cut {
                                    this.write_clip_text(cx);
                                }
                            }
                            this.record_operation_failures(report.failures, cx);
                            this.reload(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
                if finished {
                    break;
                }
            }
        })
        .detach();
    }

    fn cancel_transfer(&mut self, cx: &mut Context<Self>) {
        if let Some(transfer) = self.transfer.as_mut() {
            transfer.cancel.store(true, Ordering::Release);
            transfer.cancelling = true;
            cx.notify();
        }
    }

    // ---- operations ----
    fn new_folder(&mut self, cx: &mut Context<Self>) {
        let path = unique_path(self.cwd.join("untitled folder"));
        let failures = file_ops::create_folder(&file_ops::RealFileSystem, &path)
            .err()
            .into_iter()
            .collect();
        self.finish_file_operations(failures, cx);
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        let mut tasks = Vec::new();
        let mut destinations = BTreeSet::new();
        for src in self.selected_paths() {
            let stem = src
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ext = src.extension().map(|e| e.to_string_lossy().into_owned());
            let copy_name = match &ext {
                Some(e) => format!("{stem} copy.{e}"),
                None => format!("{stem} copy"),
            };
            let dst = unique_path_avoiding(self.cwd.join(copy_name), &destinations);
            destinations.insert(dst.clone());
            tasks.push(file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: src,
                destination: dst,
            });
        }
        self.start_transfer("Duplicating", tasks, false, cx);
    }

    fn move_to_trash(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let paths = self.selected_paths();
        if !paths.is_empty() {
            let failures = trash::delete_all(&paths)
                .err()
                .map(|error| {
                    file_ops::Failure::message(
                        file_ops::Operation::Trash,
                        &paths[0],
                        None,
                        error.to_string(),
                    )
                })
                .into_iter()
                .collect();
            self.finish_file_operations(failures, cx);
        }
    }

    fn delete_immediately(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let mut failures = Vec::new();
        for p in self.selected_paths() {
            if let Err(failure) = file_ops::delete(&file_ops::RealFileSystem, &p) {
                failures.push(failure);
            }
        }
        self.finish_file_operations(failures, cx);
    }

    fn write_clip_text(&self, cx: &mut Context<Self>) {
        // Native pasteboard: real file:// URLs so the system Finder (and any
        // app) can paste the copied items.
        pasteboard::write_file_urls(&self.clipboard);
        // Plain-text fallback: newline-joined paths, for the text bridge.
        let text = self
            .clipboard
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        self.clipboard = self.selected_paths();
        self.clip_cut = false;
        self.write_clip_text(cx);
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        self.clipboard = self.selected_paths();
        self.clip_cut = true;
        self.write_clip_text(cx);
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        // Nothing copied inside rmac Finder — pull from the system pasteboard so
        // items copied in the real Finder (or elsewhere) can be pasted here.
        if self.clipboard.is_empty() {
            // Prefer the native file:// URLs; fall back to the text bridge.
            let mut paths = pasteboard::read_file_urls();
            paths.retain(|p| p.exists());
            if paths.is_empty() {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    paths = text
                        .lines()
                        .map(PathBuf::from)
                        .filter(|p| p.exists())
                        .collect();
                }
            }
            if !paths.is_empty() {
                self.clipboard = paths;
                self.clip_cut = false;
            }
        }
        let kind = if self.clip_cut {
            file_ops::TransferKind::Move
        } else {
            file_ops::TransferKind::Copy
        };
        let mut tasks = Vec::new();
        let mut destinations = BTreeSet::new();
        for src in self.clipboard.clone() {
            let name = src.file_name().map(|n| n.to_owned()).unwrap_or_default();
            let dst = unique_path_avoiding(self.cwd.join(name), &destinations);
            destinations.insert(dst.clone());
            tasks.push(file_ops::TransferTask {
                kind,
                source: src,
                destination: dst,
            });
        }
        self.start_transfer(
            if self.clip_cut { "Moving" } else { "Copying" },
            tasks,
            self.clip_cut,
            cx,
        );
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        // Only the entries currently visible (after the search filter).
        let q = self.query.read(cx).value().to_lowercase();
        self.selected = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
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
        if self.block_mutation_during_transfer(cx) {
            return;
        }
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
                // Don't clobber an existing file/folder at the target name.
                if !dst.exists() {
                    let failures = file_ops::rename(&file_ops::RealFileSystem, &entry.path, &dst)
                        .err()
                        .into_iter()
                        .collect();
                    self.record_operation_failures(failures, cx);
                } else {
                    self.record_operation_failures(
                        vec![file_ops::Failure::message(
                            file_ops::Operation::Rename,
                            &entry.path,
                            Some(&dst),
                            "an item with that name already exists",
                        )],
                        cx,
                    );
                }
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
                .when(enabled, |el: Stateful<Div>| {
                    el.hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
                })
                .child(icon(
                    glyph,
                    17.0,
                    if enabled { label() } else { tertiary() },
                ))
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
                .when(active, |el: Stateful<Div>| el.bg(rmac_ui::mac::raised()))
                .child(icon(
                    glyph,
                    15.0,
                    if active { label() } else { secondary() },
                ))
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
            .bg(rmac_ui::mac::control_fill())
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
            .bg(rmac_ui::mac::control_fill())
            .child(icon("icons/search.svg", 14.0, tertiary()))
            .child(
                div()
                    .flex_1()
                    .child(SearchField::new(&self.query).appearance(false)),
            );

        div()
            .id("toolbar")
            .h(px(52.0))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .pl(px(13.0))
            .pr_3()
            .bg(toolbar_bg())
            .border_b_1()
            .border_color(sep())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.dragging = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.dragging = false),
            )
            .on_mouse_move(cx.listener(|t, _, window, _| {
                if t.dragging {
                    t.dragging = false;
                    window.start_window_move();
                }
            }))
            .child(div().mr_1().child(rmac_ui::traffic_lights()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        nav("back", "icons/chevron-left.svg", !self.back.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
                    )
                    .child(
                        nav("fwd", "icons/chevron-right.svg", !self.fwd.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
                    ),
            )
            .child(
                div()
                    .pl_1()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(label())
                    .child(self.title()),
            )
            .child(div().flex_1())
            .child(view_control)
            .child(tool("icons/share-2.svg"))
            .child(tool("icons/tag.svg"))
            // The ⋯ button opens the item context menu (anchored below itself).
            .child(
                div()
                    .id("more")
                    .w(px(30.0))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
                    .child(icon("icons/ellipsis.svg", 16.0, secondary()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.menu_at = Some(ev.position);
                            cx.notify();
                        }),
                    ),
            )
            .child(search)
    }

    fn title(&self) -> SharedString {
        if let Some(rt) = &self.result_title {
            return rt.clone();
        }
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_volume_name().to_string())
            .into()
    }

    // ---- sidebar ----
    fn render_place(&self, p: &Place, cx: &Context<Self>) -> impl IntoElement {
        let is_tag = p.kind == PlaceKind::Tag;
        let selected = !is_tag && self.cwd == p.path;
        let key = format!("{}-{}", p.name, p.path.display());

        let leading: gpui::AnyElement = if is_tag {
            div()
                .w(px(12.0))
                .h(px(12.0))
                .flex_none()
                .rounded_full()
                .bg(p.tint)
                .into_any_element()
        } else {
            icon(p.icon, 17.0, p.tint).into_any_element()
        };

        let np = p.path.clone();
        let tag_name = p.name.clone();
        let kind = p.kind;
        let main = div()
            .id(SharedString::from(format!("placemain-{key}")))
            .flex_1()
            .flex()
            .items_center()
            .gap_2()
            .min_w(px(0.0))
            .child(leading)
            .child(
                div()
                    .flex_1()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(label())
                    .truncate()
                    .child(p.name.clone()),
            )
            .on_click(cx.listener(move |this, _, _, cx| match kind {
                PlaceKind::Tag => this.tag_click(tag_name.clone(), cx),
                PlaceKind::Recents => this.recents_click(cx),
                _ => this.navigate(np.clone(), cx),
            }));

        let mut row = div()
            .id(SharedString::from(format!("place-{key}")))
            .flex()
            .items_center()
            .gap_2()
            .h(px(28.0))
            .px_2()
            .rounded(px(6.0))
            .when(selected, |el: Stateful<Div>| {
                el.bg(rmac_ui::mac::sidebar_selection())
            })
            .when(!selected && !is_tag, |el: Stateful<Div>| {
                el.hover(|h| h.bg(rmac_ui::mac::hover()))
            })
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
                    .hover(|h| h.bg(rmac_ui::mac::hover()))
                    .child(icon("icons/eject.svg", 11.0, secondary()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.eject_volume(ep.clone(), cx);
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
                    .text_size(rmac_ui::text_px(11.0))
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

    fn build_context_menu(
        pos: Point<Pixels>,
        has_selection: bool,
        can_paste: bool,
    ) -> rmac_ui::ContextMenu {
        let mut m = rmac_ui::ContextMenu::new(pos);
        if has_selection {
            m = m
                .item("Open", Box::new(OpenItems))
                .item("Rename", Box::new(RenameItem))
                .item("Duplicate", Box::new(Duplicate))
                .separator()
                .item("Copy", Box::new(CopyItems))
                .item("Cut", Box::new(CutItems));
        }
        if can_paste {
            m = m.item("Paste Item", Box::new(PasteItems));
        }
        m = m.separator().item("New Folder", Box::new(NewFolder));
        if has_selection {
            m = m
                .separator()
                .item("Move to Trash", Box::new(MoveToTrash))
                .danger_item("Delete Immediately", Box::new(DeleteItem));
        }
        m
    }

    // ---- list ----
    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let q = self.query.read(cx).value().to_lowercase();

        let sort_caret = |key: SortKey| -> Option<Svg> {
            if self.sort_key == key {
                Some(icon(
                    if self.sort_asc {
                        "icons/chevron-up.svg"
                    } else {
                        "icons/chevron-down.svg"
                    },
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
            .text_size(rmac_ui::text_px(12.0))
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
            let glyph = if e.is_dir {
                "icons/folder-fill.svg"
            } else {
                "icons/file-fill.svg"
            };
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
                    .child(TextField::new(input).appearance(true))
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
                    .text_size(rmac_ui::text_px(13.0))
                    .when(selected, |el: Stateful<Div>| el.bg(sel()))
                    .when(!selected && ix % 2 == 1, |el: Stateful<Div>| {
                        el.bg(alt_row())
                    })
                    .when(!selected, |el: Stateful<Div>| {
                        el.hover(|h| h.bg(rmac_ui::mac::hover()))
                    })
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .min_w(px(0.0))
                            .child(div().w(px(16.0)).flex().justify_center().when(
                                e.is_dir,
                                |el: Div| {
                                    el.child(icon(
                                        "icons/chevron-right.svg",
                                        11.0,
                                        if selected { white() } else { tertiary() },
                                    ))
                                },
                            ))
                            .child(icon(glyph, 16.0, icon_color))
                            .child(name_cell),
                    )
                    .child(
                        div()
                            .w(px(DATE_W))
                            .text_color(sub)
                            .child(e.modified.clone()),
                    )
                    .child(
                        div()
                            .w(px(SIZE_W))
                            .flex()
                            .justify_end()
                            .text_color(sub)
                            .child(e.size.clone()),
                    )
                    .child(
                        div()
                            .w(px(KIND_W))
                            .pl_3()
                            .text_color(sub)
                            .truncate()
                            .child(e.kind.clone()),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            if !this.selected.contains(&ix) {
                                this.select_single(ix);
                            }
                            window.focus(&this.focus);
                            this.menu_at = Some(ev.position);
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
                        el.drag_over::<DraggedPaths>(|s, _, _, _| {
                            s.bg(rmac_ui::mac::accent_subtle())
                        })
                        .on_drop(cx.listener(
                            move |this, p: &DraggedPaths, _, cx| {
                                this.drop_into(dd.clone(), &p.0, cx)
                            },
                        ))
                    })
                    .into_any_element(),
            );
        }

        let show_list = self.view == ViewMode::List;
        let show_icons = matches!(self.view, ViewMode::Icon | ViewMode::Gallery);

        // Icon-grid tiles (Icon & Gallery modes).
        let mut tiles: Vec<gpui::AnyElement> = Vec::new();
        if show_icons {
            for (ix, e) in self.entries.iter().enumerate() {
                if !q.is_empty() && !e.name.to_lowercase().contains(&q) {
                    continue;
                }
                let selected = self.selected.contains(&ix);
                let glyph = if e.is_dir {
                    "icons/folder-fill.svg"
                } else {
                    "icons/file-fill.svg"
                };
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
                                .text_size(rmac_ui::text_px(12.0))
                                .text_center()
                                .truncate()
                                .text_color(if selected { white() } else { label() })
                                .child(e.name.clone()),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                if !this.selected.contains(&ix) {
                                    this.select_single(ix);
                                }
                                window.focus(&this.focus);
                                this.menu_at = Some(ev.position);
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

        let content = match self.view {
            ViewMode::List => div()
                .id("file-list")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(div().v_flex().children(rows))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.menu_at = Some(ev.position);
                        cx.notify();
                    }),
                )
                .into_any_element(),
            ViewMode::Column => self.render_columns(cx).into_any_element(),
            _ => div()
                .id("icon-grid")
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .p_3()
                .child(div().flex().flex_wrap().gap_2().children(tiles))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.menu_at = Some(ev.position);
                        cx.notify();
                    }),
                )
                .into_any_element(),
        };

        div()
            .track_focus(&self.focus)
            .key_context("Finder")
            .on_action(cx.listener(|this, _: &NewFolder, _, cx| this.new_folder(cx)))
            .on_action(
                cx.listener(|this, _: &RenameItem, window, cx| this.rename_start(window, cx)),
            )
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
                        let next = this
                            .anchor
                            .map(|a| a + 1)
                            .unwrap_or(0)
                            .min(this.entries.len().saturating_sub(1));
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
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rmac_ui::mac::accent_subtle()))
            .on_drop(cx.listener(|this, ep: &ExternalPaths, _, cx| {
                this.drop_external(ep.paths().to_vec(), cx)
            }))
            .flex_1()
            .v_flex()
            .overflow_hidden()
            .bg(list_bg())
            .when(show_list, |el: Div| el.child(header))
            .child(content)
            .child(self.render_path_bar(cx))
            .child(self.render_status_bar())
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .h(px(30.0))
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(rmac_ui::mac::chrome())
            .border_b_1()
            .border_color(sep());
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let name = tab
                .cwd
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
                    .when(active, |el: Stateful<Div>| el.bg(rmac_ui::mac::raised()))
                    .when(!active, |el: Stateful<Div>| {
                        el.hover(|h| h.bg(rmac_ui::mac::hover()))
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("tabname-{i}")))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(label())
                            .child(name)
                            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(i, cx))),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("tabclose-{i}")))
                            .w(px(18.0))
                            .h(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| this.close_tab(i, cx))),
                    ),
            );
        }
        bar.child(div().flex_1()).child(
            div()
                .id("newtab")
                .w(px(26.0))
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .text_size(rmac_ui::text_px(16.0))
                .text_color(secondary())
                .hover(|h| h.bg(rmac_ui::mac::hover()))
                .child("+")
                .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
        )
    }

    fn render_columns(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .id("columns")
            .flex_1()
            .flex()
            .overflow_x_scroll()
            .bg(list_bg());
        for (ci, dir) in self.col_stack.iter().enumerate() {
            let mut entries = read_entries(dir, self.show_hidden);
            sort_entries(&mut entries, SortKey::Name, true);
            let selected_child = self.col_stack.get(ci + 1).cloned();
            let mut col = div()
                .id(SharedString::from(format!("col-{ci}")))
                .w(px(232.0))
                .h_full()
                .flex_none()
                .border_r_1()
                .border_color(sep())
                .overflow_y_scroll()
                .v_flex()
                .py_1();
            for e in entries {
                let is_sel = selected_child.as_ref() == Some(&e.path);
                let ep = e.path.clone();
                let is_dir = e.is_dir;
                let glyph = if is_dir {
                    "icons/folder-fill.svg"
                } else {
                    "icons/file-fill.svg"
                };
                let icol = if is_sel {
                    white()
                } else if is_dir {
                    accent()
                } else {
                    secondary()
                };
                col = col.child(
                    div()
                        .id(SharedString::from(format!("colrow-{ci}-{}", e.name)))
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(22.0))
                        .mx_1()
                        .px_2()
                        .rounded(px(5.0))
                        .when(is_sel, |el: Stateful<Div>| el.bg(sel()))
                        .when(!is_sel, |el: Stateful<Div>| {
                            el.hover(|h| h.bg(rmac_ui::mac::hover()))
                        })
                        .child(icon(glyph, 15.0, icol))
                        .child(
                            div()
                                .flex_1()
                                .text_size(rmac_ui::text_px(13.0))
                                .truncate()
                                .text_color(if is_sel { white() } else { label() })
                                .child(e.name.clone()),
                        )
                        .when(is_dir, |el: Stateful<Div>| {
                            el.child(icon(
                                "icons/chevron-right.svg",
                                10.0,
                                if is_sel { white() } else { tertiary() },
                            ))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if is_dir {
                                this.col_stack.truncate(ci + 1);
                                this.col_stack.push(ep.clone());
                                cx.notify();
                            } else {
                                cx.open_with_system(&ep);
                            }
                        })),
                );
            }
            row = row.child(col);
        }
        row
    }

    /// macOS-style status bar: item / selection count + free space available.
    fn render_status_bar(&self) -> impl IntoElement {
        let n = self.entries.len();
        let sel = self.selected.len();
        let count = if sel > 0 {
            format!("{sel} of {n} selected")
        } else {
            format!("{n} item{}", if n == 1 { "" } else { "s" })
        };
        let free = self
            .free_bytes
            .map(|b| format!("{} available", human_size(b)))
            .unwrap_or_default();
        div()
            .h(px(22.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(toolbar_bg())
            .border_t_1()
            .border_color(sep())
            .text_size(rmac_ui::text_px(11.0))
            .text_color(secondary())
            .child(count)
            .when(!free.is_empty(), |el| {
                el.child(div().text_color(tertiary()).child("•"))
                    .child(free)
            })
    }

    fn render_path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut comps: Vec<(String, PathBuf)> = Vec::new();
        let mut acc = PathBuf::new();
        for c in self.cwd.components() {
            acc.push(c.as_os_str());
            let name = match c {
                Component::RootDir => root_volume_name().to_string(),
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
            .text_size(rmac_ui::text_px(11.0))
            .text_color(secondary());
        for (i, (name, path)) in comps.into_iter().enumerate() {
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("crumb-{i}")))
                    .px_1()
                    .rounded(px(3.0))
                    .hover(|h| h.bg(rmac_ui::mac::hover()))
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
        let mut tasks = Vec::new();
        let mut destinations = BTreeSet::new();
        for src in paths {
            if src == &dir || src.parent() == Some(dir.as_path()) {
                continue;
            }
            let Some(name) = src.file_name() else {
                continue;
            };
            let dst = unique_path_avoiding(dir.join(name), &destinations);
            destinations.insert(dst.clone());
            tasks.push(file_ops::TransferTask {
                kind: file_ops::TransferKind::Move,
                source: src.clone(),
                destination: dst,
            });
        }
        self.start_transfer("Moving", tasks, false, cx);
    }

    /// Files dropped from another app (Finder, etc.) → copy into the current dir.
    fn drop_external(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut tasks = Vec::new();
        let mut destinations = BTreeSet::new();
        for src in paths {
            if let Some(name) = src.file_name() {
                let dst = unique_path_avoiding(self.cwd.join(name), &destinations);
                destinations.insert(dst.clone());
                tasks.push(file_ops::TransferTask {
                    kind: file_ops::TransferKind::Copy,
                    source: src,
                    destination: dst,
                });
            }
        }
        self.start_transfer("Copying", tasks, false, cx);
    }

    fn get_info(&mut self, cx: &mut Context<Self>) {
        self.info = self.selected.iter().next().copied();
        cx.notify();
    }

    /// Recursive platform search of the current folder tree (Return in the search box).
    fn recursive_search(&mut self, cx: &mut Context<Self>) {
        let q = self.query.read(cx).value().to_string();
        if q.trim().is_empty() {
            return;
        }
        let cwd = self.cwd.clone();
        let title: SharedString = format!("Search: {q}").into();
        let key = self.sort_key;
        let asc = self.sort_asc;
        let include_hidden = self.show_hidden;
        let (generation, cancel) = self.begin_search();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut options = rmac_search::Options::new(&cancel);
                    options.include_hidden = include_hidden;
                    let mut v = rmac_search::filenames(&cwd, &q, options)?
                        .into_iter()
                        .filter_map(|path| entry_for(&path))
                        .collect::<Vec<_>>();
                    sort_entries(&mut v, key, asc);
                    Ok::<_, rmac_search::Error>(v)
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_cancel = None;
                match result {
                    Ok(entries) => {
                        this.entries = entries;
                        this.result_title = Some(title);
                        this.selected.clear();
                        this.anchor = None;
                    }
                    Err(rmac_search::Error::Cancelled) => {}
                    Err(error) => this.operation_error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn tag_click(&mut self, name: SharedString, cx: &mut Context<Self>) {
        let title: SharedString = format!("Tag: {name}").into();
        let key = self.sort_key;
        let asc = self.sort_asc;
        let (generation, cancel) = self.begin_search();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut v = rmac_search::tagged(&name, rmac_search::Options::new(&cancel))?
                        .into_iter()
                        .filter_map(|path| entry_for(&path))
                        .collect::<Vec<_>>();
                    sort_entries(&mut v, key, asc);
                    Ok::<_, rmac_search::Error>(v)
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_cancel = None;
                match result {
                    Ok(entries) => {
                        this.entries = entries;
                        this.result_title = Some(title);
                        this.selected.clear();
                        this.anchor = None;
                    }
                    Err(rmac_search::Error::Cancelled) => {}
                    Err(error) => this.operation_error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Show real recently-used files from Spotlight or the XDG bookmark store.
    fn recents_click(&mut self, cx: &mut Context<Self>) {
        let (generation, cancel) = self.begin_search();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut v: Vec<(Entry, std::time::SystemTime)> =
                        rmac_search::recents(rmac_search::Options::new(&cancel))?
                            .into_iter()
                            .filter_map(|path| {
                                let when = std::fs::metadata(&path)
                                    .and_then(|metadata| metadata.modified())
                                    .unwrap_or(std::time::UNIX_EPOCH);
                                entry_for(&path).map(|entry| (entry, when))
                            })
                            .collect();
                    // Most recently modified first, capped so the list stays manageable.
                    v.sort_by(|a, b| b.1.cmp(&a.1));
                    v.truncate(200);
                    Ok::<_, rmac_search::Error>(
                        v.into_iter().map(|(entry, _)| entry).collect::<Vec<_>>(),
                    )
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_cancel = None;
                match result {
                    Ok(entries) => {
                        this.entries = entries;
                        this.result_title = Some("Recents".into());
                        this.selected.clear();
                        this.anchor = None;
                    }
                    Err(rmac_search::Error::Cancelled) => {}
                    Err(error) => this.operation_error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_info(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.entries.get(ix) else {
            return div();
        };
        let glyph = if e.is_dir {
            "icons/folder-fill.svg"
        } else {
            "icons/file-fill.svg"
        };
        let glyph_color = if e.is_dir { accent() } else { secondary() };

        let mut card = div()
            .w(px(300.0))
            .rounded(px(12.0))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(sep())
            .shadow_lg()
            .child(
                // header bar with close
                div().h(px(28.0)).flex().items_center().px_2().child(
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
                            .text_size(rmac_ui::text_px(15.0))
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
                    .text_size(rmac_ui::text_px(12.0))
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
            .bg(rmac_ui::mac::scrim())
            .child(card.pb_3())
    }
}

impl Render for FinderView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let info = self.info;
        let multi = self.tabs.len() > 1;
        let menu_at = self.menu_at;
        let has_sel = !self.selected.is_empty();
        let can_paste = !self.clipboard.is_empty();
        let operation_error = self.operation_error.clone();
        let transfer = self.transfer.clone();
        div()
            .size_full()
            .relative()
            .v_flex()
            .bg(list_bg())
            .text_color(label())
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                this.menu_at = None;
                cx.notify();
            }))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .child(self.render_toolbar(cx))
            .when_some(operation_error, |el, message| {
                el.child(
                    div()
                        .id("operation-error")
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(rmac_ui::mac::error_background())
                        .border_b_1()
                        .border_color(rmac_ui::mac::error_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .cursor_pointer()
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rmac_ui::mac::danger())
                                .text_color(rmac_ui::mac::on_danger())
                                .child("!"),
                        )
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.operation_error = None;
                            cx.notify();
                        })),
                )
            })
            .when_some(transfer, |el, transfer| {
                let action = if transfer.cancelling {
                    "Cancelling…"
                } else {
                    "Cancel"
                };
                el.child(
                    div()
                        .h(px(34.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(rmac_ui::mac::accent_subtle())
                        .border_b_1()
                        .border_color(rmac_ui::mac::accent_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(label())
                        .child(div().flex_1().child(format!(
                            "{} — {} of {} items",
                            transfer.label, transfer.processed, transfer.total
                        )))
                        .child(
                            div()
                                .id("cancel-transfer")
                                .px_2()
                                .py_0p5()
                                .rounded(px(5.0))
                                .bg(rmac_ui::mac::raised())
                                .border_1()
                                .border_color(rmac_ui::mac::accent_border())
                                .cursor_pointer()
                                .child(action)
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_transfer(cx))),
                        ),
                )
            })
            .when(multi, |el: Div| el.child(self.render_tabs(cx)))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(self.render_sidebar(cx))
                    .child(self.render_list(cx)),
            )
            .when_some(info, |el, ix| el.child(self.render_info(ix, cx)))
            .when_some(menu_at, |el, pos| {
                el.child(Self::build_context_menu(pos, has_sel, can_paste).render())
            })
    }
}

// ---- helpers ----

fn unique_path(path: PathBuf) -> PathBuf {
    unique_path_avoiding(path, &BTreeSet::new())
}

fn unique_path_avoiding(path: PathBuf, reserved: &BTreeSet<PathBuf>) -> PathBuf {
    if !path.exists() && !reserved.contains(&path) {
        return path;
    }
    let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    for n in 2..10_000 {
        let name = match &ext {
            Some(e) => format!("{stem} {n}.{e}"),
            None => format!("{stem} {n}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() && !reserved.contains(&candidate) {
            return candidate;
        }
    }
    path
}

/// Copy preserving macOS metadata (xattrs, resource forks, ACLs, packages) via
/// `ditto`, falling back to a plain recursive copy if ditto is unavailable.
fn copy_item(src: &Path, dst: &Path) -> std::io::Result<()> {
    copy_item_cancellable(src, dst, &AtomicBool::new(false))
}

fn copy_item_cancellable(src: &Path, dst: &Path, cancel: &AtomicBool) -> std::io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "copy cancelled",
        ));
    }
    match Command::new("ditto").arg(src).arg(dst).spawn() {
        Ok(mut child) => loop {
            if cancel.load(Ordering::Acquire) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "copy cancelled",
                ));
            }
            match child.try_wait()? {
                Some(status) if status.success() => return Ok(()),
                Some(status) => {
                    return Err(std::io::Error::other(format!("ditto failed with {status}")));
                }
                None => std::thread::sleep(Duration::from_millis(25)),
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            copy_recursive_cancellable(src, dst, cancel)
        }
        Err(error) => Err(error),
    }
}

fn copy_recursive_cancellable(src: &Path, dst: &Path, cancel: &AtomicBool) -> std::io::Result<()> {
    use std::io::{Read as _, Write as _};

    if cancel.load(Ordering::Acquire) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "copy cancelled",
        ));
    }
    let metadata = std::fs::symlink_metadata(src)?;
    if metadata.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)?;
    } else if metadata.is_dir() {
        std::fs::create_dir(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_recursive_cancellable(&e.path(), &dst.join(e.file_name()), cancel)?;
        }
        std::fs::set_permissions(dst, metadata.permissions())?;
    } else {
        let mut source = std::fs::File::open(src)?;
        let mut destination = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dst)?;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "copy cancelled",
                ));
            }
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            destination.write_all(&buffer[..read])?;
        }
        destination.sync_all()?;
        std::fs::set_permissions(dst, metadata.permissions())?;
    }
    Ok(())
}

fn entry_for(path: &Path) -> Option<Entry> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let md = std::fs::symlink_metadata(path).ok();
    let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let size_bytes = if is_dir {
        0
    } else {
        md.as_ref().map(|m| m.len()).unwrap_or(0)
    };
    let mtime = md
        .as_ref()
        .and_then(|m| m.modified().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let size = if is_dir {
        "--".to_string()
    } else {
        human_size(size_bytes)
    };
    let kind = kind_of(path, is_dir);
    Some(Entry {
        name: name.into(),
        path: path.to_path_buf(),
        is_dir,
        size: size.into(),
        modified: date_label(mtime).into(),
        kind: kind.into(),
        size_bytes,
        mtime,
    })
}

fn read_entries(dir: &Path, show_hidden: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            if let Some(entry) = entry_for(&e.path()) {
                v.push(entry);
            }
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
        if asc {
            o
        } else {
            o.reverse()
        }
    });
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
    // Owner / group (names) via stat.
    if let Ok(out) = Command::new("stat")
        .args(["-f", "%Su\n%Sg", &e.path.to_string_lossy()])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        let mut lines = s.lines();
        if let Some(o) = lines.next().filter(|l| !l.is_empty()) {
            v.push(("Owner", o.to_string()));
        }
        if let Some(g) = lines.next().filter(|l| !l.is_empty()) {
            v.push(("Group", g.to_string()));
        }
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

/// Free space (bytes) on the volume containing `path`, via `df -k`.
fn free_space(path: &Path) -> Option<u64> {
    let out = Command::new("df").arg("-k").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Second line, 4th column = available 1K-blocks.
    let line = text.lines().nth(1)?;
    let avail_k: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(avail_k * 1024)
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
    let days = now
        .date_naive()
        .signed_duration_since(dt.date_naive())
        .num_days();
    if days == 0 {
        format!("Today at {time}")
    } else if days == 1 {
        format!("Yesterday at {time}")
    } else {
        format!("{} {} {} at {time}", dt.day(), dt.format("%b"), dt.year())
    }
}

fn main() {
    rmac_ui::boot_unified_app_with_assets(
        rmac_ui::app_id::FILES,
        CombinedAssets,
        1100.0,
        720.0,
        FinderView::new,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_event_bursts_coalesce_until_consumed() {
        let (sender, receiver) = async_channel::bounded(1);

        sender.try_send(()).expect("first event should wake the UI");
        assert!(sender.try_send(()).is_err(), "burst should stay bounded");
        receiver
            .try_recv()
            .expect("the queued wake should be available");
        sender
            .try_send(())
            .expect("a new event should queue after consumption");
    }

    #[test]
    fn transfer_destinations_do_not_collide_with_reserved_batch_paths() {
        let original = PathBuf::from(format!(
            "/tmp/rmac-reserved-destination-{}",
            std::process::id()
        ));
        let reserved = BTreeSet::from([original.clone()]);

        let destination = unique_path_avoiding(original, &reserved);

        assert!(!reserved.contains(&destination));
        assert!(destination.ends_with(format!(
            "rmac-reserved-destination-{} 2",
            std::process::id()
        )));
    }
}
