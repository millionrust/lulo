use super::*;

impl FinderView {
    /// Column-view preview (design-lab/finder.html): the preview fills the
    /// top of the column, then the name, "kind – size" and an Information
    /// table, all left-aligned 10 in from the column edge.
    fn render_column_preview(&self, entry: &Entry) -> gpui::AnyElement {
        let visual = entry
            .application
            .as_ref()
            .and_then(|application| application.icon.clone())
            .map(|path| {
                img(path)
                    .max_w(px(COLUMN_PREVIEW_ARTWORK))
                    .max_h(px(COLUMN_PREVIEW_ARTWORK))
                    .rounded(px(rmac_ui::mac::radius_popover()))
                    .into_any_element()
            })
            .or_else(|| {
                self.thumbs.get(&entry.path).map(|thumbnail| {
                    img(thumbnail.clone())
                        .max_w(gpui::relative(1.0))
                        .max_h(gpui::relative(1.0))
                        .rounded(px(rmac_ui::mac::radius_control()))
                        .into_any_element()
                })
            })
            .unwrap_or_else(|| item_artwork(false, &entry.name, COLUMN_PREVIEW_ARTWORK));
        let summary = SharedString::from(format!("{} – {}", entry.kind, entry.size));
        let information = [("Modified", entry.modified.clone())];

        div()
            .id("column-preview")
            .min_w(px(COLUMN_WIDTH))
            .flex_1()
            .h_full()
            .v_flex()
            .px(px(COLUMN_PREVIEW_INSET))
            .pt(px(COLUMN_PREVIEW_INSET))
            .pb(px(COLUMN_PREVIEW_INSET))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(visual),
            )
            .child(
                div()
                    .pt(px(COLUMN_PREVIEW_TEXT_GAP))
                    .truncate()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(label())
                    .child(entry.name.clone()),
            )
            .child(
                div()
                    .truncate()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(secondary_text())
                    .child(summary),
            )
            .child(
                div()
                    .pt(px(12.0))
                    .pb(px(4.0))
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child("Information"),
            )
            .children(
                information
                    .into_iter()
                    .enumerate()
                    .map(|(row, (name, value))| inspector_row(row > 0, name, value)),
            )
            .into_any_element()
    }

    pub(super) fn render_columns(
        &self,
        window_active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if self.applications_view {
            return self
                .render_application_columns(window_active, cx)
                .into_any_element();
        }
        // Column view is a browser of nested folders: assistive technology
        // sees one tree whose items carry their column as their level.
        let entity = cx.entity();
        let mut row = div()
            .id("columns")
            .role(Role::Tree)
            .aria_label(self.title())
            .flex_1()
            .flex()
            .overflow_x_scroll()
            .bg(list_bg());
        for (ci, dir) in self.col_stack.iter().enumerate() {
            let mut entries = read_entries(dir, self.show_hidden);
            sort_entries(&mut entries, self.sort_key, self.sort_asc);
            let selected_child = self.col_stack.get(ci + 1).cloned();
            let mut col = div()
                .id(SharedString::from(format!("col-{ci}")))
                .w(px(COLUMN_WIDTH))
                .h_full()
                .flex_none()
                .border_r_1()
                .border_color(dark_rule())
                .overflow_y_scroll()
                .v_flex()
                .pt(px(COLUMN_ROWS_TOP));
            let column_count = entries.len();
            for (position, e) in entries.into_iter().enumerate() {
                // The focused selection is blue; the folders leading to it in
                // earlier columns keep the grey unfocused selection.
                let is_focus_sel = self
                    .column_selection
                    .as_ref()
                    .is_some_and(|selected| selected.path == e.path);
                let is_sel = is_focus_sel || selected_child.as_ref() == Some(&e.path);
                let row_active = window_active && is_focus_sel;
                let row_text = if is_sel {
                    selected_text(row_active)
                } else {
                    primary_text()
                };
                let open_path = e.path.clone();
                let is_dir = e.is_dir;
                let selected_entry = e.clone();
                let context_entry = e.clone();
                let rename_entry = e.clone();
                let accessible_entry = e.clone();
                let name_cell: gpui::AnyElement = match &self.renaming {
                    Some((rename_path, input)) if rename_path == &e.path => div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(TextField::new(input).appearance(true))
                        .into_any_element(),
                    _ => div()
                        .id(SharedString::from(format!("column-name-{ci}-{}", e.name)))
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(rmac_ui::text_px(13.0))
                        .truncate()
                        .text_color(row_text)
                        .child(e.name.clone())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                if is_sel && !event.modifiers.platform && !event.modifiers.shift {
                                    cx.stop_propagation();
                                    this.column_selection = Some(rename_entry.clone());
                                    this.rename_start(window, cx);
                                }
                            }),
                        )
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            if is_sel {
                                cx.stop_propagation();
                            }
                        }))
                        .into_any_element(),
                };
                col = col.child(
                    accessible_item(
                        div().id(SharedString::from(format!("colrow-{ci}-{}", e.name))),
                        Role::TreeItem,
                        e.name.clone(),
                        is_sel,
                        position,
                        column_count,
                        &entity,
                        move |this, window, cx| {
                            this.accessible_select_column(ci, accessible_entry.clone(), window, cx)
                        },
                    )
                    .aria_level(ci + 1)
                    .when(is_dir, |item| {
                        item.aria_expanded(selected_child.as_ref() == Some(&e.path))
                    })
                    .flex()
                    .items_center()
                    .flex_none()
                    .h(px(COLUMN_ROW_HEIGHT))
                    .mx(px(COLUMN_ROW_INSET))
                    .pl(px(COLUMN_ICON_X))
                    .rounded(px(ROW_RADIUS))
                    .text_size(rmac_ui::text_px(13.0))
                    .when(is_sel, |el: Stateful<Div>| el.bg(selection(row_active)))
                    .child(match self.thumbs.get(&e.path) {
                        Some(thumbnail) => div()
                            .size(px(LIST_ICON))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                img(thumbnail.clone())
                                    .max_w(px(LIST_ICON))
                                    .max_h(px(LIST_ICON - 4.0))
                                    .rounded(px(2.0)),
                            )
                            .into_any_element(),
                        None => item_artwork(is_dir, &e.name, LIST_ICON),
                    })
                    .child(
                        div()
                            .w(px(COLUMN_TEXT_X - COLUMN_ICON_X - LIST_ICON))
                            .flex_none(),
                    )
                    .child(name_cell)
                    .when(is_dir, |el: Stateful<Div>| {
                        el.child(icon(
                            "icons/chevron-right.svg",
                            12.0,
                            if is_sel { row_text } else { chrome_text() },
                        ))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if event.modifiers.control {
                                this.open_column_context_menu(
                                    selected_entry.clone(),
                                    event.position,
                                    window,
                                    cx,
                                );
                                return;
                            }
                            this.select_column_entry(ci, selected_entry.clone());
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_column_context_menu(
                                context_entry.clone(),
                                event.position,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_click(cx.listener(
                        move |this, event: &ClickEvent, _, cx| {
                            if event.click_count() < 2 || is_dir {
                                return;
                            }
                            this.open_paths(vec![open_path.clone()], cx);
                        },
                    )),
                );
            }
            row = row.child(col);
        }
        if let Some(entry) = self.column_selection.as_ref().filter(|entry| !entry.is_dir) {
            row = row.child(self.render_column_preview(entry));
        }
        row.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.column_selection = None;
                this.selected.clear();
                this.anchor = None;
                window.focus(&this.focus, cx);
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.column_selection = None;
                this.open_context_menu(None, event.position, window, cx);
            }),
        )
        .into_any_element()
    }

    fn render_application_columns(
        &self,
        window_active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut applications = div()
            .id("applications-column")
            .role(Role::ListBox)
            .aria_label("Applications")
            .w(px(COLUMN_WIDTH))
            .h_full()
            .flex_none()
            .overflow_y_scroll()
            .v_flex()
            .pt(px(COLUMN_ROWS_TOP))
            .border_r_1()
            .border_color(dark_rule());

        let entity = cx.entity();
        let count = self.entries.len();
        for (index, entry) in self.entries.iter().enumerate() {
            let selected = self.selected.contains(&index);
            let icon_element = entry
                .application
                .as_ref()
                .and_then(|application| application.icon.clone())
                .map_or_else(
                    || icon("icons/layout-grid.svg", LIST_ICON, secondary()).into_any_element(),
                    |path| {
                        img(path)
                            .w(px(LIST_ICON))
                            .h(px(LIST_ICON))
                            .rounded(px(rmac_ui::mac::radius_menu_item()))
                            .into_any_element()
                    },
                );
            applications = applications.child(
                accessible_item(
                    div().id(("application-column-row", index)),
                    Role::ListBoxOption,
                    entry.name.clone(),
                    selected,
                    index,
                    count,
                    &entity,
                    move |this, window, cx| this.accessible_select(index, window, cx),
                )
                .h(px(COLUMN_ROW_HEIGHT))
                .mx(px(COLUMN_ROW_INSET))
                .pl(px(COLUMN_ICON_X))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(COLUMN_TEXT_X - COLUMN_ICON_X - LIST_ICON))
                .rounded(px(ROW_RADIUS))
                .when(selected, |element: Stateful<Div>| {
                    element.bg(selection(window_active))
                })
                .child(icon_element)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(if selected {
                            selected_text(window_active)
                        } else {
                            primary_text()
                        })
                        .child(entry.name.clone()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        if event.modifiers.control {
                            this.open_context_menu(Some(index), event.position, window, cx);
                            return;
                        }
                        this.handle_click(index, event.modifiers.platform, event.modifiers.shift);
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_context_menu(Some(index), event.position, window, cx);
                    }),
                )
                .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                    if event.click_count() >= 2 {
                        this.open_index(index, cx);
                    }
                })),
            );
        }

        let preview = self
            .selected
            .iter()
            .next()
            .and_then(|index| self.entries.get(*index))
            .map(|entry| {
                let artwork = entry
                    .application
                    .as_ref()
                    .and_then(|application| application.icon.clone())
                    .map_or_else(
                        || icon("icons/layout-grid.svg", 96.0, secondary()).into_any_element(),
                        |path| {
                            img(path)
                                .w(px(96.0))
                                .h(px(96.0))
                                .rounded(px(rmac_ui::mac::radius_popover()))
                                .into_any_element()
                        },
                    );
                div()
                    .flex_1()
                    .h_full()
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(artwork)
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(15.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(entry.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child("Application"),
                    )
            })
            .unwrap_or_else(|| {
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        rmac_ui::EmptyState::new("Select an Application")
                            .message("Choose an application to see its preview."),
                    )
            });

        div()
            .id("application-columns")
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .bg(list_bg())
            .child(applications)
            .child(preview)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.selected.clear();
                    this.anchor = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_context_menu(None, event.position, window, cx);
                }),
            )
    }

    /// Select one Column-view item for assistive technology, as a plain click
    /// on its row does: the item becomes the selection, later columns close,
    /// and a folder opens its own column.
    fn accessible_select_column(
        &mut self,
        column: usize,
        entry: Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if column >= self.col_stack.len() {
            return;
        }
        self.select_column_entry(column, entry);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Makes `entry`, shown in column `ci`, the Column-view selection: any
    /// columns to its right close, and a folder opens its own preview
    /// column immediately, as clicking a row does.
    pub(super) fn select_column_entry(&mut self, ci: usize, entry: Entry) {
        self.selected.clear();
        self.anchor = None;
        self.column_selection = Some(entry.clone());
        self.col_stack.truncate(ci + 1);
        if entry.is_dir {
            self.col_stack.push(entry.path);
        }
    }

    fn column_entries(&self, dir: &Path) -> Vec<Entry> {
        let mut entries = read_entries(dir, self.show_hidden);
        sort_entries(&mut entries, self.sort_key, self.sort_asc);
        entries
    }

    /// ↑/↓: move the Column-view selection within its own column, as the
    /// Mac does. `delta` is negative for ↑, positive for ↓.
    pub(super) fn column_move_vertical(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.col_stack.is_empty() {
            return;
        }
        let ci = self
            .column_selection
            .as_ref()
            .and_then(|selection| column_index_for_selection(&self.col_stack, selection))
            .unwrap_or(self.col_stack.len() - 1);
        let Some(dir) = self.col_stack.get(ci).cloned() else {
            return;
        };
        let entries = self.column_entries(&dir);
        let current = self
            .column_selection
            .as_ref()
            .map(|entry| entry.path.as_path());
        let Some(target) = column_vertical_target(&entries, current, delta) else {
            return;
        };
        let target = target.clone();
        self.select_column_entry(ci, target);
        cx.notify();
    }

    /// →: enter the selected folder's column, selecting its first row, as
    /// the Mac does. A no-op on a file, which has no next column, or when
    /// nothing is selected yet (in which case ↓ starts the selection).
    pub(super) fn column_move_right(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.column_selection.clone() else {
            self.column_move_vertical(1, cx);
            return;
        };
        if !selection.is_dir {
            return;
        }
        let Some(ci) = column_index_for_selection(&self.col_stack, &selection) else {
            return;
        };
        let next_ci = ci + 1;
        let Some(dir) = self.col_stack.get(next_ci).cloned() else {
            return;
        };
        let entries = self.column_entries(&dir);
        let Some(first) = entries.first().cloned() else {
            return;
        };
        self.select_column_entry(next_ci, first);
        cx.notify();
    }

    /// ←: go back to the parent column, selecting the folder just left, as
    /// the Mac does. A no-op at the leftmost column.
    pub(super) fn column_move_left(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = self.column_selection.clone() else {
            return;
        };
        let Some(ci) = column_index_for_selection(&self.col_stack, &selection) else {
            return;
        };
        if ci == 0 {
            return;
        }
        let parent_ci = ci - 1;
        let Some(parent_dir) = self.col_stack.get(parent_ci).cloned() else {
            return;
        };
        let Some(current_dir) = self.col_stack.get(ci).cloned() else {
            return;
        };
        let entries = self.column_entries(&parent_dir);
        let Some(target) = entries.into_iter().find(|entry| entry.path == current_dir) else {
            return;
        };
        self.select_column_entry(parent_ci, target);
        cx.notify();
    }

    /// macOS-style status bar: item / selection count + free space available.
    pub(super) fn render_status_bar(&self) -> impl IntoElement {
        let n = self.entries.len();
        let sel = self.selection_count();
        let count: SharedString = if sel > 0 {
            format!("{sel} of {n} selected").into()
        } else if let Some(summary) = &self.search_summary {
            summary.clone()
        } else {
            format!("{n} item{}", if n == 1 { "" } else { "s" }).into()
        };
        let free = self
            .free_bytes
            .map(|b| format!("{} available", human_size(b)))
            .unwrap_or_default();
        let status: SharedString = if free.is_empty() {
            count
        } else {
            format!("{}, {free}", count.as_ref()).into()
        };
        // design-lab/finder.html: 28 tall under a black 0.5 pt rule, 11 pt
        // centred text; icon view adds the 82 pt size slider 14 from the right.
        div()
            .h(px(STATUS_BAR_HEIGHT))
            .flex_none()
            .v_flex()
            .child(div().h(px(0.5)).flex_none().bg(dark_rule()))
            .child(
                div()
                    .flex_1()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rmac_ui::text_px(STATUS_TEXT))
                    .text_color(chrome_text())
                    .child(
                        div()
                            .id("status-text")
                            .role(Role::Status)
                            .aria_label(status.clone())
                            .child(status),
                    )
                    .when(self.view == ViewMode::Icon, |bar| {
                        bar.child(
                            div()
                                .absolute()
                                .right(px(STATUS_SLIDER_TRAILING))
                                .w(px(STATUS_SLIDER_WIDTH))
                                .h_full()
                                .flex()
                                .items_center()
                                .child(
                                    Slider::new(&self.icon_size_slider)
                                        .accessible_name("Icon size")
                                        .w_full(),
                                ),
                        )
                    }),
            )
    }

    /// View ▸ Show Path Bar: crumbs for the selected item (or the folder),
    /// each opening its folder when clicked.
    pub(super) fn render_path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let target = if self.trash_view || self.applications_view {
            None
        } else if self.selection_count() == 1 {
            self.selected_entry().map(|entry| entry.path.clone())
        } else {
            Some(self.cwd.clone())
        };
        let mut crumbs = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .items_center()
            .pl(px(PATH_BAR_LEADING))
            .overflow_hidden();
        if let Some(target) = target {
            let mut ancestors: Vec<PathBuf> = target.ancestors().map(Path::to_path_buf).collect();
            ancestors.reverse();
            for (index, path) in ancestors.into_iter().enumerate() {
                let is_dir = path.is_dir();
                let (glyph, name): (&'static str, String) = if path.parent().is_none() {
                    ("icons/hard-drive.svg", root_volume_name().to_string())
                } else if path == self.home {
                    (
                        "icons/house.svg",
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    )
                } else {
                    (
                        if is_dir {
                            "icons/folder-fill.svg"
                        } else {
                            "icons/file.svg"
                        },
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    )
                };
                if index > 0 {
                    crumbs = crumbs.child(
                        div()
                            .mx(px(PATH_BAR_SEPARATOR_MARGIN))
                            .flex_none()
                            .child(icon("icons/chevron-right.svg", 10.0, chrome_text())),
                    );
                }
                let destination = path.clone();
                crumbs = crumbs.child(
                    div()
                        .id(("path-crumb", index))
                        .role(Role::Link)
                        .aria_label(name.clone())
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(PATH_BAR_ICON_GAP))
                        .cursor_pointer()
                        .child(if glyph == "icons/folder-fill.svg" {
                            item_artwork(true, "", PATH_BAR_ICON)
                        } else {
                            icon(glyph, PATH_BAR_ICON, chrome_text()).into_any_element()
                        })
                        .child(name)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if destination.is_dir() {
                                this.navigate(destination.clone(), cx);
                            }
                        })),
                );
            }
        }
        div()
            .h(px(PATH_BAR_HEIGHT))
            .flex_none()
            .v_flex()
            .child(div().h(px(0.5)).flex_none().bg(hairline()))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .text_size(rmac_ui::text_px(STATUS_TEXT))
                    .text_color(chrome_text())
                    .child(crumbs),
            )
    }

    pub(super) fn drop_into(&mut self, dir: PathBuf, paths: &[PathBuf], cx: &mut Context<Self>) {
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
        self.start_transfer_with_conflicts("Moving", tasks, false, true, cx);
    }

    /// Files dropped from another app (Finder, etc.) → copy into the current dir.
    pub(super) fn drop_external(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
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
        self.start_transfer_with_conflicts("Copying", tasks, false, true, cx);
    }
}
