use super::*;

impl FinderView {
    pub(super) fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Recursive content matches may not contain the query in their names.
        // Local filtering remains active until Return starts a ranked search.
        let q = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };

        let sort_caret = |key: SortKey| -> Option<Svg> {
            if !self.search_relevance_order && self.sort_key == key {
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
            let search_detail = e.search_detail.clone();
            let has_search_detail = search_detail.is_some();

            let name_cell: gpui::AnyElement = match &self.renaming {
                Some((ri, input)) if *ri == ix => div()
                    .pl(px(6.0))
                    .flex_1()
                    .child(TextField::new(input).appearance(true))
                    .into_any_element(),
                _ => div()
                    .pl(px(6.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .v_flex()
                    .justify_center()
                    .child(div().truncate().text_color(primary).child(e.name.clone()))
                    .when_some(search_detail, |el, detail| {
                        el.child(
                            div()
                                .truncate()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(sub)
                                .child(detail),
                        )
                    })
                    .into_any_element(),
            };

            rows.push(
                div()
                    .id(("row", ix))
                    .flex()
                    .items_center()
                    .h(px(if has_search_detail { 38.0 } else { 24.0 }))
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
            .on_action(cx.listener(|this, _: &UndoOperation, _, cx| this.start_undo(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &GoUp, _, cx| this.go_up(cx)))
            .on_action(cx.listener(|this, _: &OpenItems, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &OpenWith, _, cx| this.request_open_with(cx)))
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
                                this.open_paths(vec![ep.clone()], cx);
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
}
