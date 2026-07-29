//! rmac Finder — a functional macOS-style file manager (list view). See SPEC.md.
//!
//! Beyond the pixel-accurate chrome: multi-selection, file operations
//! (new folder, rename, duplicate, copy/cut/paste, move-to-trash), keyboard
//! shortcuts + right-click context menus, live search, clickable sort headers,
//! hidden-file toggle, and live directory watching.

mod file_ops;
mod operation_journal;
mod pasteboard;
#[cfg(any(target_os = "linux", test))]
mod trash_store;

use std::borrow::Cow;
use std::collections::{BTreeSet, VecDeque};
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
        RestoreItems,
        DeletePermanently,
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
    Progress(file_ops::TransferProgress),
    Finished {
        report: file_ops::TransferReport,
        recovery_reviews: std::io::Result<Vec<operation_journal::RecoveryReview>>,
    },
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy)]
enum TrashTaskKind {
    Move,
    Restore,
    Delete,
}

#[cfg(any(target_os = "linux", test))]
enum TrashEvent {
    Progress {
        processed: usize,
        total: usize,
    },
    Finished {
        kind: TrashTaskKind,
        completed: usize,
        cancelled: bool,
        failures: Vec<file_ops::Failure>,
        recovery: std::io::Result<(
            trash_store::TrashRecovery,
            Vec<trash_store::TrashRecoveryReview>,
        )>,
    },
}

#[derive(Clone)]
struct ActiveTransfer {
    label: SharedString,
    phase: file_ops::TransferPhase,
    processed: usize,
    total: usize,
    bytes_processed: u64,
    bytes_total: u64,
    cancel: Arc<AtomicBool>,
    cancelling: bool,
    keep_unfinished_in_clipboard: bool,
    retained_clipboard: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConflictTransferKind {
    Copy,
    Move,
}

#[derive(Clone)]
struct TransferConflict {
    kind: ConflictTransferKind,
    source: PathBuf,
    destination: PathBuf,
    source_snapshot: operation_journal::TreeSnapshot,
    destination_snapshot: Option<operation_journal::TreeSnapshot>,
}

struct ConflictBatch {
    label: &'static str,
    ready: Vec<file_ops::TransferTask>,
    conflicts: VecDeque<TransferConflict>,
    conflict_total: usize,
    reserved_destinations: BTreeSet<PathBuf>,
    skipped_moves: Vec<PathBuf>,
    keep_unfinished_in_clipboard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConflictDecision {
    KeepBoth,
    Replace,
    Skip,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone)]
struct ActiveTrash {
    label: SharedString,
    processed: usize,
    total: usize,
    cancel: Arc<AtomicBool>,
    cancelling: bool,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Clone)]
struct DeleteConfirmation {
    items: Vec<trash_store::TrashedItem>,
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
    Trash,
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
    operation_notice: Option<SharedString>,
    operation_error: Option<SharedString>,
    operation_journal: Option<Arc<operation_journal::Journal>>,
    journal_loading: bool,
    pending_operations: usize,
    recovery_reviews: Vec<operation_journal::RecoveryReview>,
    recovery_open: bool,
    recovery_busy: bool,
    transfer: Option<ActiveTransfer>,
    conflict_preflight: bool,
    conflict_batch: Option<ConflictBatch>,
    conflict_busy: bool,
    #[cfg(any(target_os = "linux", test))]
    trash_store: Option<Arc<trash_store::TrashStore>>,
    #[cfg(any(target_os = "linux", test))]
    trash_loading: bool,
    #[cfg(any(target_os = "linux", test))]
    trash_pending: usize,
    #[cfg(any(target_os = "linux", test))]
    trash_recovery_reviews: Vec<trash_store::TrashRecoveryReview>,
    #[cfg(any(target_os = "linux", test))]
    trash_recovery_open: bool,
    #[cfg(any(target_os = "linux", test))]
    trash_recovery_busy: bool,
    #[cfg(any(target_os = "linux", test))]
    trash_operation: Option<ActiveTrash>,
    trash_view: bool,
    #[cfg(any(target_os = "linux", test))]
    trash_items: Vec<trash_store::TrashedItem>,
    #[cfg(any(target_os = "linux", test))]
    trash_generation: u64,
    #[cfg(any(target_os = "linux", test))]
    delete_confirmation: Option<DeleteConfirmation>,
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
        #[cfg(any(target_os = "linux", test))]
        if let Some(trash) = &self.trash_operation {
            trash.cancel.store(true, Ordering::Release);
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
        #[cfg(target_os = "linux")]
        favorites.push(p(
            "Trash",
            PathBuf::new(),
            "icons/trash-2.svg",
            accent(),
            PlaceKind::Trash,
        ));
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
            KeyBinding::new("cmd-option-backspace", DeletePermanently, Some("Finder")),
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
            operation_notice: None,
            operation_error: mount_error,
            operation_journal: None,
            journal_loading: true,
            pending_operations: 0,
            recovery_reviews: Vec::new(),
            recovery_open: false,
            recovery_busy: false,
            transfer: None,
            conflict_preflight: false,
            conflict_batch: None,
            conflict_busy: false,
            #[cfg(any(target_os = "linux", test))]
            trash_store: None,
            #[cfg(any(target_os = "linux", test))]
            trash_loading: true,
            #[cfg(any(target_os = "linux", test))]
            trash_pending: 0,
            #[cfg(any(target_os = "linux", test))]
            trash_recovery_reviews: Vec::new(),
            #[cfg(any(target_os = "linux", test))]
            trash_recovery_open: false,
            #[cfg(any(target_os = "linux", test))]
            trash_recovery_busy: false,
            #[cfg(any(target_os = "linux", test))]
            trash_operation: None,
            trash_view: false,
            #[cfg(any(target_os = "linux", test))]
            trash_items: Vec::new(),
            #[cfg(any(target_os = "linux", test))]
            trash_generation: 0,
            #[cfg(any(target_os = "linux", test))]
            delete_confirmation: None,
            free_bytes: None,
            dragging: false,
            focus,
            watcher,
            watched: None,
            search_generation: 0,
            search_cancel: None,
        };
        view.reload(cx);

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let journal = Arc::new(operation_journal::Journal::open_default()?);
                    let recovery = journal.recover_unambiguous()?;
                    let reviews = journal.review_pending()?;
                    Ok::<_, std::io::Error>((journal, recovery, reviews))
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.journal_loading = false;
                match result {
                    Ok((journal, recovery, reviews)) => {
                        this.operation_journal = Some(journal);
                        this.pending_operations = reviews.len();
                        this.recovery_open = !reviews.is_empty();
                        this.recovery_reviews = reviews;
                        if recovery.finalized != 0 {
                            this.operation_notice = Some(
                                format!(
                                    "Files safely completed {} interrupted file operation{}",
                                    recovery.finalized,
                                    if recovery.finalized == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        } else if recovery.active != 0 {
                            this.operation_notice = Some(
                                format!(
                                    "Another Files window is safely handling {} file operation{}",
                                    recovery.active,
                                    if recovery.active == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        }
                        if this.pending_operations != 0 {
                            this.operation_error = Some(
                                format!(
                                    "Review {} unfinished file operation{} before starting another transfer",
                                    this.pending_operations,
                                    if this.pending_operations == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        }
                    }
                    Err(_) => {
                        this.operation_journal = None;
                        this.operation_error = Some(
                            "File-operation recovery data could not be verified; transfers are disabled"
                                .into(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();

        #[cfg(any(target_os = "linux", test))]
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let store = Arc::new(trash_store::TrashStore::open_default()?);
                    let recovery = store.recover_and_review();
                    Ok::<_, std::io::Error>((store, recovery))
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.trash_loading = false;
                match result {
                    Ok((store, recovery)) => match recovery {
                        Ok((recovery, reviews)) => {
                            this.trash_store = Some(store);
                            this.trash_pending = recovery.pending;
                            this.trash_recovery_reviews = reviews;
                            this.trash_recovery_open = recovery.pending != 0;
                            if recovery.finalized != 0 {
                                this.operation_notice = Some(
                                    format!(
                                        "Files safely completed {} interrupted Trash operation{}",
                                        recovery.finalized,
                                        if recovery.finalized == 1 { "" } else { "s" }
                                    )
                                    .into(),
                                );
                            }
                            if recovery.pending != 0 {
                                this.operation_error = Some(
                                    format!(
                                        "Review {} changed Trash operation{} before using Trash",
                                        recovery.pending,
                                        if recovery.pending == 1 { "" } else { "s" }
                                    )
                                    .into(),
                                );
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            this.trash_store = Some(store);
                            this.operation_notice =
                                Some("Another Files window is safely handling Trash".into());
                        }
                        Err(_) => {
                            this.trash_store = None;
                            this.trash_recovery_reviews.clear();
                            this.trash_recovery_open = false;
                            this.operation_error = Some(
                                "Trash recovery data could not be verified; Trash actions are disabled"
                                    .into(),
                            );
                        }
                    },
                    Err(_) => {
                        this.trash_store = None;
                        this.trash_recovery_reviews.clear();
                        this.trash_recovery_open = false;
                        this.operation_error = Some(
                            "Trash recovery data could not be verified; Trash actions are disabled"
                                .into(),
                        );
                    }
                }
                if this.trash_view && this.trash_store.is_some() {
                    this.reload_trash(cx);
                } else {
                    cx.notify();
                }
            });
        })
        .detach();

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
        if self.trash_view {
            self.reload_trash(cx);
            return;
        }
        #[cfg(any(target_os = "linux", test))]
        {
            self.delete_confirmation = None;
        }
        self.reload_inner(cx, true);
    }

    /// Refresh after a watcher event without spawning `df`; free space changes
    /// slowly and is refreshed on navigation and explicit file operations.
    fn reload_after_event(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            return;
        }
        self.reload_inner(cx, false);
    }

    fn reload_trash(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", test))]
        {
            self.cancel_search();
            self.result_title = Some("Trash".into());
            self.selected.clear();
            self.anchor = None;
            self.renaming = None;
            self.trash_generation = self.trash_generation.wrapping_add(1);
            let generation = self.trash_generation;
            let key = self.sort_key;
            let asc = self.sort_asc;
            let Some(store) = self.trash_store.clone() else {
                self.entries.clear();
                self.trash_items.clear();
                self.operation_error = Some(
                    if self.trash_loading {
                        "Files is still verifying Trash recovery"
                    } else {
                        "Trash is unavailable"
                    }
                    .into(),
                );
                cx.notify();
                return;
            };
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let items = store.list()?;
                        let mut entries = Vec::with_capacity(items.len());
                        for item in &items {
                            let mut entry = entry_for(item.data_path()).ok_or_else(|| {
                                std::io::Error::new(
                                    std::io::ErrorKind::WouldBlock,
                                    "Trash changed while it was listed",
                                )
                            })?;
                            entry.name = item
                                .original_path
                                .file_name()
                                .unwrap_or(item.name.as_os_str())
                                .to_string_lossy()
                                .into_owned()
                                .into();
                            entry.modified = item.deleted_at.clone().into();
                            entries.push(entry);
                        }
                        sort_entries(&mut entries, key, asc);
                        Ok::<_, std::io::Error>((items, entries))
                    })
                    .await;
                let _ = this.update(cx, |this: &mut FinderView, cx| {
                    if !this.trash_view || this.trash_generation != generation {
                        return;
                    }
                    match result {
                        Ok((items, entries)) => {
                            this.trash_items = items;
                            this.entries = entries;
                            this.free_bytes = None;
                        }
                        Err(error) => {
                            this.entries.clear();
                            this.trash_items.clear();
                            this.operation_error = Some(
                                match error.kind() {
                                    std::io::ErrorKind::WouldBlock => {
                                        "Trash is busy or changed; try again"
                                    }
                                    _ => "Trash could not be verified safely",
                                }
                                .into(),
                            );
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        #[cfg(not(any(target_os = "linux", test)))]
        {
            self.entries.clear();
            self.operation_error = Some("Trash browsing is available on Linux".into());
            cx.notify();
        }
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
        self.trash_view = false;
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
            self.trash_view = false;
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
        self.trash_view = false;
        self.active = i;
        self.load_tab(i);
        self.reload(cx);
    }

    fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_dir() || (path == self.cwd && !self.trash_view) {
            return;
        }
        self.trash_view = false;
        self.back.push(self.cwd.clone());
        self.fwd.clear();
        self.cwd = path;
        self.reload(cx);
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.trash_view = false;
            self.reload(cx);
            return;
        }
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
        if self.trash_view {
            self.trash_view = false;
            self.reload(cx);
            return;
        }
        if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
            self.navigate(parent, cx);
        }
    }

    fn open_index(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Restore the item before opening it".into());
            cx.notify();
            return;
        }
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
        if self.trash_view {
            self.operation_error = Some("Restore items before opening them".into());
            cx.notify();
            return;
        }
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
        if self.trash_view {
            self.operation_error =
                Some("Use Restore for items in Trash; direct changes are disabled".into());
            cx.notify();
            return true;
        }
        #[cfg(any(target_os = "linux", test))]
        let trash_busy = self.trash_operation.is_some();
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_busy = false;
        if self.transfer.is_none()
            && !trash_busy
            && !self.conflict_preflight
            && self.conflict_batch.is_none()
        {
            return false;
        }
        self.operation_error = Some("Wait for the current file operation to finish".into());
        cx.notify();
        true
    }

    fn start_transfer_with_conflicts(
        &mut self,
        label: &'static str,
        tasks: Vec<file_ops::TransferTask>,
        keep_unfinished_in_clipboard: bool,
        cx: &mut Context<Self>,
    ) {
        if tasks.is_empty() || self.block_mutation_during_transfer(cx) {
            return;
        }
        if self.journal_loading {
            self.operation_error = Some("Files is still verifying file-operation recovery".into());
            cx.notify();
            return;
        }
        if self.operation_journal.is_none() {
            self.operation_error =
                Some("File-operation recovery is unavailable; transfers are disabled".into());
            cx.notify();
            return;
        }
        if self.pending_operations != 0 {
            self.operation_error = Some(
                format!(
                    "Resolve {} unfinished file operation{} before starting another transfer",
                    self.pending_operations,
                    if self.pending_operations == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
                .into(),
            );
            self.recovery_open = true;
            cx.notify();
            return;
        }

        self.conflict_preflight = true;
        self.operation_error = None;
        self.operation_notice = Some("Checking for file-name conflicts…".into());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let prepared =
                cx.background_executor()
                    .spawn(async move {
                        prepare_conflict_batch(label, tasks, keep_unfinished_in_clipboard)
                    })
                    .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.conflict_preflight = false;
                this.operation_notice = None;
                match prepared {
                    Ok(batch) if batch.conflicts.is_empty() => {
                        this.start_transfer_with_retained(
                            batch.label,
                            batch.ready,
                            batch.keep_unfinished_in_clipboard,
                            batch.skipped_moves,
                            cx,
                        );
                    }
                    Ok(batch) => {
                        this.conflict_batch = Some(batch);
                        this.conflict_busy = false;
                        cx.notify();
                    }
                    Err(_) => {
                        this.operation_error = Some(
                            "Files could not verify the conflicting items; nothing was changed"
                                .into(),
                        );
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn resolve_current_conflict(&mut self, decision: ConflictDecision, cx: &mut Context<Self>) {
        if self.conflict_busy {
            return;
        }
        let Some(batch) = self.conflict_batch.as_ref() else {
            return;
        };
        let Some(conflict) = batch.conflicts.front().cloned() else {
            return;
        };
        let reserved = batch.reserved_destinations.clone();
        if decision == ConflictDecision::Replace
            && (conflict.destination_snapshot.is_none() || conflict.source == conflict.destination)
        {
            return;
        }

        self.conflict_busy = true;
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    resolve_conflict_task(&conflict, decision, &reserved)
                        .map(|task| (task, conflict))
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.conflict_busy = false;
                let (task, resolved) = match result {
                    Ok(result) => result,
                    Err(_) => {
                        this.conflict_batch = None;
                        this.operation_error = Some(
                            "The source or destination changed while the conflict was open; nothing was changed. Start the operation again to review the current items."
                                .into(),
                        );
                        cx.notify();
                        return;
                    }
                };
                let Some(batch) = this.conflict_batch.as_mut() else {
                    return;
                };
                batch.conflicts.pop_front();
                if decision == ConflictDecision::Skip
                    && resolved.kind == ConflictTransferKind::Move
                    && batch.keep_unfinished_in_clipboard
                {
                    batch.skipped_moves.push(resolved.source);
                }
                if let Some(task) = task {
                    batch.reserved_destinations.insert(task.destination.clone());
                    batch.ready.push(task);
                }
                if batch.conflicts.is_empty() {
                    let batch = this
                        .conflict_batch
                        .take()
                        .expect("completed conflict batch should still exist");
                    if batch.ready.is_empty() {
                        if batch.keep_unfinished_in_clipboard {
                            this.clipboard = batch.skipped_moves;
                            this.clipboard.sort();
                            this.clipboard.dedup();
                            this.clip_cut = !this.clipboard.is_empty();
                            if this.clip_cut {
                                this.write_clip_text(cx);
                            }
                        }
                        this.operation_notice =
                            Some("Skipped the conflicting items; nothing was changed".into());
                        cx.notify();
                    } else {
                        this.start_transfer_with_retained(
                            batch.label,
                            batch.ready,
                            batch.keep_unfinished_in_clipboard,
                            batch.skipped_moves,
                            cx,
                        );
                    }
                } else {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_transfer(
        &mut self,
        label: &'static str,
        tasks: Vec<file_ops::TransferTask>,
        keep_unfinished_in_clipboard: bool,
        cx: &mut Context<Self>,
    ) {
        self.start_transfer_with_retained(
            label,
            tasks,
            keep_unfinished_in_clipboard,
            Vec::new(),
            cx,
        );
    }

    fn start_transfer_with_retained(
        &mut self,
        label: &'static str,
        tasks: Vec<file_ops::TransferTask>,
        keep_unfinished_in_clipboard: bool,
        retained_clipboard: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if tasks.is_empty() {
            return;
        }
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        if self.journal_loading {
            self.operation_error = Some("Files is still verifying file-operation recovery".into());
            cx.notify();
            return;
        }
        let Some(journal) = self.operation_journal.clone() else {
            self.operation_error =
                Some("File-operation recovery is unavailable; transfers are disabled".into());
            cx.notify();
            return;
        };
        if self.pending_operations != 0 {
            self.operation_error = Some(
                format!(
                    "Resolve {} unfinished file operation{} before starting another transfer",
                    self.pending_operations,
                    if self.pending_operations == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
                .into(),
            );
            self.recovery_open = true;
            cx.notify();
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        self.operation_error = None;
        self.transfer = Some(ActiveTransfer {
            label: label.into(),
            phase: file_ops::TransferPhase::Scanning,
            processed: 0,
            total: tasks.len(),
            bytes_processed: 0,
            bytes_total: 0,
            cancel: cancel.clone(),
            cancelling: false,
            keep_unfinished_in_clipboard,
            retained_clipboard,
        });
        cx.notify();

        // Byte progress can advance faster than the renderer. Keep this bridge
        // bounded and drop intermediate snapshots; the terminal result still
        // uses backpressure and is never intentionally discarded.
        let (events, event_rx) = async_channel::bounded(64);
        cx.background_executor()
            .spawn(async move {
                let progress_events = events.clone();
                let report = file_ops::execute_transfers(
                    &file_ops::RealFileSystem,
                    Some(journal.as_ref()),
                    &tasks,
                    &cancel,
                    move |progress| {
                        let _ = progress_events.try_send(TransferEvent::Progress(progress));
                    },
                );
                let recovery_reviews = journal
                    .recover_unambiguous()
                    .and_then(|_| journal.review_pending());
                let _ = events.send_blocking(TransferEvent::Finished {
                    report,
                    recovery_reviews,
                });
            })
            .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = event_rx.recv().await {
                let finished = matches!(event, TransferEvent::Finished { .. });
                if this
                    .update(cx, |this: &mut FinderView, cx| match event {
                        TransferEvent::Progress(progress) => {
                            if let Some(transfer) = this.transfer.as_mut() {
                                transfer.phase = progress.phase;
                                transfer.processed = progress.processed;
                                transfer.total = progress.total;
                                transfer.bytes_processed = progress.bytes_processed;
                                transfer.bytes_total = progress.bytes_total;
                            }
                            cx.notify();
                        }
                        TransferEvent::Finished {
                            report,
                            recovery_reviews,
                        } => {
                            let keep_clipboard = this
                                .transfer
                                .as_ref()
                                .is_some_and(|transfer| transfer.keep_unfinished_in_clipboard);
                            let retained_clipboard = this
                                .transfer
                                .as_ref()
                                .map(|transfer| transfer.retained_clipboard.clone())
                                .unwrap_or_default();
                            this.transfer = None;
                            if keep_clipboard {
                                let mut unfinished = retained_clipboard;
                                unfinished.extend(report.unfinished_moves);
                                unfinished.sort();
                                unfinished.dedup();
                                this.clipboard = unfinished;
                                this.clip_cut = !this.clipboard.is_empty();
                                if this.clip_cut {
                                    this.write_clip_text(cx);
                                }
                            }
                            match recovery_reviews {
                                Ok(reviews) => {
                                    this.pending_operations = reviews.len();
                                    this.recovery_open = !reviews.is_empty();
                                    this.recovery_reviews = reviews;
                                }
                                Err(_) => {
                                    this.operation_journal = None;
                                    this.pending_operations = 0;
                                    this.recovery_reviews.clear();
                                    this.recovery_open = false;
                                }
                            }
                            this.record_operation_failures(report.failures, cx);
                            if this.operation_journal.is_none() {
                                this.operation_error = Some(
                                    "File-operation recovery data could not be verified; transfers are disabled"
                                        .into(),
                                );
                            } else if this.pending_operations != 0
                                && this.operation_error.is_none()
                            {
                                this.operation_error = Some(
                                    format!(
                                        "Files retained {} unfinished file-operation record{} for recovery",
                                        this.pending_operations,
                                        if this.pending_operations == 1 { "" } else { "s" }
                                    )
                                    .into(),
                                );
                            }
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

    fn cancel_trash(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", test))]
        if let Some(operation) = self.trash_operation.as_mut() {
            operation.cancel.store(true, Ordering::Release);
            operation.cancelling = true;
            cx.notify();
        }
        #[cfg(not(any(target_os = "linux", test)))]
        let _ = cx;
    }

    #[cfg(any(target_os = "linux", test))]
    fn receive_trash_events(
        &mut self,
        event_rx: async_channel::Receiver<TrashEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = event_rx.recv().await {
                let finished = matches!(event, TrashEvent::Finished { .. });
                if this
                    .update(cx, |this: &mut FinderView, cx| match event {
                        TrashEvent::Progress { processed, total } => {
                            if let Some(operation) = this.trash_operation.as_mut() {
                                operation.processed = processed;
                                operation.total = total;
                            }
                            cx.notify();
                        }
                        TrashEvent::Finished {
                            kind,
                            completed,
                            cancelled,
                            failures,
                            recovery,
                        } => this
                            .finish_trash_task(kind, completed, cancelled, failures, recovery, cx),
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

    #[cfg(any(target_os = "linux", test))]
    fn finish_trash_task(
        &mut self,
        kind: TrashTaskKind,
        completed: usize,
        cancelled: bool,
        failures: Vec<file_ops::Failure>,
        recovery: std::io::Result<(
            trash_store::TrashRecovery,
            Vec<trash_store::TrashRecoveryReview>,
        )>,
        cx: &mut Context<Self>,
    ) {
        self.trash_operation = None;
        let recovery_unavailable = match recovery {
            Ok((recovery, reviews)) => {
                self.trash_pending = recovery.pending;
                self.trash_recovery_reviews = reviews;
                self.trash_recovery_open = recovery.pending != 0;
                false
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                self.operation_notice =
                    Some("Another Files window is safely handling Trash".into());
                false
            }
            Err(_) => {
                self.trash_store = None;
                self.trash_pending = 0;
                self.trash_recovery_reviews.clear();
                self.trash_recovery_open = false;
                self.operation_error = Some(
                    "Trash recovery data could not be verified; Trash actions are disabled".into(),
                );
                true
            }
        };
        if !failures.is_empty() {
            self.operation_notice = None;
            self.record_operation_failures(failures, cx);
        }
        if recovery_unavailable {
            let unavailable = "Trash recovery is unavailable; Trash actions are disabled";
            self.operation_error = Some(
                match self.operation_error.take() {
                    Some(failure) => format!("{failure}. {unavailable}"),
                    None => unavailable.to_string(),
                }
                .into(),
            );
        }
        if self.trash_pending != 0 {
            let retained = format!(
                "{} changed Trash operation{} retained for manual recovery",
                self.trash_pending,
                if self.trash_pending == 1 {
                    " was"
                } else {
                    "s were"
                }
            );
            self.operation_error = Some(
                match self.operation_error.take() {
                    Some(failure) => format!("{failure}. {retained}"),
                    None => retained,
                }
                .into(),
            );
        } else if cancelled && self.operation_error.is_none() {
            self.operation_notice = Some(
                match (kind, completed) {
                    (TrashTaskKind::Move, 0) => {
                        "Move to Trash cancelled; no item was moved".to_string()
                    }
                    (TrashTaskKind::Move, completed) => format!(
                        "Move to Trash cancelled after moving {completed} item{}",
                        if completed == 1 { "" } else { "s" }
                    ),
                    (TrashTaskKind::Restore, 0) => {
                        "Restore cancelled; no item was restored".to_string()
                    }
                    (TrashTaskKind::Restore, completed) => format!(
                        "Restore cancelled after restoring {completed} item{}",
                        if completed == 1 { "" } else { "s" }
                    ),
                    (TrashTaskKind::Delete, 0) => {
                        "Permanent deletion cancelled; no item was deleted".to_string()
                    }
                    (TrashTaskKind::Delete, completed) => format!(
                        "Permanent deletion cancelled after deleting {completed} item{}",
                        if completed == 1 { "" } else { "s" }
                    ),
                }
                .into(),
            );
        } else if self.operation_error.is_none() {
            self.operation_notice = Some(
                match (kind, completed) {
                    (TrashTaskKind::Move, 1) => "Moved 1 item to Trash".to_string(),
                    (TrashTaskKind::Move, completed) => {
                        format!("Moved {completed} items to Trash")
                    }
                    (TrashTaskKind::Restore, 1) => "Restored 1 item".to_string(),
                    (TrashTaskKind::Restore, completed) => {
                        format!("Restored {completed} items")
                    }
                    (TrashTaskKind::Delete, 1) => "Permanently deleted 1 item".to_string(),
                    (TrashTaskKind::Delete, completed) => {
                        format!("Permanently deleted {completed} items")
                    }
                }
                .into(),
            );
        }
        self.reload(cx);
    }

    fn close_recovery(&mut self, cx: &mut Context<Self>) {
        if self.recovery_busy {
            return;
        }
        self.recovery_open = false;
        cx.notify();
    }

    fn resolve_current_recovery(&mut self, cx: &mut Context<Self>) {
        if self.recovery_busy {
            return;
        }
        let Some(review) = self.recovery_reviews.first().cloned() else {
            self.recovery_open = false;
            cx.notify();
            return;
        };
        let Some(journal) = self.operation_journal.clone() else {
            self.operation_error =
                Some("File-operation recovery is unavailable; no item was changed".into());
            cx.notify();
            return;
        };
        self.recovery_busy = true;
        self.operation_error = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (outcome, refresh) = cx
                .background_executor()
                .spawn(async move {
                    let outcome = journal.resolve_review(&review);
                    let refresh = journal
                        .recover_unambiguous()
                        .and_then(|recovery| {
                            journal
                                .review_pending()
                                .map(|reviews| (recovery, reviews))
                        });
                    (outcome, refresh)
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.recovery_busy = false;
                match refresh {
                    Ok((recovery, reviews)) => {
                        this.pending_operations = reviews.len();
                        this.recovery_reviews = reviews;
                        this.recovery_open = this.pending_operations != 0;
                        if recovery.finalized != 0 && outcome.is_err() {
                            this.operation_notice = Some(
                                format!(
                                    "Files safely completed {} interrupted file operation{}",
                                    recovery.finalized,
                                    if recovery.finalized == 1 { "" } else { "s" }
                                )
                                .into(),
                            );
                        }
                    }
                    Err(_) => {
                        this.operation_journal = None;
                        this.pending_operations = 0;
                        this.recovery_reviews.clear();
                        this.recovery_open = false;
                        this.operation_error = Some(
                            "File-operation recovery data could not be verified; transfers are disabled"
                                .into(),
                        );
                        cx.notify();
                        return;
                    }
                }

                match outcome {
                    Ok(operation_journal::ResolutionOutcome::PreservedCopy {
                        complete,
                        name,
                    }) => {
                        this.operation_notice = Some(
                            if complete {
                                format!("Recovery copy preserved as “{name}”")
                            } else {
                                format!(
                                    "Partial recovery copy preserved as “{name}”; inspect it before relying on it"
                                )
                            }
                            .into(),
                        );
                        this.operation_error = None;
                    }
                    Ok(
                        operation_journal::ResolutionOutcome::PreservedReplacementBackup {
                            complete,
                            name,
                        },
                    ) => {
                        this.operation_notice = Some(
                            if complete {
                                format!("Previous destination preserved as “{name}”")
                            } else {
                                format!(
                                    "Possibly changed previous destination preserved as “{name}”; inspect it before relying on it"
                                )
                            }
                            .into(),
                        );
                        this.operation_error = None;
                    }
                    Ok(operation_journal::ResolutionOutcome::KeptExistingItems) => {
                        this.operation_notice =
                            Some("Existing items kept; no file was deleted".into());
                        this.operation_error = None;
                    }
                    Err(error) => {
                        this.operation_error = Some(
                            match error.kind() {
                                std::io::ErrorKind::AlreadyExists => {
                                    "The recovery name is no longer available; review the updated choice"
                                }
                                std::io::ErrorKind::WouldBlock => {
                                    "Recovery state changed; review it again before continuing"
                                }
                                _ => {
                                    "Files could not preserve the recovery copy; no existing item was overwritten"
                                }
                            }
                            .into(),
                        );
                        this.recovery_open = !this.recovery_reviews.is_empty();
                    }
                }
                if this.pending_operations != 0 && this.operation_error.is_none() {
                    this.operation_error = Some(
                        format!(
                            "Review {} remaining file operation{} before starting another transfer",
                            this.pending_operations,
                            if this.pending_operations == 1 { "" } else { "s" }
                        )
                        .into(),
                    );
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    #[cfg(any(target_os = "linux", test))]
    fn close_trash_recovery(&mut self, cx: &mut Context<Self>) {
        if self.trash_recovery_busy {
            return;
        }
        self.trash_recovery_open = false;
        cx.notify();
    }

    #[cfg(any(target_os = "linux", test))]
    fn resolve_current_trash_recovery(&mut self, cx: &mut Context<Self>) {
        if self.trash_recovery_busy {
            return;
        }
        let Some(review) = self.trash_recovery_reviews.first().cloned() else {
            self.trash_recovery_open = false;
            cx.notify();
            return;
        };
        let Some(store) = self.trash_store.clone() else {
            self.operation_error =
                Some("Trash recovery is unavailable; no item was changed".into());
            cx.notify();
            return;
        };
        self.trash_recovery_busy = true;
        self.operation_error = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (outcome, refresh) = cx
                .background_executor()
                .spawn(async move {
                    match store.resolve_review_and_refresh(&review) {
                        Ok((outcome, recovery, reviews)) => {
                            (Ok(outcome), Ok((recovery, reviews)))
                        }
                        Err(error) => {
                            let refresh = store.recover_and_review();
                            (Err(error), refresh)
                        }
                    }
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.trash_recovery_busy = false;
                match refresh {
                    Ok((recovery, reviews)) => {
                        this.trash_pending = recovery.pending;
                        this.trash_recovery_reviews = reviews;
                        this.trash_recovery_open = recovery.pending != 0;
                    }
                    Err(_) => {
                        this.trash_store = None;
                        this.trash_pending = 0;
                        this.trash_recovery_reviews.clear();
                        this.trash_recovery_open = false;
                        this.operation_error = Some(
                            "Trash recovery data could not be verified; Trash actions are disabled"
                                .into(),
                        );
                        cx.notify();
                        return;
                    }
                }

                match outcome {
                    Ok(trash_store::TrashResolutionOutcome::ReturnedRemainingItem {
                        may_be_partial,
                    }) => {
                        this.operation_notice = Some(
                            if may_be_partial {
                                "Remaining data returned to Trash; it may be incomplete"
                            } else {
                                "Item returned safely to Trash"
                            }
                            .into(),
                        );
                        this.operation_error = None;
                    }
                    Ok(trash_store::TrashResolutionOutcome::RemovedOrphanMetadata) => {
                        this.operation_notice =
                            Some("Orphaned Trash metadata removed; no user file was deleted".into());
                        this.operation_error = None;
                    }
                    Ok(trash_store::TrashResolutionOutcome::KeptExistingItems) => {
                        this.operation_notice = Some(
                            "Existing items kept; only the exact recovery record was cleared"
                                .into(),
                        );
                        this.operation_error = None;
                    }
                    Err(error) => {
                        this.operation_error = Some(
                            match error.kind() {
                                std::io::ErrorKind::AlreadyExists => {
                                    "The Trash item location is no longer available; review the updated state"
                                }
                                std::io::ErrorKind::WouldBlock => {
                                    "Trash recovery changed; review it again before continuing"
                                }
                                _ => {
                                    "Files could not resolve Trash recovery; no existing item was replaced"
                                }
                            }
                            .into(),
                        );
                        this.trash_recovery_open = !this.trash_recovery_reviews.is_empty();
                    }
                }
                if this.trash_pending != 0 && this.operation_error.is_none() {
                    this.operation_error = Some(
                        format!(
                            "Review {} remaining Trash operation{} before using Trash",
                            this.trash_pending,
                            if this.trash_pending == 1 { "" } else { "s" }
                        )
                        .into(),
                    );
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    // ---- operations ----
    fn new_folder(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
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
        if self.trash_view {
            self.restore_selected(cx);
            return;
        }
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }

        #[cfg(any(target_os = "linux", test))]
        {
            if self.trash_loading {
                self.operation_error = Some("Files is still verifying Trash recovery".into());
                cx.notify();
                return;
            }
            let Some(store) = self.trash_store.clone() else {
                self.operation_error =
                    Some("Trash recovery is unavailable; no item was changed".into());
                cx.notify();
                return;
            };
            if self.trash_pending != 0 {
                self.operation_error = Some(
                    "A changed Trash operation needs manual recovery before another item can be moved"
                        .into(),
                );
                cx.notify();
                return;
            }
            let total = paths.len();
            let cancel = Arc::new(AtomicBool::new(false));
            self.trash_operation = Some(ActiveTrash {
                label: "Moving to Trash".into(),
                processed: 0,
                total,
                cancel: cancel.clone(),
                cancelling: false,
            });
            self.operation_error = None;
            self.operation_notice = None;
            cx.notify();

            let (events, event_rx) = async_channel::bounded(16);
            cx.background_executor()
                .spawn(async move {
                    let mut failures = Vec::new();
                    let mut completed = 0usize;
                    let mut processed = 0usize;
                    let mut cancelled = false;
                    for path in paths {
                        if cancel.load(Ordering::Acquire) {
                            cancelled = true;
                            break;
                        }
                        match store.trash(&path, &cancel) {
                            Ok(()) => completed += 1,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                cancelled = true;
                                break;
                            }
                            Err(error) => {
                                let blocked = error.kind() == std::io::ErrorKind::WouldBlock;
                                failures.push(file_ops::Failure::message(
                                    file_ops::Operation::Trash,
                                    &path,
                                    None,
                                    error.to_string(),
                                ));
                                if blocked {
                                    processed += 1;
                                    let _ =
                                        events.try_send(TrashEvent::Progress { processed, total });
                                    break;
                                }
                            }
                        }
                        processed += 1;
                        let _ = events.try_send(TrashEvent::Progress { processed, total });
                    }
                    let recovery = store.recover_and_review();
                    let _ = events.send_blocking(TrashEvent::Finished {
                        kind: TrashTaskKind::Move,
                        completed,
                        cancelled,
                        failures,
                        recovery,
                    });
                })
                .detach();
            self.receive_trash_events(event_rx, cx);
        }

        #[cfg(not(any(target_os = "linux", test)))]
        {
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

    #[cfg(any(target_os = "linux", test))]
    fn selected_trash_items(&self) -> Vec<trash_store::TrashedItem> {
        let selected_paths = self.selected_paths().into_iter().collect::<BTreeSet<_>>();
        self.trash_items
            .iter()
            .filter(|item| selected_paths.contains(item.data_path()))
            .cloned()
            .collect()
    }

    fn restore_selected(&mut self, cx: &mut Context<Self>) {
        if !self.trash_view {
            self.operation_error = Some("Open Trash to restore items".into());
            cx.notify();
            return;
        }
        #[cfg(any(target_os = "linux", test))]
        {
            if self.transfer.is_some() || self.trash_operation.is_some() {
                self.operation_error = Some("Wait for the current file operation to finish".into());
                cx.notify();
                return;
            }
            if self.trash_loading {
                self.operation_error = Some("Files is still verifying Trash recovery".into());
                cx.notify();
                return;
            }
            let Some(store) = self.trash_store.clone() else {
                self.operation_error =
                    Some("Trash recovery is unavailable; no item was changed".into());
                cx.notify();
                return;
            };
            if self.trash_pending != 0 {
                self.operation_error =
                    Some("A changed Trash operation needs manual recovery before restore".into());
                cx.notify();
                return;
            }
            let items = self.selected_trash_items();
            if items.is_empty() {
                return;
            }
            let total = items.len();
            let cancel = Arc::new(AtomicBool::new(false));
            self.trash_operation = Some(ActiveTrash {
                label: "Restoring".into(),
                processed: 0,
                total,
                cancel: cancel.clone(),
                cancelling: false,
            });
            self.operation_error = None;
            self.operation_notice = None;
            cx.notify();

            let (events, event_rx) = async_channel::bounded(16);
            cx.background_executor()
                .spawn(async move {
                    let mut failures = Vec::new();
                    let mut completed = 0usize;
                    let mut processed = 0usize;
                    let mut cancelled = false;
                    for item in items {
                        if cancel.load(Ordering::Acquire) {
                            cancelled = true;
                            break;
                        }
                        match store.restore(&item, &cancel) {
                            Ok(_) => completed += 1,
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                                cancelled = true;
                                break;
                            }
                            Err(error) => {
                                let blocked = error.kind() == std::io::ErrorKind::WouldBlock;
                                failures.push(file_ops::Failure::message(
                                    file_ops::Operation::Restore,
                                    &item.original_path,
                                    None,
                                    error.to_string(),
                                ));
                                if blocked {
                                    processed += 1;
                                    let _ =
                                        events.try_send(TrashEvent::Progress { processed, total });
                                    break;
                                }
                            }
                        }
                        processed += 1;
                        let _ = events.try_send(TrashEvent::Progress { processed, total });
                    }
                    let recovery = store.recover_and_review();
                    let _ = events.send_blocking(TrashEvent::Finished {
                        kind: TrashTaskKind::Restore,
                        completed,
                        cancelled,
                        failures,
                        recovery,
                    });
                })
                .detach();
            self.receive_trash_events(event_rx, cx);
        }
        #[cfg(not(any(target_os = "linux", test)))]
        {
            self.operation_error = Some("Trash restore is available on Linux".into());
            cx.notify();
        }
    }

    fn request_permanent_delete(&mut self, cx: &mut Context<Self>) {
        if !self.trash_view {
            self.operation_error =
                Some("Permanent deletion is available for items in Trash".into());
            cx.notify();
            return;
        }
        #[cfg(any(target_os = "linux", test))]
        {
            if self.transfer.is_some()
                || self.trash_operation.is_some()
                || self.recovery_open
                || self.recovery_busy
                || self.trash_recovery_open
                || self.trash_recovery_busy
            {
                self.operation_error = Some("Wait for the current file operation to finish".into());
                cx.notify();
                return;
            }
            if self.trash_loading {
                self.operation_error = Some("Files is still verifying Trash recovery".into());
                cx.notify();
                return;
            }
            if self.trash_store.is_none() {
                self.operation_error =
                    Some("Trash recovery is unavailable; no item was changed".into());
                cx.notify();
                return;
            }
            if self.trash_pending != 0 {
                self.operation_error = Some(
                    "A changed Trash operation needs manual recovery before permanent deletion"
                        .into(),
                );
                cx.notify();
                return;
            }
            let items = self.selected_trash_items();
            if items.is_empty() {
                return;
            }
            self.menu_at = None;
            self.operation_error = None;
            self.operation_notice = None;
            self.delete_confirmation = Some(DeleteConfirmation { items });
            cx.notify();
        }
        #[cfg(not(any(target_os = "linux", test)))]
        {
            self.operation_error = Some("Permanent deletion is available on Linux".into());
            cx.notify();
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn cancel_permanent_delete(&mut self, cx: &mut Context<Self>) {
        self.delete_confirmation = None;
        cx.notify();
    }

    #[cfg(any(target_os = "linux", test))]
    fn confirm_permanent_delete(&mut self, cx: &mut Context<Self>) {
        let Some(confirmation) = self.delete_confirmation.take() else {
            return;
        };
        let Some(store) = self.trash_store.clone() else {
            self.operation_error =
                Some("Trash recovery is unavailable; no item was changed".into());
            cx.notify();
            return;
        };
        if self.transfer.is_some() || self.trash_operation.is_some() || self.trash_pending != 0 {
            self.operation_error = Some("Wait for the current file operation to finish".into());
            cx.notify();
            return;
        }
        let items = confirmation.items;
        let total = items.len();
        if total == 0 {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.trash_operation = Some(ActiveTrash {
            label: "Deleting Permanently".into(),
            processed: 0,
            total,
            cancel: cancel.clone(),
            cancelling: false,
        });
        self.operation_error = None;
        self.operation_notice = None;
        cx.notify();

        let (events, event_rx) = async_channel::bounded(16);
        cx.background_executor()
            .spawn(async move {
                let mut failures = Vec::new();
                let mut completed = 0usize;
                let mut processed = 0usize;
                let mut cancelled = false;
                for item in items {
                    if cancel.load(Ordering::Acquire) {
                        cancelled = true;
                        break;
                    }
                    match store.delete_permanently(&item, &cancel) {
                        Ok(()) => completed += 1,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                            cancelled = true;
                            break;
                        }
                        Err(error) => {
                            let blocked = error.kind() == std::io::ErrorKind::WouldBlock;
                            failures.push(file_ops::Failure::message(
                                file_ops::Operation::PermanentDelete,
                                &item.original_path,
                                None,
                                error.to_string(),
                            ));
                            if blocked {
                                processed += 1;
                                let _ = events.try_send(TrashEvent::Progress { processed, total });
                                break;
                            }
                        }
                    }
                    processed += 1;
                    let _ = events.try_send(TrashEvent::Progress { processed, total });
                }
                let recovery = store.recover_and_review();
                let _ = events.send_blocking(TrashEvent::Finished {
                    kind: TrashTaskKind::Delete,
                    completed,
                    cancelled,
                    failures,
                    recovery,
                });
            })
            .detach();
        self.receive_trash_events(event_rx, cx);
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
        if self.trash_view {
            self.operation_error = Some("Restore items before copying them".into());
            cx.notify();
            return;
        }
        self.clipboard = self.selected_paths();
        self.clip_cut = false;
        self.write_clip_text(cx);
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Use Restore to move an item out of Trash".into());
            cx.notify();
            return;
        }
        self.clipboard = self.selected_paths();
        self.clip_cut = true;
        self.write_clip_text(cx);
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
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
        for src in self.clipboard.clone() {
            if self.clip_cut && src.parent() == Some(self.cwd.as_path()) {
                continue;
            }
            let name = src.file_name().map(|n| n.to_owned()).unwrap_or_default();
            tasks.push(file_ops::TransferTask {
                kind: kind.clone(),
                source: src,
                destination: self.cwd.join(name),
            });
        }
        if tasks.is_empty() {
            if self.clip_cut {
                self.clipboard.clear();
                self.clip_cut = false;
                pasteboard::clear_file_urls();
                self.operation_notice =
                    Some("The items are already in this folder; nothing was moved".into());
                cx.notify();
            }
            return;
        }
        self.start_transfer_with_conflicts(
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
                    if this.trash_view && mode == ViewMode::Column {
                        this.operation_error = Some("Column view is unavailable in Trash".into());
                        cx.notify();
                        return;
                    }
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
                        nav(
                            "back",
                            "icons/chevron-left.svg",
                            self.trash_view || !self.back.is_empty(),
                        )
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
        let selected = if p.kind == PlaceKind::Trash {
            self.trash_view
        } else {
            !is_tag && !self.trash_view && self.cwd == p.path
        };
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
                PlaceKind::Trash => this.trash_click(cx),
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

    fn trash_click(&mut self, cx: &mut Context<Self>) {
        self.trash_view = true;
        if self.view == ViewMode::Column {
            self.view = ViewMode::List;
        }
        self.result_title = Some("Trash".into());
        self.operation_error = None;
        self.reload_trash(cx);
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
        trash_view: bool,
    ) -> rmac_ui::ContextMenu {
        let mut m = rmac_ui::ContextMenu::new(pos);
        if trash_view {
            if has_selection {
                m = m
                    .item("Restore", Box::new(RestoreItems))
                    .separator()
                    .danger_item("Delete Permanently…", Box::new(DeletePermanently));
            }
            return m;
        }
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
                    .when(!self.trash_view, |el: Stateful<Div>| {
                        el.on_drag(DraggedPaths(drag_paths), move |_, _, _, cx| {
                            cx.new(|_| DragPreview { count: drag_count })
                        })
                    })
                    .when(row_is_dir && !self.trash_view, |el: Stateful<Div>| {
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

        let content = if self.trash_view && self.entries.is_empty() {
            div()
                .id("trash-empty")
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    rmac_ui::EmptyState::new("Trash is Empty")
                        .message("Items moved to Trash will appear here."),
                )
                .into_any_element()
        } else {
            match self.view {
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
            }
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
            .on_action(cx.listener(|this, _: &RestoreItems, _, cx| this.restore_selected(cx)))
            .on_action(
                cx.listener(|this, _: &DeletePermanently, _, cx| this.request_permanent_delete(cx)),
            )
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
        if self.trash_view {
            return div()
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
                .text_color(secondary())
                .child(icon("icons/trash-2.svg", 12.0, secondary()))
                .child("Trash");
        }
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

    fn quick_look(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Restore items before previewing them".into());
            cx.notify();
            return;
        }
        let paths = self.selected_paths();
        if !paths.is_empty() {
            let _ = Command::new("qlmanage").arg("-p").args(&paths).spawn();
        }
    }

    fn drop_into(&mut self, dir: PathBuf, paths: &[PathBuf], cx: &mut Context<Self>) {
        let mut tasks = Vec::new();
        for src in paths {
            if src == &dir || src.parent() == Some(dir.as_path()) {
                continue;
            }
            let Some(name) = src.file_name() else {
                continue;
            };
            tasks.push(file_ops::TransferTask {
                kind: file_ops::TransferKind::Move,
                source: src.clone(),
                destination: dir.join(name),
            });
        }
        self.start_transfer_with_conflicts("Moving", tasks, false, cx);
    }

    /// Files dropped from another app (Finder, etc.) → copy into the current dir.
    fn drop_external(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut tasks = Vec::new();
        for src in paths {
            if let Some(name) = src.file_name().map(|name| name.to_owned()) {
                tasks.push(file_ops::TransferTask {
                    kind: file_ops::TransferKind::Copy,
                    source: src,
                    destination: self.cwd.join(name),
                });
            }
        }
        self.start_transfer_with_conflicts("Copying", tasks, false, cx);
    }

    fn get_info(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error =
                Some("Restore an item before viewing its file information".into());
            cx.notify();
            return;
        }
        self.info = self.selected.iter().next().copied();
        cx.notify();
    }

    /// Recursive platform search of the current folder tree (Return in the search box).
    fn recursive_search(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Trash search filters the current list as you type".into());
            cx.notify();
            return;
        }
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
        self.trash_view = false;
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
        self.trash_view = false;
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

    fn render_recovery(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.recovery_open {
            return None;
        }
        let review = self.recovery_reviews.first()?;
        let presentation = recovery_presentation(&review.action);
        let title = format!(
            "Recover File Operation (1 of {})",
            self.recovery_reviews.len()
        );
        let busy = self.recovery_busy;
        let buttons = vec![
            rmac_ui::dialog_button("recovery-later", "Later", rmac_ui::DialogButtonKind::Normal)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.close_recovery(cx)))
                .into_any_element(),
            rmac_ui::dialog_button(
                "recovery-confirm",
                if busy {
                    "Resolving…"
                } else {
                    presentation.action_label
                },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| this.resolve_current_recovery(cx)))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, presentation.message, buttons).into_any_element())
    }

    fn render_conflict(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let batch = self.conflict_batch.as_ref()?;
        let conflict = batch.conflicts.front()?;
        let current = batch
            .conflict_total
            .saturating_sub(batch.conflicts.len())
            .saturating_add(1);
        let title = format!(
            "An Item With This Name Already Exists ({current} of {})",
            batch.conflict_total
        );
        let busy = self.conflict_busy;
        let replace_available =
            conflict.destination_snapshot.is_some() && conflict.source != conflict.destination;
        let buttons = vec![
            rmac_ui::dialog_button("conflict-skip", "Skip", rmac_ui::DialogButtonKind::Normal)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.resolve_current_conflict(ConflictDecision::Skip, cx)
                }))
                .into_any_element(),
            rmac_ui::dialog_button(
                "conflict-replace",
                "Replace",
                rmac_ui::DialogButtonKind::Destructive,
            )
            .busy(busy)
            .disabled(busy || !replace_available)
            .on_click(cx.listener(|this, _, _, cx| {
                this.resolve_current_conflict(ConflictDecision::Replace, cx)
            }))
            .into_any_element(),
            rmac_ui::dialog_button(
                "conflict-keep-both",
                if busy { "Checking…" } else { "Keep Both" },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| {
                this.resolve_current_conflict(ConflictDecision::KeepBoth, cx)
            }))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, conflict_prompt(conflict), buttons).into_any_element())
    }

    #[cfg(any(target_os = "linux", test))]
    fn render_trash_recovery(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.trash_recovery_open || self.recovery_open {
            return None;
        }
        let review = self.trash_recovery_reviews.first()?;
        let presentation = trash_recovery_presentation(&review.action);
        let title = format!(
            "Recover Trash Operation (1 of {})",
            self.trash_recovery_reviews.len()
        );
        let busy = self.trash_recovery_busy;
        let resolvable = !matches!(
            &review.action,
            trash_store::TrashRecoveryAction::RequiresManualRepair
        );
        let buttons = vec![
            rmac_ui::dialog_button(
                "trash-recovery-later",
                "Later",
                rmac_ui::DialogButtonKind::Normal,
            )
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| this.close_trash_recovery(cx)))
            .into_any_element(),
            rmac_ui::dialog_button(
                "trash-recovery-confirm",
                if busy {
                    "Resolving…"
                } else {
                    presentation.action_label
                },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy || !resolvable)
            .on_click(cx.listener(|this, _, _, cx| this.resolve_current_trash_recovery(cx)))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, presentation.message, buttons).into_any_element())
    }

    #[cfg(any(target_os = "linux", test))]
    fn render_delete_confirmation(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let confirmation = self.delete_confirmation.as_ref()?;
        let count = confirmation.items.len();
        let name = confirmation
            .items
            .first()
            .and_then(|item| item.original_path.file_name())
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()));
        let title = if count == 1 {
            "Delete Item Permanently?"
        } else {
            "Delete Items Permanently?"
        };
        let buttons = vec![
            rmac_ui::dialog_button(
                "permanent-delete-cancel",
                "Cancel",
                rmac_ui::DialogButtonKind::Normal,
            )
            .on_click(cx.listener(|this, _, _, cx| this.cancel_permanent_delete(cx)))
            .into_any_element(),
            rmac_ui::dialog_button(
                "permanent-delete-confirm",
                "Delete",
                rmac_ui::DialogButtonKind::Destructive,
            )
            .on_click(cx.listener(|this, _, _, cx| this.confirm_permanent_delete(cx)))
            .into_any_element(),
        ];
        Some(
            rmac_ui::alert(
                title,
                permanent_delete_prompt(count, name.as_deref()),
                buttons,
            )
            .into_any_element(),
        )
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
        let operation_notice = self.operation_notice.clone();
        let operation_error = self.operation_error.clone();
        let transfer = self.transfer.clone();
        #[cfg(any(target_os = "linux", test))]
        let trash_progress = self.trash_operation.as_ref().map(|operation| {
            (
                operation.label.clone(),
                operation.processed,
                operation.total,
                operation.cancelling,
            )
        });
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_progress: Option<(SharedString, usize, usize, bool)> = None;
        let recovery_pending = self.pending_operations != 0;
        #[cfg(any(target_os = "linux", test))]
        let trash_recovery_pending = self.trash_pending != 0;
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_recovery_pending = false;
        let any_recovery_pending = recovery_pending || trash_recovery_pending;
        let conflict_dialog = self.render_conflict(cx);
        let recovery_dialog = self.render_recovery(cx);
        #[cfg(any(target_os = "linux", test))]
        let trash_recovery_dialog = self.render_trash_recovery(cx);
        #[cfg(not(any(target_os = "linux", test)))]
        let trash_recovery_dialog: Option<gpui::AnyElement> = None;
        #[cfg(any(target_os = "linux", test))]
        let delete_dialog = self.render_delete_confirmation(cx);
        #[cfg(not(any(target_os = "linux", test)))]
        let delete_dialog: Option<gpui::AnyElement> = None;
        div()
            .id("files-root")
            .size_full()
            .relative()
            .v_flex()
            .bg(list_bg())
            .text_color(label())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                #[cfg(any(target_os = "linux", test))]
                if this.delete_confirmation.is_some() {
                    cx.stop_propagation();
                    if event.keystroke.key.as_str() == "escape" {
                        this.cancel_permanent_delete(cx);
                    }
                    return;
                }
                if this.conflict_batch.is_some() {
                    cx.stop_propagation();
                    match conflict_key_intent(event.keystroke.key.as_str(), this.conflict_busy) {
                        Some(ConflictDecision::Skip) => {
                            this.resolve_current_conflict(ConflictDecision::Skip, cx)
                        }
                        Some(ConflictDecision::KeepBoth) => {
                            this.resolve_current_conflict(ConflictDecision::KeepBoth, cx)
                        }
                        _ => {}
                    }
                } else if this.recovery_open {
                    cx.stop_propagation();
                    match recovery_key_intent(event.keystroke.key.as_str(), this.recovery_busy) {
                        Some(RecoveryKeyIntent::Close) => this.close_recovery(cx),
                        Some(RecoveryKeyIntent::Resolve) => this.resolve_current_recovery(cx),
                        None => {}
                    }
                } else {
                    #[cfg(any(target_os = "linux", test))]
                    if this.trash_recovery_open {
                        cx.stop_propagation();
                        match recovery_key_intent(
                            event.keystroke.key.as_str(),
                            this.trash_recovery_busy,
                        ) {
                            Some(RecoveryKeyIntent::Close) => this.close_trash_recovery(cx),
                            Some(RecoveryKeyIntent::Resolve) => {
                                this.resolve_current_trash_recovery(cx)
                            }
                            None => {}
                        }
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                this.menu_at = None;
                cx.notify();
            }))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .child(self.render_toolbar(cx))
            .when_some(operation_notice, |el, message| {
                el.child(
                    div()
                        .id("operation-notice")
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
                        .cursor_pointer()
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(rmac_ui::mac::accent())
                                .text_color(rmac_ui::mac::on_accent())
                                .child("✓"),
                        )
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.operation_notice = None;
                            cx.notify();
                        })),
                )
            })
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
                        .child(if any_recovery_pending {
                            "Review"
                        } else {
                            "Dismiss"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if recovery_pending {
                                this.recovery_open = true;
                            } else {
                                #[cfg(any(target_os = "linux", test))]
                                if trash_recovery_pending {
                                    this.trash_recovery_open = true;
                                    cx.notify();
                                    return;
                                }
                                this.operation_error = None;
                            }
                            cx.notify();
                        })),
                )
            })
            .when_some(
                trash_progress,
                |el, (operation, processed, total, cancelling)| {
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
                            .child(
                                div()
                                    .flex_1()
                                    .child(format!("{operation} — {processed} of {total} items")),
                            )
                            .child(
                                div()
                                    .id("cancel-trash")
                                    .px_2()
                                    .py_0p5()
                                    .rounded(px(5.0))
                                    .bg(rmac_ui::mac::raised())
                                    .border_1()
                                    .border_color(rmac_ui::mac::accent_border())
                                    .cursor_pointer()
                                    .child(if cancelling {
                                        "Cancelling…"
                                    } else {
                                        "Cancel"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_trash(cx))),
                            ),
                    )
                },
            )
            .when_some(transfer, |el, transfer| {
                let action = if transfer.cancelling {
                    "Cancelling…"
                } else {
                    "Cancel"
                };
                let status = match transfer.phase {
                    file_ops::TransferPhase::Scanning => format!(
                        "{} — Scanning {} of {} items",
                        transfer.label, transfer.processed, transfer.total
                    ),
                    file_ops::TransferPhase::Copying if transfer.bytes_total > 0 => format!(
                        "{} — {} of {} · {} of {}",
                        transfer.label,
                        transfer.processed,
                        transfer.total,
                        human_size(transfer.bytes_processed),
                        human_size(transfer.bytes_total.max(transfer.bytes_processed))
                    ),
                    file_ops::TransferPhase::Copying => format!(
                        "{} — Copying {} of {} items",
                        transfer.label, transfer.processed, transfer.total
                    ),
                    file_ops::TransferPhase::Finishing => format!(
                        "{} — Finishing {} of {} items",
                        transfer.label, transfer.processed, transfer.total
                    ),
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
                        .child(div().flex_1().child(status))
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
            .when(multi, |el| el.child(self.render_tabs(cx)))
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
                el.child(
                    Self::build_context_menu(pos, has_sel, can_paste, self.trash_view).render(),
                )
            })
            .when_some(conflict_dialog, |el, dialog| el.child(dialog))
            .when_some(recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(trash_recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(delete_dialog, |el, dialog| el.child(dialog))
    }
}

// ---- helpers ----

struct RecoveryPresentation {
    message: String,
    action_label: &'static str,
}

fn sanitize_dialog_name(name: &str) -> String {
    let mut output = String::new();
    let mut truncated = false;
    for (index, character) in name.chars().enumerate() {
        if index == 120 {
            truncated = true;
            break;
        }
        output.push(if character.is_control() {
            '\u{fffd}'
        } else {
            character
        });
    }
    if truncated {
        output.push('…');
    }
    output
}

fn conflict_prompt(conflict: &TransferConflict) -> String {
    let name = conflict
        .destination
        .file_name()
        .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
        .unwrap_or_else(|| "this item".to_string());
    let folder = conflict
        .destination
        .parent()
        .and_then(Path::file_name)
        .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "the destination folder".to_string());
    let choice = if conflict.destination_snapshot.is_none() {
        "Another item in this batch needs the same name. Keep Both chooses an available numbered name. Replace is unavailable because no existing destination was reviewed."
    } else if conflict.kind == ConflictTransferKind::Move {
        "Keep Both moves this item under an available numbered name. Replace durably stages the move, atomically publishes it, and removes the reviewed previous item only after the source-removal boundary is safe. Replace cannot be undone yet. Skip leaves both items unchanged."
    } else if conflict.source == conflict.destination {
        "Keep Both creates a copy under an available numbered name. An item cannot replace itself, so Replace is disabled. Skip leaves it unchanged."
    } else {
        "Keep Both uses an available numbered name. Replace atomically publishes the new copy before removing the reviewed previous item. Replace cannot be undone yet. Skip leaves both items unchanged."
    };
    format!("An item named “{name}” already exists in “{folder}”. {choice}")
}

#[cfg(any(target_os = "linux", test))]
fn permanent_delete_prompt(count: usize, name: Option<&str>) -> String {
    if count == 1 {
        format!(
            "“{}” will be deleted immediately. This action cannot be undone. Deletion of an item cannot be cancelled once it begins.",
            name.unwrap_or("This item")
        )
    } else {
        format!(
            "{count} items will be deleted immediately. This action cannot be undone. Deletion of an item cannot be cancelled once it begins."
        )
    }
}

#[cfg(any(target_os = "linux", test))]
fn trash_recovery_presentation(action: &trash_store::TrashRecoveryAction) -> RecoveryPresentation {
    match action {
        trash_store::TrashRecoveryAction::ReturnRemainingItem {
            may_be_partial: true,
        } => RecoveryPresentation {
            message: "Files found remaining data from an interrupted permanent deletion. It may be incomplete. Return it to Trash without deleting or replacing any existing item."
                .to_string(),
            action_label: "Return to Trash",
        },
        trash_store::TrashRecoveryAction::ReturnRemainingItem {
            may_be_partial: false,
        } => RecoveryPresentation {
            message: "Files found an item hidden by an interrupted permanent deletion. Return it to Trash without deleting or replacing any existing item."
                .to_string(),
            action_label: "Return to Trash",
        },
        trash_store::TrashRecoveryAction::RemoveOrphanMetadata => RecoveryPresentation {
            message: "No file data remains for this transaction. Remove only its reviewed Trash metadata; no user file will be deleted."
                .to_string(),
            action_label: "Remove Metadata",
        },
        trash_store::TrashRecoveryAction::KeepExistingItems => RecoveryPresentation {
            message: "Keep every existing source, Trash, and destination item. Files will clear only the exact recovery record and will not delete, move, or replace a file."
                .to_string(),
            action_label: "Keep Existing Items",
        },
        trash_store::TrashRecoveryAction::RequiresManualRepair => RecoveryPresentation {
            message: "Files cannot prove a safe automatic repair for this state. Keep it for later; no item or recovery record will be changed."
                .to_string(),
            action_label: "Manual Repair Required",
        },
    }
}

fn recovery_presentation(action: &operation_journal::RecoveryAction) -> RecoveryPresentation {
    match action {
        operation_journal::RecoveryAction::PreserveCopy {
            complete,
            suggested_name,
        } => RecoveryPresentation {
            message: if *complete {
                format!(
                    "Files has a complete copy from an interrupted file operation. Preserve it as “{suggested_name}”. Existing items will not be changed."
                )
            } else {
                format!(
                    "The interrupted copy may be incomplete. Preserve it as “{suggested_name}” so you can inspect it. Existing items will not be changed."
                )
            },
            action_label: "Preserve Copy",
        },
        operation_journal::RecoveryAction::PreserveReplacementBackup {
            complete,
            suggested_name,
        } => RecoveryPresentation {
            message: if *complete {
                format!(
                    "Files retained the previous destination from an interrupted replacement. Preserve it as “{suggested_name}”. The replacement and other existing items will not be changed."
                )
            } else {
                format!(
                    "The item retained from an interrupted replacement may have changed or may be incomplete. Preserve it as “{suggested_name}” so you can inspect it. Existing items will not be changed."
                )
            },
            action_label: "Preserve Previous Item",
        },
        operation_journal::RecoveryAction::KeepExistingItems => RecoveryPresentation {
            message: "No staged recovery copy remains. Keep every existing item and clear only this recovery record. No file will be deleted.".to_string(),
            action_label: "Keep Existing Items",
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryKeyIntent {
    Close,
    Resolve,
}

fn recovery_key_intent(key: &str, busy: bool) -> Option<RecoveryKeyIntent> {
    if busy {
        return None;
    }
    match key {
        "escape" => Some(RecoveryKeyIntent::Close),
        "enter" => Some(RecoveryKeyIntent::Resolve),
        _ => None,
    }
}

fn conflict_key_intent(key: &str, busy: bool) -> Option<ConflictDecision> {
    if busy {
        return None;
    }
    match key {
        "escape" => Some(ConflictDecision::Skip),
        "enter" => Some(ConflictDecision::KeepBoth),
        _ => None,
    }
}

fn unique_path(path: PathBuf) -> PathBuf {
    unique_path_avoiding(path, &BTreeSet::new())
}

fn prepare_conflict_batch(
    label: &'static str,
    tasks: Vec<file_ops::TransferTask>,
    keep_unfinished_in_clipboard: bool,
) -> std::io::Result<ConflictBatch> {
    let mut ready = Vec::with_capacity(tasks.len());
    let mut conflicts = VecDeque::new();
    let mut reserved_destinations = BTreeSet::new();

    for task in tasks {
        let requested_destination = task.destination.clone();
        let kind = match &task.kind {
            file_ops::TransferKind::Copy => ConflictTransferKind::Copy,
            file_ops::TransferKind::Move => ConflictTransferKind::Move,
            file_ops::TransferKind::Replace(_) | file_ops::TransferKind::MoveReplace(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "replacement task cannot enter conflict preflight",
                ));
            }
        };
        let destination_exists = match std::fs::symlink_metadata(&task.destination) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error),
        };
        if destination_exists || reserved_destinations.contains(&task.destination) {
            let source_snapshot = operation_journal::TreeSnapshot::capture(&task.source)?;
            let destination_snapshot = destination_exists
                .then(|| operation_journal::TreeSnapshot::capture(&task.destination))
                .transpose()?;
            conflicts.push_back(TransferConflict {
                kind,
                source: task.source,
                destination: requested_destination.clone(),
                source_snapshot,
                destination_snapshot,
            });
        } else {
            ready.push(task);
        }
        reserved_destinations.insert(requested_destination);
    }
    let conflict_total = conflicts.len();
    Ok(ConflictBatch {
        label,
        ready,
        conflicts,
        conflict_total,
        reserved_destinations,
        skipped_moves: Vec::new(),
        keep_unfinished_in_clipboard,
    })
}

fn resolve_conflict_task(
    conflict: &TransferConflict,
    decision: ConflictDecision,
    reserved: &BTreeSet<PathBuf>,
) -> std::io::Result<Option<file_ops::TransferTask>> {
    if !conflict.source_snapshot.still_matches(&conflict.source)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "conflict source changed",
        ));
    }
    let destination_matches = match &conflict.destination_snapshot {
        Some(snapshot) => snapshot.still_matches(&conflict.destination)?,
        None => match std::fs::symlink_metadata(&conflict.destination) {
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(error),
        },
    };
    if !destination_matches {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "conflict destination changed",
        ));
    }

    match decision {
        ConflictDecision::KeepBoth => {
            let destination = unique_path_avoiding(conflict.destination.clone(), reserved);
            Ok(Some(file_ops::TransferTask {
                kind: match conflict.kind {
                    ConflictTransferKind::Copy => file_ops::TransferKind::Copy,
                    ConflictTransferKind::Move => file_ops::TransferKind::Move,
                },
                source: conflict.source.clone(),
                destination,
            }))
        }
        ConflictDecision::Replace => {
            let destination_snapshot = conflict.destination_snapshot.clone().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "batch-only conflict cannot replace a destination",
                )
            })?;
            if conflict.source == conflict.destination {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "safe replacement is unavailable for this transfer",
                ));
            }
            Ok(Some(file_ops::TransferTask {
                kind: match conflict.kind {
                    ConflictTransferKind::Copy => {
                        file_ops::TransferKind::Replace(Box::new(file_ops::ReplacementBinding {
                            expected_source: conflict.source_snapshot.clone(),
                            expected_destination: destination_snapshot,
                        }))
                    }
                    ConflictTransferKind::Move => file_ops::TransferKind::MoveReplace(Box::new(
                        file_ops::ReplacementBinding {
                            expected_source: conflict.source_snapshot.clone(),
                            expected_destination: destination_snapshot,
                        },
                    )),
                },
                source: conflict.source.clone(),
                destination: conflict.destination.clone(),
            }))
        }
        ConflictDecision::Skip => Ok(None),
    }
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

/// Copy without following symlinks or replacing any destination entry.
///
/// Every destination node is created exclusively. A concurrent writer can
/// therefore make the operation fail, but can never have its data overwritten.
fn copy_item(src: &Path, dst: &Path) -> std::io::Result<()> {
    copy_item_cancellable(src, dst, &AtomicBool::new(false), &mut |_| {})
}

fn copy_item_cancellable(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(file_ops::CopyActivity),
) -> std::io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "copy cancelled",
        ));
    }
    validate_copy_destination(src, dst)?;
    copy_recursive_cancellable(src, dst, cancel, progress)?;
    progress(file_ops::CopyActivity::Finishing);
    sync_copied_tree(dst)?;
    if let Some(parent) = dst.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Reject a directory copy into itself or any real descendant before creating
/// the first destination node. Canonicalizing the existing destination parent
/// also catches a path routed back into the source through a symlink.
fn validate_copy_destination(src: &Path, dst: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(src)?;
    if !metadata.is_dir() {
        return Ok(());
    }
    let source = std::fs::canonicalize(src)?;
    let parent = dst.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "copy destination has no parent",
        )
    })?;
    let destination_name = dst.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "copy destination has no file name",
        )
    })?;
    let destination = std::fs::canonicalize(parent)?.join(destination_name);
    if destination == source || destination.starts_with(&source) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a folder cannot be copied into itself",
        ));
    }
    Ok(())
}

fn copy_recursive_cancellable(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(file_ops::CopyActivity),
) -> std::io::Result<()> {
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
            copy_recursive_cancellable(&e.path(), &dst.join(e.file_name()), cancel, progress)?;
        }
        std::fs::set_permissions(dst, metadata.permissions())?;
    } else if metadata.is_file() {
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
            progress(file_ops::CopyActivity::Bytes(read as u64));
        }
        destination.sync_all()?;
        std::fs::set_permissions(dst, metadata.permissions())?;
    } else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "special files cannot be copied",
        ));
    }
    Ok(())
}

/// Durably flush the completed copy before a cross-volume move can remove its
/// source. Symlinks are never opened or followed; their directory entry is
/// covered by the parent-directory sync.
fn sync_copied_tree(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            sync_copied_tree(&entry?.path())?;
        }
        std::fs::File::open(path)?.sync_all()?;
    } else if !metadata.file_type().is_symlink() {
        std::fs::File::open(path)?.sync_all()?;
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

/// Bytes available to an unprivileged process on the volume containing
/// `path`. Use the filesystem authority directly rather than parsing localized
/// command output.
fn free_space(path: &Path) -> Option<u64> {
    let stats = rustix::fs::statvfs(path).ok()?;
    let fragment_size = if stats.f_frsize == 0 {
        stats.f_bsize
    } else {
        stats.f_frsize
    };
    stats.f_bavail.checked_mul(fragment_size)
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
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should follow the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rmac-files-main-{label}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }

        fn new_short(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock should follow the Unix epoch")
                .as_nanos();
            let path =
                PathBuf::from("/tmp").join(format!("rmac-{label}-{}-{unique}", std::process::id()));
            std::fs::create_dir(&path).expect("short test directory should be created");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

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

    #[test]
    fn existing_transfer_destination_opens_a_bound_keep_replace_skip_conflict() {
        let root = TestDirectory::new("conflict-preflight");
        let source = root.0.join("incoming").join("report.txt");
        let destination = root.0.join("destination").join("report.txt");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&source, b"incoming bytes").unwrap();
        std::fs::write(&destination, b"previous bytes").unwrap();

        let batch = prepare_conflict_batch(
            "Copying",
            vec![file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: source.clone(),
                destination: destination.clone(),
            }],
            false,
        )
        .unwrap();
        let conflict = batch.conflicts.front().unwrap();

        assert_eq!(batch.conflict_total, 1);
        assert!(batch.ready.is_empty());
        assert!(conflict.destination_snapshot.is_some());
        assert!(conflict_prompt(conflict).contains("Keep Both"));
        assert!(conflict_prompt(conflict).contains("Replace"));

        let keep_both = resolve_conflict_task(
            conflict,
            ConflictDecision::KeepBoth,
            &batch.reserved_destinations,
        )
        .unwrap()
        .unwrap();
        assert_eq!(keep_both.kind, file_ops::TransferKind::Copy);
        assert!(keep_both.destination.ends_with("report 2.txt"));

        let replace = resolve_conflict_task(
            conflict,
            ConflictDecision::Replace,
            &batch.reserved_destinations,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(replace.kind, file_ops::TransferKind::Replace(_)));
        assert_eq!(std::fs::read(source).unwrap(), b"incoming bytes");
        assert_eq!(std::fs::read(destination).unwrap(), b"previous bytes");
    }

    #[test]
    fn conflict_decision_rejects_a_nested_destination_change() {
        let root = TestDirectory::new("conflict-destination-change");
        let source = root.0.join("incoming");
        let destination = root.0.join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(source.join("new"), b"new bytes").unwrap();
        std::fs::write(destination.join("old"), b"previous bytes").unwrap();
        let batch = prepare_conflict_batch(
            "Copying",
            vec![file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: source.clone(),
                destination: destination.clone(),
            }],
            false,
        )
        .unwrap();
        let conflict = batch.conflicts.front().unwrap();
        std::fs::write(destination.join("changed"), b"racing bytes").unwrap();

        let error = resolve_conflict_task(
            conflict,
            ConflictDecision::Replace,
            &batch.reserved_destinations,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(std::fs::read(source.join("new")).unwrap(), b"new bytes");
        assert_eq!(
            std::fs::read(destination.join("old")).unwrap(),
            b"previous bytes"
        );
        assert_eq!(
            std::fs::read(destination.join("changed")).unwrap(),
            b"racing bytes"
        );
    }

    #[test]
    fn duplicate_names_inside_one_batch_require_a_non_destructive_choice() {
        let root = TestDirectory::new("conflict-batch-name");
        let first = root.0.join("one").join("item");
        let second = root.0.join("two").join("item");
        let destination = root.0.join("destination").join("item");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        let batch = prepare_conflict_batch(
            "Copying",
            vec![
                file_ops::TransferTask {
                    kind: file_ops::TransferKind::Copy,
                    source: first,
                    destination: destination.clone(),
                },
                file_ops::TransferTask {
                    kind: file_ops::TransferKind::Copy,
                    source: second,
                    destination: destination.clone(),
                },
            ],
            false,
        )
        .unwrap();
        let conflict = batch.conflicts.front().unwrap();

        assert_eq!(batch.ready.len(), 1);
        assert_eq!(batch.conflict_total, 1);
        assert!(conflict.destination_snapshot.is_none());
        assert!(resolve_conflict_task(
            conflict,
            ConflictDecision::Replace,
            &batch.reserved_destinations,
        )
        .is_err());
        let keep_both = resolve_conflict_task(
            conflict,
            ConflictDecision::KeepBoth,
            &batch.reserved_destinations,
        )
        .unwrap()
        .unwrap();
        assert!(keep_both.destination.ends_with("item 2"));
        assert!(!destination.exists());
    }

    #[test]
    fn moved_item_conflict_offers_only_snapshot_bound_replacement() {
        let root = TestDirectory::new("move-conflict");
        let source = root.0.join("incoming").join("item");
        let destination = root.0.join("destination").join("item");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(&source, b"incoming").unwrap();
        std::fs::write(&destination, b"existing").unwrap();
        let batch = prepare_conflict_batch(
            "Moving",
            vec![file_ops::TransferTask {
                kind: file_ops::TransferKind::Move,
                source,
                destination,
            }],
            true,
        )
        .unwrap();
        let conflict = batch.conflicts.front().unwrap();

        assert!(conflict_prompt(conflict).contains("durably stages the move"));
        let replacement = resolve_conflict_task(
            conflict,
            ConflictDecision::Replace,
            &batch.reserved_destinations,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            replacement.kind,
            file_ops::TransferKind::MoveReplace(_)
        ));
    }

    #[test]
    fn conflict_keyboard_policy_uses_safe_defaults_and_blocks_while_busy() {
        assert_eq!(
            conflict_key_intent("escape", false),
            Some(ConflictDecision::Skip)
        );
        assert_eq!(
            conflict_key_intent("enter", false),
            Some(ConflictDecision::KeepBoth)
        );
        assert_eq!(conflict_key_intent("space", false), None);
        assert_eq!(conflict_key_intent("escape", true), None);
        assert_eq!(conflict_key_intent("enter", true), None);
    }

    #[test]
    fn recursive_copy_refuses_a_destination_inside_the_source() {
        let root = TestDirectory::new("copy-descendant");
        let source = root.0.join("source");
        let child = source.join("child");
        let destination = child.join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&child).unwrap();
        std::fs::write(source.join("keep"), b"source bytes").unwrap();

        let error = copy_item(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!destination.exists());
        assert_eq!(std::fs::read(source.join("keep")).unwrap(), b"source bytes");
    }

    #[test]
    fn recursive_copy_detects_a_descendant_reached_through_a_symlink() {
        let root = TestDirectory::new("copy-symlink-descendant");
        let source = root.0.join("source");
        let child = source.join("child");
        let routed_parent = root.0.join("routed-parent");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&child).unwrap();
        std::os::unix::fs::symlink(&child, &routed_parent).unwrap();
        let destination = routed_parent.join("source");

        let error = copy_item(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!child.join("source").exists());
    }

    #[test]
    fn recursive_copy_preserves_a_symlink_without_traversing_its_target() {
        let root = TestDirectory::new("copy-symlink");
        let target = root.0.join("target");
        let source = root.0.join("source-link");
        let destination = root.0.join("destination-link");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("private"), b"target bytes").unwrap();
        std::os::unix::fs::symlink("target", &source).unwrap();

        copy_item(&source, &destination).unwrap();

        assert!(std::fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read_link(&destination).unwrap(),
            PathBuf::from("target")
        );
    }

    #[test]
    fn recursive_copy_never_replaces_an_existing_file() {
        let root = TestDirectory::new("copy-conflict");
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        std::fs::write(&source, b"source bytes").unwrap();
        std::fs::write(&destination, b"destination bytes").unwrap();

        let error = copy_item(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&destination).unwrap(), b"destination bytes");
    }

    #[test]
    fn recursive_copy_refuses_special_files_without_opening_them() {
        let root = TestDirectory::new_short("special");
        let source = root.0.join("source.socket");
        let destination = root.0.join("destination");
        let _listener = std::os::unix::net::UnixListener::bind(&source).unwrap();

        let error = copy_item(&source, &destination).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
        assert!(!destination.exists());
    }

    #[test]
    fn recovery_keyboard_policy_blocks_shortcuts_while_busy() {
        assert_eq!(
            recovery_key_intent("escape", false),
            Some(RecoveryKeyIntent::Close)
        );
        assert_eq!(
            recovery_key_intent("enter", false),
            Some(RecoveryKeyIntent::Resolve)
        );
        assert_eq!(recovery_key_intent("space", false), None);
        assert_eq!(recovery_key_intent("escape", true), None);
        assert_eq!(recovery_key_intent("enter", true), None);
    }

    #[test]
    fn recovery_presentation_never_claims_a_partial_copy_is_complete() {
        let partial = recovery_presentation(&operation_journal::RecoveryAction::PreserveCopy {
            complete: false,
            suggested_name: "Recovered item".to_string(),
        });
        let complete = recovery_presentation(&operation_journal::RecoveryAction::PreserveCopy {
            complete: true,
            suggested_name: "Recovered item".to_string(),
        });
        let replacement = recovery_presentation(
            &operation_journal::RecoveryAction::PreserveReplacementBackup {
                complete: true,
                suggested_name: "Recovered item".to_string(),
            },
        );
        let existing = recovery_presentation(&operation_journal::RecoveryAction::KeepExistingItems);

        assert!(partial.message.contains("may be incomplete"));
        assert!(!complete.message.contains("may be incomplete"));
        assert!(complete.message.contains("complete copy"));
        assert!(replacement.message.contains("previous destination"));
        assert_eq!(replacement.action_label, "Preserve Previous Item");
        assert!(existing.message.contains("No file will be deleted"));
    }

    #[test]
    fn permanent_delete_prompt_is_explicitly_irreversible_and_path_free() {
        let single = permanent_delete_prompt(1, Some("report.txt"));
        let multiple = permanent_delete_prompt(3, None);

        assert!(single.contains("“report.txt”"));
        assert!(single.contains("cannot be undone"));
        assert!(!single.contains("/home/"));
        assert_eq!(
            multiple,
            "3 items will be deleted immediately. This action cannot be undone. Deletion of an item cannot be cancelled once it begins."
        );
    }

    #[test]
    fn permanent_delete_dialog_name_replaces_controls_and_bounds_length() {
        assert_eq!(sanitize_dialog_name("line\nbreak"), "line\u{fffd}break");
        let long = "a".repeat(121);
        let sanitized = sanitize_dialog_name(&long);

        assert_eq!(sanitized.chars().count(), 121);
        assert!(sanitized.ends_with('…'));
    }

    #[test]
    fn trash_recovery_presentations_never_overstate_safe_actions() {
        let partial =
            trash_recovery_presentation(&trash_store::TrashRecoveryAction::ReturnRemainingItem {
                may_be_partial: true,
            });
        let orphan =
            trash_recovery_presentation(&trash_store::TrashRecoveryAction::RemoveOrphanMetadata);
        let keep =
            trash_recovery_presentation(&trash_store::TrashRecoveryAction::KeepExistingItems);
        let manual =
            trash_recovery_presentation(&trash_store::TrashRecoveryAction::RequiresManualRepair);

        assert!(partial.message.contains("may be incomplete"));
        assert!(partial.message.contains("without deleting or replacing"));
        assert!(orphan.message.contains("no user file will be deleted"));
        assert!(keep
            .message
            .contains("clear only the exact recovery record"));
        assert!(keep.message.contains("will not delete, move, or replace"));
        assert!(manual
            .message
            .contains("cannot prove a safe automatic repair"));
        assert!(manual
            .message
            .contains("no item or recovery record will be changed"));
    }
}
