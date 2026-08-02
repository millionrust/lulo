//! Finder window, controller, and rendering ownership.
//!
//! Beyond the pixel-accurate chrome: multi-selection, file operations
//! (new folder, rename, duplicate, copy/cut/paste, move-to-trash), keyboard
//! shortcuts + right-click context menus, live search, clickable sort headers,
//! hidden-file toggle, and live directory watching.

mod filesystem_helpers;
mod lifecycle_controller;
mod mount_controller;
mod navigation;
mod open_with_controller;
mod operations;
mod presentation;
mod rename_controller;
mod selection_controller;
mod startup;
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
