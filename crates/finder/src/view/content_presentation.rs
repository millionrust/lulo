use super::*;

impl FinderView {
    fn render_column_preview(&self, entry: &Entry) -> gpui::AnyElement {
        let visual = entry
            .application
            .as_ref()
            .and_then(|application| application.icon.clone())
            .map(|path| {
                img(path)
                    .max_w(px(256.0))
                    .max_h(px(256.0))
                    .rounded(px(rmac_ui::mac::radius_popover()))
                    .into_any_element()
            })
            .or_else(|| {
                self.thumbs.get(&entry.path).map(|thumbnail| {
                    img(thumbnail.clone())
                        .max_w(px(256.0))
                        .max_h(px(256.0))
                        .rounded(px(rmac_ui::mac::radius_control()))
                        .into_any_element()
                })
            })
            .unwrap_or_else(|| icon("icons/file-fill.svg", 128.0, secondary()).into_any_element());

        div()
            .id("column-preview")
            .min_w(px(COLUMN_WIDTH))
            .flex_1()
            .h_full()
            .v_flex()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .border_r_1()
            .border_color(sep())
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
                    .max_w(px(256.0))
                    .truncate()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(label())
                    .child(entry.name.clone()),
            )
            .child(
                div()
                    .max_w(px(256.0))
                    .truncate()
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(secondary())
                    .child(format!(
                        "{}  •  {}  •  {}",
                        entry.kind, entry.size, entry.modified
                    )),
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
        let mut row = div()
            .id("columns")
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
            for e in entries {
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
                let ep = e.path.clone();
                let open_path = e.path.clone();
                let is_dir = e.is_dir;
                let selected_entry = e.clone();
                let context_entry = e.clone();
                let rename_entry = e.clone();
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
                let glyph = if is_dir {
                    "icons/folder-artwork.svg"
                } else {
                    "icons/file-fill.svg"
                };
                let icol = if is_dir {
                    folder_blue()
                } else if is_sel {
                    row_text
                } else {
                    secondary_text()
                };
                col = col.child(
                    div()
                        .id(SharedString::from(format!("colrow-{ci}-{}", e.name)))
                        .flex()
                        .items_center()
                        .flex_none()
                        .h(px(COLUMN_ROW_HEIGHT))
                        .mx(px(COLUMN_ROW_INSET))
                        .pl(px(COLUMN_ICON_X))
                        .rounded(px(ROW_RADIUS))
                        .text_size(rmac_ui::text_px(13.0))
                        .when(is_sel, |el: Stateful<Div>| el.bg(selection(row_active)))
                        .child(icon(glyph, LIST_ICON, icol))
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
                                this.selected.clear();
                                this.anchor = None;
                                this.column_selection = Some(selected_entry.clone());
                                this.col_stack.truncate(ci + 1);
                                if is_dir {
                                    this.col_stack.push(ep.clone());
                                }
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
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            if event.click_count() < 2 || is_dir {
                                return;
                            }
                            this.open_paths(vec![open_path.clone()], cx);
                        })),
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
            .w(px(COLUMN_WIDTH))
            .h_full()
            .flex_none()
            .overflow_y_scroll()
            .v_flex()
            .pt(px(COLUMN_ROWS_TOP))
            .border_r_1()
            .border_color(dark_rule());

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
                div()
                    .id(("application-column-row", index))
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
                            this.handle_click(
                                index,
                                event.modifiers.platform,
                                event.modifiers.shift,
                            );
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
                        .child(icon(
                            glyph,
                            PATH_BAR_ICON,
                            if glyph == "icons/folder-fill.svg" {
                                folder_blue()
                            } else {
                                chrome_text()
                            },
                        ))
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
