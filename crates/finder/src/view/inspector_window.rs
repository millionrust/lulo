//! File ▸ Show Inspector (⌥⌘I) and File ▸ Get Summary Info (⌃⌘I).
//!
//! Get Info (`search_info_controller.rs`) snapshots one item per window.
//! The Inspector is instead a single floating panel per Finder window that
//! follows that window's selection live: it observes the `FinderView` and
//! redescribes whatever is selected (or the open folder, with nothing
//! selected). A multi-selection shows the same aggregate summary that Get
//! Summary Info opens as its own fixed window: item counts, the total size
//! counted recursively off the UI thread, and the modified-date range.

use super::search_info_controller::{
    format_folder_size, info_artwork, info_block, info_body, info_card, info_details, info_general,
    info_header, info_permissions, info_rows, info_section, info_title_strip, scan_folder_size,
    FolderSize, MAX_FOLDER_INFO_ENTRIES,
};
use super::*;

const NO_SELECTION: &str = "No Selection";
const TRASH_UNAVAILABLE: &str = "Restore an item before viewing its file information";

/// What the Inspector (or a summary window) describes.
#[derive(Clone, Debug, PartialEq)]
enum InspectorTarget {
    /// Nothing can be described; the panel shows this message.
    Message(&'static str),
    /// One item, or the open folder when nothing is selected.
    Item(PathBuf),
    /// Several items, summarised together.
    Summary(Vec<PathBuf>),
}

/// The Finder state the Inspector last described. Notifications that leave
/// it unchanged (hover, scrolling, redraws) cost nothing beyond comparing it.
#[derive(Clone, Debug, PartialEq)]
struct InspectorSnapshot {
    target: InspectorTarget,
    thumbnail: Option<PathBuf>,
}

impl InspectorSnapshot {
    fn title(&self) -> String {
        match &self.target {
            InspectorTarget::Message(_) => "Inspector".to_owned(),
            InspectorTarget::Item(path) => format!("{} Info", display_name(path)),
            InspectorTarget::Summary(_) => "Summary Info".to_owned(),
        }
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Finder's rule for what the Inspector shows: the selection, the open
/// folder with nothing selected, and nothing for the Trash or an empty
/// Applications selection.
fn inspector_target(
    selected: &[PathBuf],
    folder: &Path,
    trash_view: bool,
    applications_view: bool,
) -> InspectorTarget {
    if trash_view {
        return InspectorTarget::Message(TRASH_UNAVAILABLE);
    }
    match selected {
        [] if applications_view => InspectorTarget::Message(NO_SELECTION),
        [] => InspectorTarget::Item(folder.to_path_buf()),
        [path] => InspectorTarget::Item(path.clone()),
        paths => InspectorTarget::Summary(paths.to_vec()),
    }
}

/// The aggregate Get Summary Info describes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SelectionSummary {
    files: usize,
    folders: usize,
    /// Every selected item plus everything inside the selected folders.
    size: FolderSize,
    earliest_modified: Option<SystemTime>,
    latest_modified: Option<SystemTime>,
    /// The folder every item is in, when they share one.
    common_parent: Option<PathBuf>,
}

/// Total the selection without following links or leaving each folder's
/// filesystem, stopping after `max_entries` items. `None` means cancelled.
fn summarize_selection(
    paths: &[PathBuf],
    cancel: &AtomicBool,
    max_entries: usize,
) -> Option<SelectionSummary> {
    let mut summary = SelectionSummary::default();
    let mut parents = paths.iter().map(|path| path.parent());
    if let Some(first) = parents.next() {
        if parents.all(|parent| parent == first) {
            summary.common_parent = first.map(Path::to_path_buf);
        }
    }
    for path in paths {
        if cancel.load(Ordering::Acquire) {
            return None;
        }
        if summary.size.items >= max_entries {
            summary.size.incomplete = true;
            break;
        }
        summary.size.items += 1;
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => {
                summary.size.incomplete = true;
                continue;
            }
        };
        if let Ok(modified) = metadata.modified() {
            summary.earliest_modified = Some(
                summary
                    .earliest_modified
                    .map_or(modified, |earliest| earliest.min(modified)),
            );
            summary.latest_modified = Some(
                summary
                    .latest_modified
                    .map_or(modified, |latest| latest.max(modified)),
            );
        }
        if metadata.is_dir() {
            summary.folders += 1;
            let remaining = max_entries.saturating_sub(summary.size.items);
            let nested = scan_folder_size(path, cancel, remaining)?;
            summary.size.items += nested.items;
            summary.size.bytes = summary.size.bytes.saturating_add(nested.bytes);
            summary.size.incomplete |= nested.incomplete;
        } else {
            summary.files += 1;
            summary.size.bytes = summary.size.bytes.saturating_add(metadata.len());
        }
    }
    Some(summary)
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// "3 files, 1 folder"; a kind with no items is left out.
fn contents_label(files: usize, folders: usize) -> String {
    match (files, folders) {
        (0, 0) => "No items".to_owned(),
        (files, 0) => count_label(files, "file", "files"),
        (0, folders) => count_label(folders, "folder", "folders"),
        (files, folders) => format!(
            "{}, {}",
            count_label(files, "file", "files"),
            count_label(folders, "folder", "folders")
        ),
    }
}

/// One date when every item was modified together, otherwise the range.
fn modified_range_label(summary: &SelectionSummary, date: impl Fn(SystemTime) -> String) -> String {
    match (summary.earliest_modified, summary.latest_modified) {
        (Some(earliest), Some(latest)) if earliest == latest => date(earliest),
        (Some(earliest), Some(latest)) => format!("{} to {}", date(earliest), date(latest)),
        _ => "--".to_owned(),
    }
}

fn summary_details(
    summary: &SelectionSummary,
    date: impl Fn(SystemTime) -> String,
) -> Vec<(&'static str, String)> {
    vec![
        ("Contains", contents_label(summary.files, summary.folders)),
        ("Size", format_folder_size(summary.size)),
        (
            "Where",
            summary
                .common_parent
                .as_ref()
                .map(|parent| parent.display().to_string())
                .unwrap_or_else(|| "Multiple locations".to_owned()),
        ),
        ("Modified", modified_range_label(summary, date)),
    ]
}

fn pending_summary_details() -> Vec<(&'static str, String)> {
    vec![("Size", "Calculating…".to_owned())]
}

enum InspectorContent {
    Message(&'static str),
    Item {
        entry: Box<Entry>,
        details: Vec<(&'static str, String)>,
    },
    Summary {
        count: usize,
        details: Vec<(&'static str, String)>,
    },
}

struct InspectorWindow {
    snapshot: Option<InspectorSnapshot>,
    content: InspectorContent,
    scan_cancel: Option<Arc<AtomicBool>>,
    scan_generation: u64,
    focus: FocusHandle,
    _owner_observer: Option<gpui::Subscription>,
}

impl InspectorWindow {
    fn new(
        owner: Entity<FinderView>,
        live: bool,
        snapshot: InspectorSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let owner_observer = live.then(|| {
            cx.observe_in(&owner, window, |this, owner, window, cx| {
                let snapshot = owner.read(cx).inspector_snapshot();
                this.describe(snapshot, window, cx);
            })
        });
        let mut inspector = Self {
            snapshot: None,
            content: InspectorContent::Message(NO_SELECTION),
            scan_cancel: None,
            scan_generation: 0,
            focus: cx.focus_handle(),
            _owner_observer: owner_observer,
        };
        inspector.describe(snapshot, window, cx);
        inspector
    }

    fn cancel_scan(&mut self) {
        if let Some(cancel) = self.scan_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
    }

    /// Show `snapshot`, unless it is what the panel already shows.
    fn describe(
        &mut self,
        snapshot: InspectorSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        self.cancel_scan();
        self.scan_generation = self.scan_generation.wrapping_add(1);
        let mut title = snapshot.title();
        self.content = match &snapshot.target {
            InspectorTarget::Message(message) => InspectorContent::Message(message),
            InspectorTarget::Item(path) => match entry_for(path) {
                Some(entry) => {
                    title = format!("{} Info", entry.name);
                    if entry.is_dir {
                        self.start_folder_scan(entry.path.clone(), cx);
                    }
                    InspectorContent::Item {
                        details: info_details(&entry),
                        entry: Box::new(entry),
                    }
                }
                None => InspectorContent::Message(NO_SELECTION),
            },
            InspectorTarget::Summary(paths) => {
                self.start_summary_scan(paths.clone(), cx);
                InspectorContent::Summary {
                    count: paths.len(),
                    details: pending_summary_details(),
                }
            }
        };
        window.set_window_title(&title);
        self.snapshot = Some(snapshot);
        cx.notify();
    }

    fn start_folder_scan(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.scan_cancel = Some(cancel.clone());
        let generation = self.scan_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let scanned = cx
                .background_executor()
                .spawn(async move { scan_folder_size(&path, &cancel, MAX_FOLDER_INFO_ENTRIES) })
                .await;
            let _ = this.update(cx, |this: &mut InspectorWindow, cx| {
                if this.scan_generation != generation {
                    return;
                }
                let InspectorContent::Item { details, .. } = &mut this.content else {
                    return;
                };
                if let Some((_, value)) = details.iter_mut().find(|(key, _)| *key == "Size") {
                    *value = scanned
                        .map(format_folder_size)
                        .unwrap_or_else(|| "Unavailable".to_owned());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_summary_scan(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.scan_cancel = Some(cancel.clone());
        let generation = self.scan_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let summary = cx
                .background_executor()
                .spawn(async move { summarize_selection(&paths, &cancel, MAX_FOLDER_INFO_ENTRIES) })
                .await;
            let _ = this.update(cx, |this: &mut InspectorWindow, cx| {
                if this.scan_generation != generation {
                    return;
                }
                let InspectorContent::Summary { details, .. } = &mut this.content else {
                    return;
                };
                *details = match summary {
                    Some(summary) => summary_details(&summary, rmac_finder::listing::date_label),
                    None => vec![("Size", "Unavailable".to_owned())],
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .snapshot
            .as_ref()
            .map(InspectorSnapshot::title)
            .unwrap_or_else(|| "Inspector".to_owned());
        let body = match &self.content {
            InspectorContent::Message(message) => info_body().child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(INFO_SECTION_INSET))
                    .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
                    .text_color(secondary_text())
                    .child(*message),
            ),
            InspectorContent::Item { entry, details } => {
                let thumbnail = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.thumbnail.as_ref());
                info_body()
                    .child(info_header(
                        info_artwork(entry, thumbnail),
                        entry.name.clone(),
                        (!entry.is_dir).then(|| entry.size.clone()),
                        format!("Modified: {}", entry.modified),
                    ))
                    .child(info_general(details))
                    .child(info_permissions(details))
            }
            InspectorContent::Summary { count, details } => info_body()
                .child(info_header(
                    item_artwork(false, "", INFO_HEADER_ICON),
                    format!("{count} items").into(),
                    None,
                    details
                        .iter()
                        .find(|(key, _)| *key == "Contains")
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default(),
                ))
                .child(
                    info_block()
                        .child(info_section("General:"))
                        .children(info_rows(
                            details,
                            &["Contains", "Size", "Where", "Modified"],
                            INFO_LABEL_RIGHT - INFO_SECTION_INSET,
                        )),
                ),
        };
        info_card(&title, &self.focus)
            .child(info_title_strip(&title))
            .child(body)
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    cx.stop_propagation();
                    window.remove_window();
                }
            }))
    }
}

impl Drop for InspectorWindow {
    fn drop(&mut self) {
        self.cancel_scan();
    }
}

impl gpui::Focusable for InspectorWindow {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for InspectorWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_panel(cx)
    }
}

/// Open an Inspector (`live`) or a fixed Get Summary Info window.
fn open_inspector_window(
    owner: Entity<FinderView>,
    live: bool,
    snapshot: InspectorSnapshot,
    cx: &mut gpui::App,
) -> gpui::Result<gpui::WindowHandle<Root>> {
    let (width, height) = rmac_ui::outer_window_size(INFO_WIDTH, INFO_MAX_HEIGHT);
    let mut options = rmac_ui::window_options_for_app_with_title(
        rmac_ui::app_id::FILES,
        snapshot.title(),
        width,
        height,
        cx,
    );
    options.window_bounds = Some(gpui::WindowBounds::centered(
        gpui::size(px(width), px(height)),
        cx,
    ));
    options.window_min_size = Some(gpui::size(px(width), px(height)));
    if live {
        // A utility panel: the Finder window keeps keyboard focus, so arrow
        // keys keep moving the selection the Inspector follows.
        options.focus = false;
        options.kind = gpui::WindowKind::Floating;
    }
    cx.open_window(options, move |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| InspectorWindow::new(owner, live, snapshot, window, cx));
        if !live {
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
        }
        cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
    })
}

impl FinderView {
    fn inspector_snapshot(&self) -> InspectorSnapshot {
        let selected = if self.trash_view {
            Vec::new()
        } else {
            self.selected_paths()
        };
        let target = inspector_target(
            &selected,
            &self.cwd,
            self.trash_view,
            self.applications_view,
        );
        let thumbnail = match &target {
            InspectorTarget::Item(path) => self.thumbs.get(path).cloned(),
            _ => None,
        };
        InspectorSnapshot { target, thumbnail }
    }

    /// File ▸ Show Inspector: open this window's Inspector, or bring the
    /// open one forward.
    pub(super) fn show_inspector(&mut self, cx: &mut Context<Self>) {
        self.menu_at = None;
        if let Some(handle) = self.inspector_window {
            if cx
                .update_window(*handle, |_, window, _| window.activate_window())
                .is_ok()
            {
                cx.notify();
                return;
            }
            self.inspector_window = None;
        }
        let snapshot = self.inspector_snapshot();
        match open_inspector_window(cx.entity(), true, snapshot, cx) {
            Ok(handle) => self.inspector_window = Some(handle),
            Err(_) => {
                self.operation_error = Some("Files could not open the Inspector".into());
            }
        }
        cx.notify();
    }

    /// File ▸ Get Summary Info: one window totalling a multi-selection.
    pub(super) fn get_summary_info(&mut self, cx: &mut Context<Self>) {
        self.menu_at = None;
        if self.trash_view {
            self.operation_error = Some(TRASH_UNAVAILABLE.into());
            cx.notify();
            return;
        }
        let selected = self.selected_paths();
        if selected.len() < 2 {
            return;
        }
        let snapshot = InspectorSnapshot {
            target: InspectorTarget::Summary(selected),
            thumbnail: None,
        };
        match open_inspector_window(cx.entity(), false, snapshot, cx) {
            Ok(handle) => self.info_windows.push(handle),
            Err(_) => {
                self.operation_error = Some("Files could not open the Summary Info window".into());
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("rmac-inspector-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn inspector_follows_the_selection_or_the_open_folder() {
        let folder = PathBuf::from("/home/user/Documents");
        let one = folder.join("a.txt");
        let two = folder.join("b.txt");

        assert_eq!(
            inspector_target(&[], &folder, false, false),
            InspectorTarget::Item(folder.clone())
        );
        assert_eq!(
            inspector_target(std::slice::from_ref(&one), &folder, false, false),
            InspectorTarget::Item(one.clone())
        );
        assert_eq!(
            inspector_target(&[one.clone(), two.clone()], &folder, false, false),
            InspectorTarget::Summary(vec![one.clone(), two])
        );
        assert_eq!(
            inspector_target(&[], &folder, false, true),
            InspectorTarget::Message(NO_SELECTION)
        );
        assert_eq!(
            inspector_target(&[one], &folder, true, false),
            InspectorTarget::Message(TRASH_UNAVAILABLE)
        );
    }

    #[test]
    fn inspector_titles_name_what_they_describe() {
        let item = InspectorSnapshot {
            target: InspectorTarget::Item(PathBuf::from("/tmp/report.pdf")),
            thumbnail: None,
        };
        assert_eq!(item.title(), "report.pdf Info");
        let summary = InspectorSnapshot {
            target: InspectorTarget::Summary(vec![PathBuf::from("/a"), PathBuf::from("/b")]),
            thumbnail: None,
        };
        assert_eq!(summary.title(), "Summary Info");
        let message = InspectorSnapshot {
            target: InspectorTarget::Message(NO_SELECTION),
            thumbnail: None,
        };
        assert_eq!(message.title(), "Inspector");
    }

    #[test]
    fn contents_label_counts_files_and_folders() {
        assert_eq!(contents_label(0, 0), "No items");
        assert_eq!(contents_label(1, 0), "1 file");
        assert_eq!(contents_label(3, 0), "3 files");
        assert_eq!(contents_label(0, 1), "1 folder");
        assert_eq!(contents_label(2, 1), "2 files, 1 folder");
        assert_eq!(contents_label(1, 4), "1 file, 4 folders");
    }

    #[test]
    fn summary_totals_files_and_nested_folders_recursively() {
        let root = temp_root();
        std::fs::write(root.join("one"), b"12345").unwrap();
        std::fs::create_dir_all(root.join("folder/inner")).unwrap();
        std::fs::write(root.join("folder/two"), b"123").unwrap();
        std::fs::write(root.join("folder/inner/three"), b"12").unwrap();
        let cancel = AtomicBool::new(false);
        let paths = vec![root.join("one"), root.join("folder")];

        let summary = summarize_selection(&paths, &cancel, 100).unwrap();

        assert_eq!(summary.files, 1);
        assert_eq!(summary.folders, 1);
        // The two selected items, then folder/two, folder/inner and
        // folder/inner/three inside the selected folder.
        assert_eq!(summary.size.items, 5);
        assert_eq!(summary.size.bytes, 5 + 3 + 2);
        assert!(!summary.size.incomplete);
        assert_eq!(summary.common_parent.as_deref(), Some(root.as_path()));
        assert!(summary.earliest_modified.is_some());
        assert!(summary.earliest_modified <= summary.latest_modified);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn summary_is_bounded_cancellable_and_reports_missing_items() {
        let root = temp_root();
        std::fs::write(root.join("one"), b"1").unwrap();
        std::fs::write(root.join("two"), b"22").unwrap();
        let cancel = AtomicBool::new(false);

        let bounded =
            summarize_selection(&[root.join("one"), root.join("two")], &cancel, 1).unwrap();
        assert_eq!(bounded.size.items, 1);
        assert!(bounded.size.incomplete);

        let missing =
            summarize_selection(&[root.join("one"), root.join("gone")], &cancel, 10).unwrap();
        assert_eq!(missing.files, 1);
        assert!(missing.size.incomplete);

        cancel.store(true, Ordering::Release);
        assert_eq!(summarize_selection(&[root.join("one")], &cancel, 10), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn summary_details_describe_the_range_and_location() {
        let early = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let late = SystemTime::UNIX_EPOCH + Duration::from_secs(20);
        let date = |time: SystemTime| {
            format!(
                "t{}",
                time.duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            )
        };
        let mut summary = SelectionSummary {
            files: 2,
            folders: 1,
            size: FolderSize {
                bytes: 2048,
                items: 4,
                incomplete: false,
            },
            earliest_modified: Some(early),
            latest_modified: Some(late),
            common_parent: None,
        };
        let details = summary_details(&summary, date);
        fn value(details: &[(&'static str, String)], key: &str) -> String {
            details
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
                .unwrap()
        }
        assert_eq!(value(&details, "Contains"), "2 files, 1 folder");
        assert_eq!(value(&details, "Size"), "2 KB (2048 bytes) for 4 items");
        assert_eq!(value(&details, "Where"), "Multiple locations");
        assert_eq!(value(&details, "Modified"), "t10 to t20");

        summary.latest_modified = Some(early);
        summary.common_parent = Some(PathBuf::from("/home/user"));
        let details = summary_details(&summary, date);
        assert_eq!(value(&details, "Modified"), "t10");
        assert_eq!(value(&details, "Where"), "/home/user");
    }
}
