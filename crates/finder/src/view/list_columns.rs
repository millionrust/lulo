//! Finder's list-view columns (FILES-09).
//!
//! macOS 26 Finder keeps Name first and lets every other column be shown or
//! hidden (right-click the header, or View Options), dragged to a new
//! position, resized from its left divider, and sorted by clicking its
//! title (again to reverse). The order, visibility and widths are part of a
//! folder's View Options and persist with them.

use super::view_options::FolderOptions;
use super::*;

pub(super) const COLUMN_COUNT: usize = 9;
const MIN_COLUMN_WIDTH: f32 = 48.0;
const MAX_COLUMN_WIDTH: f32 = 800.0;
/// The Name column flexes to fill what the others leave, but never below
/// this, so a wide resize cannot hide every file name.
pub(super) const MIN_NAME_WIDTH: f32 = 140.0;

/// Every optional list column, in `FolderOptions::columns` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListColumn {
    Modified,
    Created,
    LastOpened,
    Added,
    Size,
    Kind,
    Version,
    Comments,
    Tags,
}

impl ListColumn {
    pub(super) const ALL: [ListColumn; COLUMN_COUNT] = [
        Self::Modified,
        Self::Created,
        Self::LastOpened,
        Self::Added,
        Self::Size,
        Self::Kind,
        Self::Version,
        Self::Comments,
        Self::Tags,
    ];

    pub(super) fn index(self) -> usize {
        self as usize
    }

    pub(super) fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Modified => "Date Modified",
            Self::Created => "Date Created",
            Self::LastOpened => "Date Last Opened",
            Self::Added => "Date Added",
            Self::Size => "Size",
            Self::Kind => "Kind",
            Self::Version => "Version",
            Self::Comments => "Comments",
            Self::Tags => "Tags",
        }
    }

    pub(super) fn default_width(self) -> f32 {
        match self {
            Self::Modified | Self::Created | Self::LastOpened | Self::Added | Self::Comments => {
                DATE_W
            }
            Self::Size | Self::Version => SIZE_W,
            Self::Kind | Self::Tags => KIND_W,
        }
    }

    pub(super) fn sort_key(self) -> SortKey {
        match self {
            Self::Modified => SortKey::Date,
            Self::Created => SortKey::Created,
            Self::LastOpened => SortKey::LastOpened,
            Self::Added => SortKey::Added,
            Self::Size => SortKey::Size,
            Self::Kind => SortKey::Kind,
            Self::Version => SortKey::Version,
            Self::Comments => SortKey::Comments,
            Self::Tags => SortKey::Tags,
        }
    }

    /// Finder right-aligns only Size, so the units line up.
    pub(super) fn right_aligned(self) -> bool {
        self == Self::Size
    }
}

pub(super) fn default_column_order() -> Vec<u8> {
    (0..COLUMN_COUNT as u8).collect()
}

pub(super) fn default_column_widths() -> [f32; COLUMN_COUNT] {
    ListColumn::ALL.map(ListColumn::default_width)
}

pub(super) fn width_is_valid(width: f32) -> bool {
    width.is_finite() && (MIN_COLUMN_WIDTH..=MAX_COLUMN_WIDTH).contains(&width)
}

pub(super) fn clamp_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH)
    } else {
        MIN_COLUMN_WIDTH
    }
}

/// The stored order, repaired: unknown and repeated indices are dropped and
/// any column missing from it is appended in its default place, so a
/// damaged or older state file still shows every column exactly once.
pub(super) fn column_order(options: &FolderOptions) -> Vec<ListColumn> {
    let mut seen = [false; COLUMN_COUNT];
    let mut order = Vec::with_capacity(COLUMN_COUNT);
    for &index in &options.column_order {
        if let Some(column) = ListColumn::from_index(usize::from(index)) {
            if !seen[column.index()] {
                seen[column.index()] = true;
                order.push(column);
            }
        }
    }
    for column in ListColumn::ALL {
        if !seen[column.index()] {
            order.push(column);
        }
    }
    order
}

/// The shown columns, left to right after Name.
pub(super) fn visible_columns(options: &FolderOptions) -> Vec<ListColumn> {
    column_order(options)
        .into_iter()
        .filter(|column| options.columns[column.index()])
        .collect()
}

pub(super) fn column_width(options: &FolderOptions, column: ListColumn) -> f32 {
    let width = options.column_widths[column.index()];
    if width_is_valid(width) {
        width
    } else {
        column.default_width()
    }
}

/// Drop `moved` onto `target`'s header: moving right lands after the
/// target, moving left lands before it, as Finder's live column swap does.
pub(super) fn reorder_columns(options: &mut FolderOptions, moved: ListColumn, target: ListColumn) {
    if moved == target {
        return;
    }
    let mut order = column_order(options);
    let Some(from) = order.iter().position(|column| *column == moved) else {
        return;
    };
    let Some(to) = order.iter().position(|column| *column == target) else {
        return;
    };
    let column = order.remove(from);
    order.insert(to, column);
    options.column_order = order.into_iter().map(|column| column.index() as u8).collect();
}

/// Right-click the list header, or View ▸ Show View Options ▸ Show Columns.
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = finder, no_json)]
pub(super) struct ToggleListColumnAction {
    pub(super) index: usize,
}

/// Drag payload: a column header being dragged to a new position.
pub(super) struct DraggedListColumn {
    pub(super) column: ListColumn,
}

impl Render for DraggedListColumn {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(LIST_HEADER_HEIGHT))
            .px(px(LIST_CELL_TEXT_X))
            .flex()
            .items_center()
            .rounded(px(rmac_ui::mac::radius_control()))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(header_divider())
            .opacity(0.9)
            .text_size(rmac_ui::text_px(LIST_HEADER_TEXT))
            .text_color(chrome_text())
            .child(self.column.label())
    }
}

/// Drag payload: the divider at a column's left edge, dragged to resize it. Nothing accepts it as a drop; its moves set the width live.
pub(super) struct ResizingListColumn;

/// A divider drag shows nothing under the pointer; the column itself moves.
pub(super) struct EmptyDragView;

impl Render for EmptyDragView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// A divider drag in progress: where it started and the width it started at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ColumnResize {
    pub(super) column: ListColumn,
    pub(super) start_x: f32,
    pub(super) start_width: f32,
}

impl FinderView {
    pub(super) fn toggle_list_column(&mut self, index: usize, cx: &mut Context<Self>) {
        if ListColumn::from_index(index).is_none() {
            return;
        }
        self.menu_at = None;
        self.change_options(|options| options.columns[index] = !options.columns[index], cx);
        if index == ListColumn::Comments.index() {
            cx.notify();
        }
    }

    pub(super) fn open_list_header_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu_purpose = MenuPurpose::ListColumns;
        self.menu_at = Some(rmac_ui::ContextMenuState::open(
            position,
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    /// The header's right-click menu: Name (always shown, so greyed) and a
    /// check for every optional column, in Finder's fixed menu order.
    pub(super) fn build_list_columns_menu(
        pos: Point<Pixels>,
        options: &FolderOptions,
    ) -> rmac_ui::ContextMenu {
        let mut menu = rmac_ui::ContextMenu::new(pos).entry(
            "Name",
            None,
            Box::new(ToggleListColumnAction {
                index: COLUMN_COUNT,
            }),
            false,
            rmac_ui::MenuCheck::On,
        );
        for column in ListColumn::ALL {
            menu = menu.checked_item(
                column.label(),
                if options.columns[column.index()] {
                    rmac_ui::MenuCheck::On
                } else {
                    rmac_ui::MenuCheck::None
                },
                Box::new(ToggleListColumnAction {
                    index: column.index(),
                }),
            );
        }
        menu
    }

    pub(super) fn reorder_list_column(
        &mut self,
        moved: ListColumn,
        target: ListColumn,
        cx: &mut Context<Self>,
    ) {
        self.change_options(|options| reorder_columns(options, moved, target), cx);
    }

    pub(super) fn start_column_resize(&mut self, column: ListColumn, start_x: f32) {
        let width = column_width(&self.current_options(), column);
        self.column_resize = Some(ColumnResize {
            column,
            start_x,
            start_width: width,
        });
    }

    /// Follow a divider drag. The width changes in memory only; the folder's
    /// options are saved once, when the drag ends.
    pub(super) fn drag_column_resize(&mut self, x: f32, cx: &mut Context<Self>) {
        let Some(resize) = self.column_resize else {
            return;
        };
        // Name flexes to fill the window, so a column's right edge is fixed
        // by the columns after it: its left divider follows the pointer when
        // the column grows by exactly as much as the pointer moves left.
        let width = clamp_width(resize.start_width - (x - resize.start_x));
        let mut options = self.current_options();
        if options.column_widths[resize.column.index()] == width {
            return;
        }
        options.column_widths[resize.column.index()] = width;
        self.folder_options.insert(self.cwd.clone(), options);
        cx.notify();
    }

    pub(super) fn finish_column_resize(&mut self) {
        if self.column_resize.take().is_some() {
            self.persist_finder_state();
        }
    }

    /// One column title: sortable, draggable to reorder, a drop target for
    /// other titles, with its left divider as the resize handle.
    pub(super) fn render_list_header_cell(
        &self,
        column: ListColumn,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let key = column.sort_key();
        let active = !self.search_relevance_order && self.sort_key == key;
        div()
            .id(SharedString::from(format!("head-{}", column.index())))
            .role(Role::Button)
            .aria_label(column.label())
            .h_full()
            .w(px(width))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .child(
                div()
                    .id(SharedString::from(format!("head-resize-{}", column.index())))
                    .w(px(7.0))
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .child(
                        div()
                            .w(px(1.0))
                            .h(px(LIST_HEADER_DIVIDER_HEIGHT))
                            .bg(header_divider()),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.start_column_resize(column, f32::from(event.position.x));
                        }),
                    )
                    .on_drag(ResizingListColumn, |_, _, _, cx| {
                        cx.new(|_| EmptyDragView)
                    }),
            )
            .child(
                div()
                    .pl(px(LIST_CELL_TEXT_X - 7.0).max(px(0.0)))
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .when(column.right_aligned(), |cell| {
                        cell.flex().justify_end().pr(px(LIST_SIZE_TRAILING))
                    })
                    .text_size(rmac_ui::text_px(LIST_HEADER_TEXT))
                    .font_weight(if active {
                        rmac_ui::mac::SEMIBOLD
                    } else {
                        rmac_ui::mac::REGULAR
                    })
                    .text_color(if active {
                        sorted_header_text()
                    } else {
                        chrome_text()
                    })
                    .child(column.label()),
            )
            .when(active, |cell| {
                cell.child(div().mr(px(8.0)).flex_none().child(icon(
                    if self.sort_asc {
                        "icons/chevron-up.svg"
                    } else {
                        "icons/chevron-down.svg"
                    },
                    11.0,
                    chrome_text(),
                )))
            })
            .on_drag(DraggedListColumn { column }, move |_, _, _, cx| {
                cx.new(|_| DraggedListColumn { column })
            })
            .drag_over::<DraggedListColumn>(|style, _, _, _| style.bg(stripe()))
            .on_drop(cx.listener(move |this, dragged: &DraggedListColumn, _, cx| {
                this.reorder_list_column(dragged.column, column, cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
    }

    /// The text of one optional column for `entry`.
    pub(super) fn list_cell_text(
        &self,
        entry: &Entry,
        column: ListColumn,
        options: &FolderOptions,
    ) -> SharedString {
        let pick = |relative: &SharedString, absolute: &SharedString| {
            if options.relative_dates {
                relative.clone()
            } else {
                absolute.clone()
            }
        };
        match column {
            ListColumn::Modified => pick(&entry.modified, &entry.modified_absolute),
            ListColumn::Created => pick(&entry.created, &entry.created_absolute),
            ListColumn::LastOpened => pick(&entry.last_opened, &entry.last_opened_absolute),
            ListColumn::Added => pick(&entry.added, &entry.added_absolute),
            ListColumn::Size => {
                if options.calculate_sizes && entry.is_dir {
                    self.directory_sizes
                        .get(&entry.path)
                        .map(|size| human_size(*size).into())
                        .unwrap_or_else(|| "--".into())
                } else {
                    entry.size.clone()
                }
            }
            ListColumn::Kind => entry.kind.clone(),
            ListColumn::Version => "--".into(),
            ListColumn::Comments => entry.comment.clone().unwrap_or_default(),
            ListColumn::Tags => entry
                .tag
                .as_ref()
                .map(|tag| super::file_metadata::tag_display_name(tag).into())
                .unwrap_or_default(),
        }
    }

    pub(super) fn render_list_cell(
        &self,
        entry: &Entry,
        column: ListColumn,
        options: &FolderOptions,
        color: Hsla,
    ) -> Div {
        let text = self.list_cell_text(entry, column, options);
        let width = column_width(options, column);
        let cell = div()
            .w(px(width))
            .flex_none()
            .truncate()
            .text_color(color);
        let cell = if column.right_aligned() {
            cell.flex().justify_end().pr(px(LIST_SIZE_TRAILING))
        } else {
            cell.pl(px(LIST_CELL_TEXT_X))
        };
        if column == ListColumn::Tags {
            if let Some(tag) = &entry.tag {
                return cell.flex().items_center().gap(px(4.0)).child(
                    div()
                        .size(px(8.0))
                        .flex_none()
                        .rounded_full()
                        .bg(super::file_metadata::tag_colour(tag)),
                )
                .child(div().truncate().child(text));
            }
        }
        cell.child(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_order_still_lists_every_column_once() {
        let options = FolderOptions {
            column_order: vec![5, 5, 200, 0],
            ..FolderOptions::default()
        };
        let order = column_order(&options);
        assert_eq!(order.len(), COLUMN_COUNT);
        assert_eq!(order[0], ListColumn::Kind);
        assert_eq!(order[1], ListColumn::Modified);
        assert_eq!(order[2], ListColumn::Created);
    }

    #[test]
    fn default_columns_match_finder_list_view() {
        let options = FolderOptions::default();
        assert_eq!(
            visible_columns(&options),
            [ListColumn::Modified, ListColumn::Size, ListColumn::Kind]
        );
    }

    #[test]
    fn dragging_right_lands_after_and_left_lands_before() {
        let mut options = FolderOptions::default();
        reorder_columns(&mut options, ListColumn::Modified, ListColumn::Kind);
        assert_eq!(
            visible_columns(&options),
            [ListColumn::Size, ListColumn::Kind, ListColumn::Modified]
        );
        reorder_columns(&mut options, ListColumn::Kind, ListColumn::Size);
        assert_eq!(
            visible_columns(&options),
            [ListColumn::Kind, ListColumn::Size, ListColumn::Modified]
        );
        reorder_columns(&mut options, ListColumn::Kind, ListColumn::Kind);
        assert_eq!(visible_columns(&options)[0], ListColumn::Kind);
    }

    #[test]
    fn hidden_columns_keep_their_place_when_shown_again() {
        let mut options = FolderOptions::default();
        options.columns[ListColumn::Tags.index()] = true;
        reorder_columns(&mut options, ListColumn::Tags, ListColumn::Modified);
        assert_eq!(visible_columns(&options)[0], ListColumn::Tags);
        options.columns[ListColumn::Tags.index()] = false;
        assert_eq!(visible_columns(&options)[0], ListColumn::Modified);
        options.columns[ListColumn::Tags.index()] = true;
        assert_eq!(visible_columns(&options)[0], ListColumn::Tags);
    }

    #[test]
    fn widths_are_clamped_and_bad_ones_fall_back() {
        assert_eq!(clamp_width(1.0), MIN_COLUMN_WIDTH);
        assert_eq!(clamp_width(10_000.0), MAX_COLUMN_WIDTH);
        assert_eq!(clamp_width(f32::NAN), MIN_COLUMN_WIDTH);
        let mut options = FolderOptions::default();
        options.column_widths[ListColumn::Size.index()] = f32::INFINITY;
        assert_eq!(column_width(&options, ListColumn::Size), SIZE_W);
        options.column_widths[ListColumn::Size.index()] = 150.0;
        assert_eq!(column_width(&options, ListColumn::Size), 150.0);
    }

    #[test]
    fn every_column_sorts_by_its_own_key() {
        let keys = ListColumn::ALL.map(ListColumn::sort_key);
        for (index, key) in keys.iter().enumerate() {
            assert!(!keys[..index].contains(key), "{key:?} repeats");
        }
        assert!(!keys.contains(&SortKey::Name));
    }

    #[test]
    fn options_without_column_layout_load_with_defaults() {
        let options: FolderOptions = serde_json::from_str(r#"{"columns":[true,true,false,false,true,true,false,false,false]}"#).unwrap();
        assert_eq!(options.column_order, default_column_order());
        assert_eq!(options.column_widths, default_column_widths());
        assert_eq!(
            visible_columns(&options),
            [
                ListColumn::Modified,
                ListColumn::Created,
                ListColumn::Size,
                ListColumn::Kind
            ]
        );
    }
}
