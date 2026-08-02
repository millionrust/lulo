//! Finder window, controller, and rendering ownership.
//!
//! Beyond the pixel-accurate chrome: multi-selection, file operations
//! (new folder, rename, duplicate, copy/cut/paste, move-to-trash), keyboard
//! shortcuts + right-click context menus, live search, clickable sort headers,
//! hidden-file toggle, and live directory watching.

mod filesystem_helpers;
mod navigation;
mod operations;
mod presentation;
mod updates;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

use crate::conflict::{
    conflict_prompt, prepare_conflict_batch, resolve_conflict_task, unique_path_avoiding,
    ConflictBatch, ConflictDecision, ConflictTransferKind,
};
#[cfg(any(target_os = "linux", test))]
use crate::recovery_ui::trash_recovery_presentation;
use crate::recovery_ui::{
    conflict_key_intent, recovery_key_intent, recovery_presentation, RecoveryKeyIntent,
};
#[cfg(any(target_os = "linux", test))]
use crate::trash_store;
#[cfg(target_os = "linux")]
use crate::watchers::MOUNT_WATCH_UNAVAILABLE_MESSAGE;
use crate::watchers::{
    filesystem_watcher, FilesystemHints, DIRECTORY_STALL_NOTICE, DIRECTORY_STALL_NOTICE_DELAY,
    FILESYSTEM_WATCH_INTERRUPTED_MESSAGE, FILESYSTEM_WATCH_UNAVAILABLE_MESSAGE,
};
#[cfg(target_os = "linux")]
use crate::watchers::{next_mount_watch_retry, MountWatchHealth, MountWatchNotice};
use crate::{directory_state, file_ops, operation_journal, pasteboard, quick_look, undo_journal};
use filesystem_helpers::*;

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
        UndoOperation,
        SelectAll,
        GoUp,
        ToggleHidden,
        OpenItems,
        OpenWith,
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
        undo_availability: std::io::Result<Option<undo_journal::UndoAvailability>>,
    },
}

enum UndoEvent {
    Progress(file_ops::CopyActivity),
    Finished {
        outcome: std::io::Result<Option<undo_journal::UndoOutcome>>,
        availability: std::io::Result<Option<undo_journal::UndoAvailability>>,
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
struct TrashCompletion {
    kind: TrashTaskKind,
    completed: usize,
    cancelled: bool,
    failures: Vec<file_ops::Failure>,
    recovery: std::io::Result<(
        trash_store::TrashRecovery,
        Vec<trash_store::TrashRecoveryReview>,
    )>,
    undo_availability: std::io::Result<Option<undo_journal::UndoAvailability>>,
}

#[cfg(any(target_os = "linux", test))]
enum TrashEvent {
    Progress { processed: usize, total: usize },
    Finished(TrashCompletion),
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

#[derive(Clone)]
struct ActiveUndo {
    label: SharedString,
    phase: file_ops::TransferPhase,
    bytes_processed: u64,
    cancel: Arc<AtomicBool>,
    cancelling: bool,
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

#[derive(Clone)]
struct OpenWithPicker {
    path: PathBuf,
    association: Option<rmac_apps::FileAssociation>,
    selected: usize,
    make_default: bool,
    busy: bool,
    error: Option<SharedString>,
}

#[derive(Clone)]
struct QuickLookPanel {
    paths: Vec<PathBuf>,
    current: usize,
    content: Option<quick_look::Content>,
    error: Option<SharedString>,
    cancel: Arc<AtomicBool>,
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
    search_detail: Option<SharedString>,
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
    identity: Option<directory_state::Identity>,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
}

struct FinderView {
    cwd: PathBuf,
    tabs: Vec<Tab>,
    active: usize,
    home: PathBuf,
    mounts: Vec<rmac_mounts::Mount>,
    mount_generation: u64,
    #[cfg(target_os = "linux")]
    mount_watch_health: MountWatchHealth,
    cwd_identity: Option<directory_state::Identity>,
    directory_generation: u64,
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
    open_with: Option<OpenWithPicker>,
    open_generation: u64,
    quick_look: Option<QuickLookPanel>,
    quick_look_generation: u64,
    result_title: Option<SharedString>,
    search_summary: Option<SharedString>,
    search_relevance_order: bool,
    operation_notice: Option<SharedString>,
    operation_error: Option<SharedString>,
    operation_journal: Option<Arc<operation_journal::Journal>>,
    journal_loading: bool,
    undo_available: Option<undo_journal::UndoAvailability>,
    undo_operation: Option<ActiveUndo>,
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
    filesystem_events: async_channel::Sender<()>,
    filesystem_hints: Arc<Mutex<FilesystemHints>>,
    watched: Option<PathBuf>,
    watched_parent: Option<PathBuf>,
    search_generation: u64,
    search_cancel: Option<Arc<AtomicBool>>,
}

impl Drop for FinderView {
    fn drop(&mut self) {
        if let Some(transfer) = &self.transfer {
            transfer.cancel.store(true, Ordering::Release);
        }
        if let Some(undo) = &self.undo_operation {
            undo.cancel.store(true, Ordering::Release);
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
        locations.extend(mounts.iter().cloned().map(|mount| {
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
            KeyBinding::new(
                rmac_ui::shortcuts::SELECT_ALL.keystroke,
                SelectAll,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::COPY.keystroke,
                CopyItems,
                Some("Finder"),
            ),
            KeyBinding::new(rmac_ui::shortcuts::CUT.keystroke, CutItems, Some("Finder")),
            KeyBinding::new(
                rmac_ui::shortcuts::PASTE.keystroke,
                PasteItems,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::UNDO.keystroke,
                UndoOperation,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::DUPLICATE.keystroke,
                Duplicate,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::DELETE.keystroke,
                MoveToTrash,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::DELETE_PERMANENT.keystroke,
                DeletePermanently,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::NEW_FOLDER.keystroke,
                NewFolder,
                Some("Finder"),
            ),
            KeyBinding::new(rmac_ui::shortcuts::GO_UP.keystroke, GoUp, Some("Finder")),
            KeyBinding::new(
                rmac_ui::shortcuts::OPEN_SELECTION.keystroke,
                OpenItems,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::ENTER.keystroke,
                RenameItem,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::TOGGLE_HIDDEN.keystroke,
                ToggleHidden,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::SPACE.keystroke,
                QuickLook,
                Some("Finder"),
            ),
            KeyBinding::new(rmac_ui::shortcuts::INFO.keystroke, GetInfo, Some("Finder")),
            KeyBinding::new(
                rmac_ui::shortcuts::NEW_TAB.keystroke,
                NewTab,
                Some("Finder"),
            ),
            KeyBinding::new(
                rmac_ui::shortcuts::CLOSE.keystroke,
                CloseTab,
                Some("Finder"),
            ),
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
        let fs_hints = Arc::new(Mutex::new(FilesystemHints::default()));
        let watcher = filesystem_watcher(fs_events.clone(), fs_hints.clone()).ok();
        #[cfg(target_os = "linux")]
        let (mount_events, mount_event_rx) = async_channel::bounded(8);

        let focus = cx.focus_handle();
        window.focus(&focus);

        let mut view = Self {
            cwd: home.clone(),
            tabs: vec![Tab {
                cwd: home.clone(),
                identity: None,
                back: Vec::new(),
                fwd: Vec::new(),
            }],
            active: 0,
            home: home.clone(),
            mounts: mounts.clone(),
            mount_generation: 0,
            #[cfg(target_os = "linux")]
            mount_watch_health: MountWatchHealth::default(),
            cwd_identity: None,
            directory_generation: 0,
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
            open_with: None,
            open_generation: 0,
            quick_look: None,
            quick_look_generation: 0,
            result_title: None,
            search_summary: None,
            search_relevance_order: false,
            operation_notice: None,
            operation_error: mount_error,
            operation_journal: None,
            journal_loading: true,
            undo_available: None,
            undo_operation: None,
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
            filesystem_events: fs_events,
            filesystem_hints: fs_hints.clone(),
            watched: None,
            watched_parent: None,
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
                    let undo = journal.undo_store().latest()?;
                    Ok::<_, std::io::Error>((journal, recovery, reviews, undo))
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.journal_loading = false;
                match result {
                    Ok((journal, recovery, reviews, undo)) => {
                        this.operation_journal = Some(journal);
                        this.undo_available = undo;
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
                        this.undo_available = None;
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
                    let undo_availability = store.undo_store().latest();
                    Ok::<_, std::io::Error>((store, recovery, undo_availability))
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.trash_loading = false;
                match result {
                    Ok((store, recovery, undo_availability)) => match recovery {
                        Ok((recovery, reviews)) => {
                            this.trash_store = Some(store);
                            match undo_availability {
                                Ok(availability) => this.undo_available = availability,
                                Err(_) => {
                                    this.undo_available = None;
                                    this.operation_journal = None;
                                }
                            }
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

        // Live directory watching → identity-bound reload/recovery on changes.
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
                let hints = fs_hints
                    .lock()
                    .map(|mut hints| std::mem::take(&mut *hints))
                    .unwrap_or_default();
                if this
                    .update(cx, |this: &mut FinderView, cx| {
                        this.reload_after_event(hints, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        #[cfg(target_os = "linux")]
        {
            let watch_sender = mount_events.clone();
            cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                let mut failures = 0;
                loop {
                    let started = std::time::Instant::now();
                    let _ = rmac_mounts::watch(watch_sender.clone()).await;
                    if watch_sender.is_closed() {
                        break;
                    }
                    let retry = next_mount_watch_retry(failures, started.elapsed());
                    failures = retry.0;
                    cx.background_executor().timer(retry.1).await;
                }
            })
            .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(mut event) = mount_event_rx.recv().await {
                    while let Ok(next) = mount_event_rx.try_recv() {
                        event = next;
                    }
                    if this
                        .update(cx, |this: &mut FinderView, cx| {
                            match this.mount_watch_health.record(event) {
                                MountWatchNotice::Unavailable => {
                                    if this.operation_error.is_none() {
                                        this.operation_error =
                                            Some(MOUNT_WATCH_UNAVAILABLE_MESSAGE.into());
                                    }
                                }
                                MountWatchNotice::Restored => {
                                    if this.operation_error.as_ref().is_some_and(|message| {
                                        message.as_ref() == MOUNT_WATCH_UNAVAILABLE_MESSAGE
                                    }) {
                                        this.operation_error = None;
                                    }
                                    this.operation_notice =
                                        Some("Automatic mounted-volume updates resumed".into());
                                }
                                MountWatchNotice::None => {}
                            }
                            this.refresh_mounts(cx);
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
            view.refresh_mounts(cx);
        }

        view
    }

    fn request_open_with(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Restore the item before choosing an application".into());
            cx.notify();
            return;
        }
        let mut selected = self
            .selected
            .iter()
            .filter_map(|&index| self.entries.get(index));
        let Some(entry) = selected.next() else {
            return;
        };
        if selected.next().is_some() || entry.is_dir {
            self.operation_error =
                Some("Select one file to choose which application opens it".into());
            cx.notify();
            return;
        }

        let path = entry.path.clone();
        self.menu_at = None;
        self.operation_error = None;
        self.open_with = Some(OpenWithPicker {
            path: path.clone(),
            association: None,
            selected: 0,
            make_default: false,
            busy: false,
            error: None,
        });
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_app_launch::file_association(path.clone()).await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                let Some(picker) = this.open_with.as_mut().filter(|picker| picker.path == path)
                else {
                    return;
                };
                match result {
                    Ok(association) => {
                        picker.association = Some(association);
                        picker.selected = 0;
                    }
                    Err(error) => {
                        this.open_with = None;
                        this.operation_error =
                            Some(format!("Could not load compatible applications: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_open_with(&mut self, cx: &mut Context<Self>) {
        if self.open_with.as_ref().is_some_and(|picker| picker.busy) {
            return;
        }
        self.open_with = None;
        cx.notify();
    }

    fn move_open_with_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        let Some(association) = picker.association.as_ref() else {
            return;
        };
        if picker.busy || association.handlers.is_empty() {
            return;
        }
        picker.selected = picker
            .selected
            .saturating_add_signed(delta)
            .min(association.handlers.len() - 1);
        cx.notify();
    }

    fn choose_open_with(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        let Some(association) = picker.association.as_ref() else {
            return;
        };
        if !picker.busy && index < association.handlers.len() {
            picker.selected = index;
            picker.error = None;
            cx.notify();
        }
    }

    fn toggle_open_with_default(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        let Some(association) = picker.association.as_ref() else {
            return;
        };
        let Some(application) = association.handlers.get(picker.selected) else {
            return;
        };
        if !picker.busy
            && association.default_application_id.as_deref() != Some(application.id.as_str())
        {
            picker.make_default = !picker.make_default;
            picker.error = None;
            cx.notify();
        }
    }

    fn confirm_open_with(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.open_with.as_mut() else {
            return;
        };
        let Some(association) = picker.association.as_ref() else {
            return;
        };
        let Some(application) = association.handlers.get(picker.selected) else {
            return;
        };
        if picker.busy {
            return;
        }
        let path = picker.path.clone();
        let mime_type = association.mime_type.clone();
        let application_id = application.id.clone();
        let application_name = sanitize_dialog_name(&application.name);
        let make_default = picker.make_default
            && association.default_application_id.as_deref() != Some(application.id.as_str());
        picker.busy = true;
        picker.error = None;
        self.operation_error = None;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_app_launch::open_file_with(
                path.clone(),
                mime_type.clone(),
                application_id.clone(),
                make_default,
            )
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                let Some(picker) = this.open_with.as_mut().filter(|picker| picker.path == path)
                else {
                    return;
                };
                picker.busy = false;
                match result {
                    Ok(()) => {
                        this.open_with = None;
                        this.operation_notice = Some(
                            if make_default {
                                format!(
                                    "{application_name} is now the default for {mime_type} files"
                                )
                            } else {
                                format!("Opened with {application_name}")
                            }
                            .into(),
                        );
                    }
                    Err(error) => {
                        if error.default_changed {
                            if let Some(association) = picker.association.as_mut() {
                                association.default_application_id = Some(application_id.clone());
                            }
                            picker.make_default = false;
                        }
                        picker.error = Some(format!("Could not open the file: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
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
        self.search_summary = None;
        self.search_relevance_order = false;
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

    fn refresh_mounts(&mut self, cx: &mut Context<Self>) {
        self.mount_generation = self.mount_generation.wrapping_add(1);
        let generation = self.mount_generation;
        #[cfg(target_os = "linux")]
        if self.mount_watch_health.unavailable && self.operation_error.is_none() {
            self.operation_error = Some(MOUNT_WATCH_UNAVAILABLE_MESSAGE.into());
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_mounts::discover() })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.mount_generation != generation {
                    return;
                }
                match result {
                    Ok(mounts) => this.apply_mount_snapshot(mounts, cx),
                    Err(error) => {
                        this.operation_error =
                            Some(format!("Could not refresh mounted volumes: {error}").into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn apply_mount_snapshot(&mut self, mounts: Vec<rmac_mounts::Mount>, cx: &mut Context<Self>) {
        let disappeared = disappeared_mount_roots(&self.mounts, &mounts);
        self.mounts = mounts;
        self.rebuild_location_places();
        if disappeared.is_empty() {
            cx.notify();
            return;
        }

        let current_lost = directory_state::lies_under_any(&self.cwd, &disappeared);
        self.back
            .retain(|path| !directory_state::lies_under_any(path, &disappeared));
        self.fwd
            .retain(|path| !directory_state::lies_under_any(path, &disappeared));
        for tab in &mut self.tabs {
            tab.back
                .retain(|path| !directory_state::lies_under_any(path, &disappeared));
            tab.fwd
                .retain(|path| !directory_state::lies_under_any(path, &disappeared));
            if directory_state::lies_under_any(&tab.cwd, &disappeared) {
                tab.cwd = self.home.clone();
                tab.identity = None;
                tab.back.clear();
                tab.fwd.clear();
            }
        }
        self.clipboard
            .retain(|path| !directory_state::lies_under_any(path, &disappeared));
        if self.clipboard.is_empty() {
            self.clip_cut = false;
        }
        self.thumbs
            .retain(|path, _| !directory_state::lies_under_any(path, &disappeared));
        if self
            .open_with
            .as_ref()
            .is_some_and(|picker| directory_state::lies_under_any(&picker.path, &disappeared))
        {
            self.open_with = None;
        }
        if self.quick_look.as_ref().is_some_and(|panel| {
            panel
                .paths
                .iter()
                .any(|path| directory_state::lies_under_any(path, &disappeared))
        }) {
            self.quick_look_generation = self.quick_look_generation.wrapping_add(1);
            if let Some(panel) = &self.quick_look {
                panel.cancel.store(true, Ordering::Release);
            }
            self.quick_look = None;
        }

        if current_lost {
            self.cwd = self.home.clone();
            self.cwd_identity = None;
            self.back.clear();
            self.fwd.clear();
            self.selected.clear();
            self.anchor = None;
            self.renaming = None;
            self.info = None;
            self.menu_at = None;
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.cwd = self.cwd.clone();
                tab.identity = None;
                tab.back.clear();
                tab.fwd.clear();
            }
            self.operation_notice =
                Some("A mounted volume disconnected; affected tabs returned to Home".into());
            self.reload(cx);
        } else {
            self.operation_notice =
                Some("A mounted volume disconnected; stale locations were removed".into());
            cx.notify();
        }
    }

    fn rebuild_location_places(&mut self) {
        let host = self
            .home
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_volume_name().to_string());
        let mut places = vec![
            Place {
                name: host.into(),
                path: self.home.clone(),
                icon: "icons/house.svg",
                tint: drive_gray(),
                kind: PlaceKind::Item,
            },
            Place {
                name: root_volume_name().into(),
                path: PathBuf::from("/"),
                icon: "icons/hard-drive.svg",
                tint: drive_gray(),
                kind: PlaceKind::Item,
            },
        ];
        places.extend(self.mounts.iter().map(|mount| Place {
            name: mount.name.clone().into(),
            path: mount.path.clone(),
            icon: "icons/hard-drive.svg",
            tint: drive_gray(),
            kind: if mount.ejectable {
                PlaceKind::Volume
            } else {
                PlaceKind::Item
            },
        }));
        if let Some(locations) = self
            .sections
            .iter_mut()
            .find(|section| section.title.as_ref() == "Locations")
        {
            locations.places = places;
        }
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
                        this.refresh_mounts(cx);
                        return;
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
        let q = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };
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
        self.search_relevance_order = false;
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
}

// ---- helpers ----

pub(crate) fn sanitize_dialog_name(name: &str) -> String {
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

pub(crate) fn run() {
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
    use crate::file_ops::copy_item;
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
    fn checked_listing_rejects_a_replacement_at_the_same_path() {
        let root = TestDirectory::new("directory-replacement");
        let current = root.0.join("current");
        std::fs::create_dir(&current).unwrap();
        std::fs::write(current.join("old.txt"), "old").unwrap();
        let expected = directory_state::Identity::capture(&current).unwrap();
        let moved = root.0.join("moved");
        std::fs::rename(&current, &moved).unwrap();
        std::fs::create_dir(&current).unwrap();
        std::fs::write(current.join("new.txt"), "new").unwrap();

        let error = match read_entries_checked(&current, true, Some(expected)) {
            Ok(_) => panic!("the replacement must not be accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    }

    #[test]
    fn mount_disappearance_uses_opaque_identity_not_display_name() {
        let mount = |identity: &str, name: &str, path: &str| rmac_mounts::Mount {
            identity: identity.into(),
            name: name.into(),
            path: PathBuf::from(path),
            ejectable: true,
        };
        let previous = vec![mount("linux:42", "Drive", "/media/drive")];

        assert!(disappeared_mount_roots(
            &previous,
            &[mount("linux:42", "Renamed Drive", "/media/drive")]
        )
        .is_empty());
        assert_eq!(
            disappeared_mount_roots(&previous, &[mount("linux:99", "Drive", "/media/drive")]),
            [PathBuf::from("/media/drive")]
        );
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
    fn ranked_search_summary_discloses_every_incomplete_scope() {
        let report = rmac_search::SearchReport {
            matches: vec![rmac_search::SearchMatch {
                path: PathBuf::from("/scope/result"),
                kind: rmac_search::MatchKind::Content,
                excerpt: Some("match".to_string()),
            }],
            scanned_entries: 100_000,
            content_bytes: 64 * 1024 * 1024,
            results_truncated: true,
            entry_limit_reached: true,
            content_partially_scanned: true,
            skipped_errors: 2,
        };

        let summary = ranked_search_summary(&report, 1);

        assert!(summary.contains("1 result"));
        assert!(summary.contains("100000 items checked"));
        assert!(summary.contains("result limit reached"));
        assert!(summary.contains("item limit reached"));
        assert!(summary.contains("some content not searched"));
        assert!(summary.contains("2 unavailable"));
    }

    #[test]
    fn ranked_search_entry_presents_match_reason_without_private_absolute_path() {
        let root = TestDirectory::new("search-presentation");
        let folder = root.0.join("Documents");
        let path = folder.join("notes.txt");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(&path, b"needle").unwrap();

        let entry = search_entry_for(
            &root.0,
            rmac_search::SearchMatch {
                path,
                kind: rmac_search::MatchKind::Content,
                excerpt: Some("the needle line".to_string()),
            },
        )
        .unwrap();

        let detail = entry.search_detail.unwrap().to_string();
        assert_eq!(detail, "Contents · the needle line");
        assert!(!detail.contains(root.0.to_string_lossy().as_ref()));
    }

    #[test]
    fn quick_look_errors_expose_state_without_private_diagnostics() {
        let error = std::io::Error::other("/home/private/document.pdf: decoder failed");

        let message = quick_look_error_message(&error);

        assert_eq!(
            message,
            "Files could not safely render a preview for this item."
        );
        assert!(!message.contains("/home/private"));
    }
}
