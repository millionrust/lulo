use super::*;

impl FinderView {
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
        can_open_with: bool,
        can_paste: bool,
        trash_view: bool,
        undo_label: Option<String>,
    ) -> rmac_ui::ContextMenu {
        let mut m = rmac_ui::ContextMenu::new(pos);
        if let Some(label) = undo_label {
            m = m
                .command_item(label, rmac_ui::shortcuts::UNDO, Box::new(UndoOperation))
                .separator();
        }
        if trash_view {
            if has_selection {
                m = m
                    .item("Restore", Box::new(RestoreItems))
                    .separator()
                    .danger_command_item(
                        "Delete Permanently…",
                        rmac_ui::shortcuts::DELETE_PERMANENT,
                        Box::new(DeletePermanently),
                    );
            }
            return m;
        }
        if has_selection {
            m = m.command_item(
                "Open",
                rmac_ui::shortcuts::OPEN_SELECTION,
                Box::new(OpenItems),
            );
            if can_open_with {
                m = m.item("Open With…", Box::new(OpenWith));
            }
            m = m
                .command_item("Rename", rmac_ui::shortcuts::ENTER, Box::new(RenameItem))
                .command_item(
                    "Duplicate",
                    rmac_ui::shortcuts::DUPLICATE,
                    Box::new(Duplicate),
                )
                .separator()
                .command_item("Copy", rmac_ui::shortcuts::COPY, Box::new(CopyItems))
                .command_item("Cut", rmac_ui::shortcuts::CUT, Box::new(CutItems));
        }
        if can_paste {
            m = m.command_item(
                "Paste Item",
                rmac_ui::shortcuts::PASTE,
                Box::new(PasteItems),
            );
        }
        m = m.separator().command_item(
            "New Folder",
            rmac_ui::shortcuts::NEW_FOLDER,
            Box::new(NewFolder),
        );
        if has_selection {
            m = m
                .separator()
                .command_item(
                    "Move to Trash",
                    rmac_ui::shortcuts::DELETE,
                    Box::new(MoveToTrash),
                )
                .danger_item("Delete Immediately", Box::new(DeleteItem));
        }
        m
    }

    // ---- list ----
    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
    pub(super) fn recursive_search(&mut self, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error = Some("Trash search filters the current list as you type".into());
            cx.notify();
            return;
        }
        let q = self.query.read(cx).value().trim().to_string();
        if q.is_empty() {
            return;
        }
        let cwd = self.cwd.clone();
        let title: SharedString = format!("Search: {}", sanitize_dialog_name(&q)).into();
        let include_hidden = self.show_hidden;
        let (generation, cancel) = self.begin_search();
        self.entries.clear();
        self.selected.clear();
        self.anchor = None;
        self.result_title = Some(title.clone());
        self.search_summary = Some("Searching…".into());
        self.search_relevance_order = true;
        self.view = ViewMode::List;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut options = rmac_search::Options::new(&cancel);
                    options.include_hidden = include_hidden;
                    let mut report = rmac_search::ranked(&cwd, &q, options)?;
                    let reported_matches = report.matches.len();
                    let entries = std::mem::take(&mut report.matches)
                        .into_iter()
                        .filter_map(|search_match| search_entry_for(&cwd, search_match))
                        .collect::<Vec<_>>();
                    report.skipped_errors = report
                        .skipped_errors
                        .saturating_add(reported_matches.saturating_sub(entries.len()));
                    let summary = ranked_search_summary(&report, entries.len());
                    Ok::<_, rmac_search::Error>((entries, summary))
                })
                .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_cancel = None;
                match result {
                    Ok((entries, summary)) => {
                        this.entries = entries;
                        this.result_title = Some(title);
                        this.search_summary = Some(summary.into());
                        this.selected.clear();
                        this.anchor = None;
                    }
                    Err(rmac_search::Error::Cancelled) => {}
                    Err(error) => {
                        this.search_summary = None;
                        this.search_relevance_order = false;
                        this.operation_error = Some(ranked_search_error_message(&error).into());
                    }
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

    fn render_open_with(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let picker = self.open_with.as_ref()?;
        let name = picker
            .path
            .file_name()
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
            .unwrap_or_else(|| "this file".into());
        let busy = picker.busy;

        let mut body = div().v_flex().gap_2().px_5().py_4();
        let mut can_open = false;
        let mut selected_is_default = false;
        if let Some(association) = &picker.association {
            body = body.child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary())
                    .child(format!(
                        "Choose an application for “{name}” ({})",
                        association.mime_type
                    )),
            );
            if association.handlers.is_empty() {
                body = body.child(
                    div()
                        .h(px(120.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(secondary())
                        .child("No installed application advertises support for this file type."),
                );
            } else {
                can_open = true;
                let mut rows = Vec::with_capacity(association.handlers.len());
                for (index, application) in association.handlers.iter().enumerate() {
                    let selected = index == picker.selected;
                    let is_default = association.default_application_id.as_deref()
                        == Some(application.id.as_str());
                    if selected {
                        selected_is_default = is_default;
                    }
                    rows.push(
                        div()
                            .id(("open-with-handler", index))
                            .h(px(34.0))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .when(selected, |element: Stateful<Div>| element.bg(sel()))
                            .when(!selected, |element: Stateful<Div>| {
                                element.hover(|hover| hover.bg(rmac_ui::mac::hover()))
                            })
                            .child(icon(
                                "icons/file-fill.svg",
                                16.0,
                                if selected { white() } else { secondary() },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .text_color(if selected { white() } else { label() })
                                    .child(sanitize_dialog_name(&application.name)),
                            )
                            .when(is_default, |element: Stateful<Div>| {
                                element.child(
                                    div()
                                        .text_size(rmac_ui::text_px(11.0))
                                        .text_color(if selected { white() } else { secondary() })
                                        .child("Default"),
                                )
                            })
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.choose_open_with(index, cx)),
                            )
                            .into_any_element(),
                    );
                }
                body = body.child(
                    div()
                        .id("open-with-list")
                        .max_h(px(260.0))
                        .overflow_y_scroll()
                        .v_flex()
                        .gap_0p5()
                        .children(rows),
                );

                let checked = selected_is_default || picker.make_default;
                body = body.child(
                    div()
                        .id("open-with-default")
                        .h(px(28.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(5.0))
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(if selected_is_default {
                            secondary()
                        } else {
                            label()
                        })
                        .when(!selected_is_default && !busy, |element: Stateful<Div>| {
                            element.cursor_pointer().on_click(
                                cx.listener(|this, _, _, cx| this.toggle_open_with_default(cx)),
                            )
                        })
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(16.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(if checked {
                                    rmac_ui::mac::accent()
                                } else {
                                    sep()
                                })
                                .bg(if checked {
                                    rmac_ui::mac::accent()
                                } else {
                                    rmac_ui::mac::raised()
                                })
                                .text_color(rmac_ui::mac::on_accent())
                                .child(if checked { "✓" } else { "" }),
                        )
                        .child(if selected_is_default {
                            "This application is already the default"
                        } else {
                            "Always open this file type with this application"
                        }),
                );
            }
        } else {
            body = body.child(
                div()
                    .h(px(150.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(secondary())
                    .child("Finding compatible applications…"),
            );
        }
        if let Some(error) = &picker.error {
            body = body.child(
                div()
                    .id("open-with-error")
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rmac_ui::mac::error_border())
                    .bg(rmac_ui::mac::error_background())
                    .px_3()
                    .py_2()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(rmac_ui::mac::danger())
                    .child(error.clone()),
            );
        }

        let buttons = div()
            .h(px(54.0))
            .flex()
            .items_center()
            .justify_end()
            .gap_2()
            .px_5()
            .border_t_1()
            .border_color(sep())
            .child(
                rmac_ui::dialog_button(
                    "open-with-cancel",
                    if can_open { "Cancel" } else { "Close" },
                    rmac_ui::DialogButtonKind::Normal,
                )
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.close_open_with(cx))),
            )
            .child(
                rmac_ui::dialog_button(
                    "open-with-confirm",
                    if busy { "Opening…" } else { "Open" },
                    rmac_ui::DialogButtonKind::Primary,
                )
                .busy(busy)
                .disabled(busy || !can_open)
                .on_click(cx.listener(|this, _, _, cx| this.confirm_open_with(cx))),
            );

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rmac_ui::mac::scrim())
                .child(
                    div()
                        .w(px(440.0))
                        .max_h(px(520.0))
                        .v_flex()
                        .rounded(px(12.0))
                        .bg(rmac_ui::mac::raised())
                        .border_1()
                        .border_color(sep())
                        .shadow_lg()
                        .child(
                            div()
                                .h(px(48.0))
                                .flex()
                                .items_center()
                                .px_5()
                                .border_b_1()
                                .border_color(sep())
                                .text_size(rmac_ui::text_px(15.0))
                                .font_weight(rmac_ui::mac::SEMIBOLD)
                                .text_color(label())
                                .child("Open With"),
                        )
                        .child(body)
                        .child(buttons),
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
        let can_open_with = !self.trash_view
            && self.selected.len() == 1
            && self
                .selected
                .iter()
                .next()
                .and_then(|index| self.entries.get(*index))
                .is_some_and(|entry| !entry.is_dir);
        let can_paste = !self.clipboard.is_empty();
        let undo_label = self
            .undo_available
            .as_ref()
            .map(|available| available.label.clone());
        let operation_notice = self.operation_notice.clone();
        let operation_error = self.operation_error.clone();
        let transfer = self.transfer.clone();
        let undo_progress = self.undo_operation.clone();
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
        let open_with_dialog = self.render_open_with(cx);
        let quick_look_dialog = self.render_quick_look(cx);
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
                if this.quick_look.is_some() {
                    cx.stop_propagation();
                    match event.keystroke.key.as_str() {
                        "escape" | "space" => this.close_quick_look(cx),
                        "left" => this.move_quick_look(-1, cx),
                        "right" => this.move_quick_look(1, cx),
                        _ => {}
                    }
                    return;
                }
                if this.open_with.is_some() {
                    cx.stop_propagation();
                    match event.keystroke.key.as_str() {
                        "escape" => this.close_open_with(cx),
                        "up" => this.move_open_with_selection(-1, cx),
                        "down" => this.move_open_with_selection(1, cx),
                        "enter" => this.confirm_open_with(cx),
                        "space" => this.toggle_open_with_default(cx),
                        _ => {}
                    }
                    return;
                }
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
            .when_some(undo_progress, |el, undo| {
                let status = match undo.phase {
                    file_ops::TransferPhase::Scanning => {
                        format!("{} — Checking items", undo.label)
                    }
                    file_ops::TransferPhase::Copying if undo.bytes_processed != 0 => format!(
                        "{} — Restoring {}",
                        undo.label,
                        human_size(undo.bytes_processed)
                    ),
                    file_ops::TransferPhase::Copying => {
                        format!("{} — Restoring item", undo.label)
                    }
                    file_ops::TransferPhase::Finishing => {
                        format!("{} — Finishing safely", undo.label)
                    }
                };
                el.child(
                    div()
                        .id("undo-progress")
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
                                .id("cancel-undo")
                                .px_2()
                                .py_0p5()
                                .rounded(px(5.0))
                                .bg(rmac_ui::mac::raised())
                                .border_1()
                                .border_color(rmac_ui::mac::accent_border())
                                .cursor_pointer()
                                .child(if undo.cancelling {
                                    "Cancelling…"
                                } else {
                                    "Cancel"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_undo(cx))),
                        ),
                )
            })
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
                    Self::build_context_menu(
                        pos,
                        has_sel,
                        can_open_with,
                        can_paste,
                        self.trash_view,
                        undo_label,
                    )
                    .render(),
                )
            })
            .when_some(conflict_dialog, |el, dialog| el.child(dialog))
            .when_some(recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(trash_recovery_dialog, |el, dialog| el.child(dialog))
            .when_some(delete_dialog, |el, dialog| el.child(dialog))
            .when_some(open_with_dialog, |el, dialog| el.child(dialog))
            .when_some(quick_look_dialog, |el, dialog| el.child(dialog))
    }
}
