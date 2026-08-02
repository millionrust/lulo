//! Finder window, controller, and rendering ownership.
//!
//! Beyond the pixel-accurate chrome: multi-selection, file operations
//! (new folder, rename, duplicate, copy/cut/paste, move-to-trash), keyboard
//! shortcuts + right-click context menus, live search, clickable sort headers,
//! hidden-file toggle, and live directory watching.

mod content_presentation;
mod dialog_presentation;
mod filesystem_helpers;
mod lifecycle_controller;
mod mount_controller;
mod navigation;
mod open_with_controller;
mod operations;
mod presentation;
mod presentation_support;
mod quick_look_controller;
mod rename_controller;
mod selection_controller;
mod startup;
mod transient_state;
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
use presentation_support::*;
use transient_state::*;

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
mod tests;
