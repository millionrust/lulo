//! Finder window, controller, and rendering ownership.
//!
//! Beyond the pixel-accurate chrome: multi-selection, file operations
//! (new folder, rename, duplicate, copy/cut/paste, move-to-trash), keyboard
//! shortcuts + right-click context menus, live search, clickable sort headers,
//! hidden-file toggle, and live directory watching.

mod accessibility;
mod archive_controller;
mod chrome_presentation;
mod conflict_controller;
mod content_presentation;
mod dialog_presentation;
mod filesystem_helpers;
mod finder_behaviour;
mod finder_style;
mod gallery_presentation;
mod go_to_folder_controller;
mod item_operations;
mod lifecycle_controller;
mod list_presentation;
mod mount_controller;
mod navigation;
mod open_with_controller;
mod operations;
mod permanent_delete_controller;
mod presentation;
mod presentation_persistence;
mod presentation_support;
mod quick_look_controller;
mod recovery_controller;
mod rename_controller;
mod responsive_layout;
mod search_helpers;
mod search_info_controller;
mod selection_controller;
mod startup;
mod thumbnail_controller;
mod transient_state;
mod trash_controller;
mod trash_recovery_controller;
mod trash_task_controller;
mod trash_updates;
mod undo_controller;
mod updates;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use gpui::{
    actions, div, img, prelude::FluentBuilder as _, px, svg, AccessibleAction, AppContext as _,
    AssetSource, ClickEvent, Context, Div, Entity, ExternalPaths, FocusHandle, Focusable as _,
    Hsla, InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Point, Render, Result, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, Svg, Toggled, Window,
};
use gpui_component::{Icon, IconName, StyledExt as _};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rmac_ui::{
    Button, InputEvent, InputState, SearchField, Slider, SliderEvent, SliderState, Spinner,
    TextField, Toggle,
};

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
use crate::{directory_state, file_ops, operation_journal, pasteboard, undo_journal};
use filesystem_helpers::*;
use finder_behaviour::*;
use finder_style::*;
use presentation_persistence::{FinderPersistence, MAX_RESTORED_TABS};
use presentation_support::*;
use search_helpers::*;
use selection_controller::accessible_item;
use transient_state::*;

pub(crate) use presentation_support::sanitize_dialog_name;

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
        GoBack,
        GoForward,
        GoUp,
        GoHome,
        GoApplications,
        GoDownloads,
        GoTrash,
        ToggleHidden,
        OpenItems,
        OpenWith,
        QuickLook,
        Compress,
        GetInfo,
        ViewAsIcons,
        ViewAsList,
        ViewAsColumns,
        ViewAsGallery,
        SortByName,
        SortByDate,
        SortBySize,
        SortByKind,
        NewTab,
        CloseTab,
        PreviousTab,
        NextTab,
        ShowHelp,
        ToggleSidebar,
        TogglePathBar,
        GoComputer,
        NewWindow,
        GoToFolder,
        EmptyTrash,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
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
    /// Installed applications are a Finder destination, not ordinary launcher
    /// files. Keep the trusted catalog launch contract with the projected row
    /// so opening one never routes its `.desktop` file through a document app.
    application: Option<ApplicationEntry>,
}

#[derive(Clone)]
struct ApplicationEntry {
    launch: rmac_apps::LaunchSpec,
    icon: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq)]
enum PlaceKind {
    Item,
    Volume,
    Applications,
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

#[derive(Clone, Copy, PartialEq)]
enum MenuPurpose {
    Context,
    Sort,
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
    /// The system clipboard offered files when last asked (window
    /// activation, Copy, Paste), so Paste is offered here too.
    pasteboard_has_files: bool,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<rmac_ui::ContextMenuState>,
    menu_purpose: MenuPurpose,
    help_open: bool,
    renaming: Option<(PathBuf, gpui::Entity<InputState>)>,
    show_hidden: bool,
    view: ViewMode,
    sidebar_visible: bool,
    sidebar_width: f32,
    resizing_sidebar: bool,
    finder_persistence: FinderPersistence,
    col_stack: Vec<PathBuf>,
    /// Column view can select an item several directories below `cwd`, so an
    /// index into `entries` is not sufficient. Keep the selected entry itself
    /// as the authority for commands and context menus in that view.
    column_selection: Option<Entry>,
    sort_key: SortKey,
    sort_asc: bool,
    query: gpui::Entity<InputState>,
    icon_size: f32,
    icon_size_slider: Entity<SliderState>,
    back: Vec<PathBuf>,
    fwd: Vec<PathBuf>,
    file_words: rmac_locale::FileVocabulary,
    sections: Vec<Section>,
    info: Option<Entry>,
    /// Go ▸ Go to Folder…, while its sheet is open.
    go_to: Option<go_to_folder_controller::GoToSheet>,
    /// An item Go to Folder named, selected once its folder loads.
    pending_select: Option<PathBuf>,
    /// Get Info's rows, read from the file system once when it opens (and
    /// after a rename from it), never while rendering.
    info_details: Vec<(&'static str, String)>,
    /// Get Info's editable Name & Extension field and the path it renames.
    info_name: Option<(PathBuf, gpui::Entity<InputState>)>,
    open_with: Option<OpenWithPicker>,
    open_generation: u64,
    quick_look: Option<QuickLookPanel>,
    archive_job: Option<archive_controller::ArchiveJob>,
    archive_generation: u64,
    archive_alert: Option<SharedString>,
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
    applications_view: bool,
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
    native_window_title: String,
    watcher: Option<RecommendedWatcher>,
    filesystem_events: async_channel::Sender<()>,
    filesystem_hints: Arc<Mutex<FilesystemHints>>,
    watched: Option<PathBuf>,
    watched_parent: Option<PathBuf>,
    search_generation: u64,
    search_cancel: Option<Arc<AtomicBool>>,
    /// The toolbar search circle has been opened into a field.
    search_open: bool,
    /// View ▸ Show Path Bar (⌥⌘P); off by default, as on the Mac.
    show_path_bar: bool,
    icon_scroll: gpui::ScrollHandle,
    marquee: Option<Marquee>,
    type_select: TypeSelect,
    spring: SpringLoading,
}

/// Opens one window per argument list (see `StartupDestination`).
pub(crate) fn run(windows: Vec<Vec<String>>) {
    rmac_ui::boot_unified_app_instance_with_assets(
        rmac_ui::app_id::FILES,
        CombinedAssets,
        finder_style::WINDOW_WIDTH,
        finder_style::WINDOW_HEIGHT,
        windows,
        |arguments, window, cx| {
            // Another launch's arguments were checked in that process; a
            // folder that has since gone away opens the default window.
            let destination =
                crate::StartupDestination::parse(arguments.iter().cloned()).unwrap_or_default();
            let mut finder = FinderView::new(window, cx);
            match destination {
                crate::StartupDestination::Default => {}
                crate::StartupDestination::Trash => finder.trash_click(cx),
                crate::StartupDestination::Directory(path) => finder.navigate(path, cx),
                crate::StartupDestination::Reveal(path) => {
                    if let Some(folder) = path.parent().map(Path::to_path_buf) {
                        finder.pending_select = Some(path);
                        finder.navigate(folder, cx);
                    }
                }
                crate::StartupDestination::Search(query) => {
                    // The home folder, searched as Return in the search
                    // field does.
                    finder
                        .query
                        .update(cx, |state, cx| state.set_value(query, window, cx));
                    finder.recursive_search(cx);
                }
            }
            finder
        },
    );
}

#[cfg(test)]
mod tests;
