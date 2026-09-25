use super::*;

impl FinderView {
    pub(in crate::view) fn open_context_menu(
        &mut self,
        index: Option<usize>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match index {
            Some(index) if !self.selected.contains(&index) => self.select_single(index),
            Some(_) => {}
            None => {
                self.selected.clear();
                self.anchor = None;
            }
        }
        self.menu_purpose = MenuPurpose::Context;
        self.menu_at = Some(rmac_ui::ContextMenuState::open(
            position,
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    pub(in crate::view) fn open_column_context_menu(
        &mut self,
        entry: Entry,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected.clear();
        self.anchor = None;
        self.column_selection = Some(entry);
        self.menu_purpose = MenuPurpose::Context;
        self.menu_at = Some(rmac_ui::ContextMenuState::open(
            position,
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    pub(super) fn render_list(
        &self,
        window_active: bool,
        window_height: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Recursive content matches may not contain the query in their names.
        // Local filtering remains active until Return starts a ranked search.
        let q = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };
        let entity = cx.entity();
        let visible_count = self
            .entries
            .iter()
            .filter(|entry| q.is_empty() || entry.name.to_lowercase().contains(&q))
            .count();
        let listing_name = self.title();

        // design-lab/finder.html: a 28 pt header, 11 pt labels, the sorted
        // column in semibold with its chevron, 1 × 16 column dividers and a
        // 0.5 pt hairline underneath.
        let head = |id: &'static str,
                    text: &'static str,
                    key: SortKey,
                    width: Option<f32>,
                    divider: bool,
                    text_x: f32| {
            let active = !self.search_relevance_order && self.sort_key == key;
            div()
                .id(id)
                .h_full()
                .flex()
                .items_center()
                .when_some(width, |cell, width| cell.w(px(width)).flex_none())
                .when(width.is_none(), |cell| cell.flex_1().min_w(px(0.0)))
                .when(divider, |cell| {
                    cell.child(
                        div()
                            .w(px(1.0))
                            .h(px(LIST_HEADER_DIVIDER_HEIGHT))
                            .flex_none()
                            .bg(header_divider()),
                    )
                })
                .child(
                    div()
                        .pl(px(if divider { text_x - 1.0 } else { text_x }))
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
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
                        .child(text),
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
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
        };

        let header = div()
            .h(px(LIST_HEADER_HEIGHT))
            .flex_none()
            .v_flex()
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .mx(px(LIST_ROW_INSET))
                    .child(head(
                        "head-name",
                        "Name",
                        SortKey::Name,
                        None,
                        false,
                        LIST_DISCLOSURE_X + LIST_DISCLOSURE_WIDTH + LIST_ICON + LIST_ICON_TO_NAME,
                    ))
                    .child(head(
                        "head-date",
                        "Date Modified",
                        SortKey::Date,
                        Some(DATE_W),
                        true,
                        LIST_CELL_TEXT_X,
                    ))
                    .child(head(
                        "head-size",
                        "Size",
                        SortKey::Size,
                        Some(SIZE_W),
                        true,
                        LIST_CELL_TEXT_X,
                    ))
                    .child(head(
                        "head-kind",
                        "Kind",
                        SortKey::Kind,
                        Some(KIND_W),
                        true,
                        LIST_CELL_TEXT_X,
                    )),
            )
            .child(div().h(px(0.5)).flex_none().bg(hairline()));

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut stripe_index = 0usize;
        for (ix, e) in self.entries.iter().enumerate() {
            if !q.is_empty() && !e.name.to_lowercase().contains(&q) {
                continue;
            }
            let striped = stripe_index % 2 == 1;
            let position = stripe_index;
            stripe_index += 1;
            let selected = self.selected.contains(&ix);
            let primary = if selected {
                selected_text(window_active)
            } else {
                primary_text()
            };
            let sub = if selected {
                selected_text(window_active)
            } else {
                secondary_text()
            };
            // Finder's list rows show the file's own thumbnail when it has
            // one, else the full-colour folder or page artwork.
            let row_icon: gpui::AnyElement = e
                .application
                .as_ref()
                .and_then(|application| application.icon.clone())
                .map_or_else(
                    || match self.thumbs.get(&e.path) {
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
                        None => item_artwork(e.is_dir, &e.name, LIST_ICON),
                    },
                    |path| {
                        img(path)
                            .w(px(LIST_ICON))
                            .h(px(LIST_ICON))
                            .rounded(px(rmac_ui::mac::radius_menu_item()))
                            .into_any_element()
                    },
                );

            let drag_paths: Vec<PathBuf> = if selected {
                self.selected_paths()
            } else {
                vec![e.path.clone()]
            };
            let drag_count = drag_paths.len();
            let drop_dir = e.path.clone();
            let spring_dir = e.path.clone();
            let row_is_dir = e.is_dir;
            let search_detail = e.search_detail.clone();
            let has_search_detail = search_detail.is_some();

            let name_cell: gpui::AnyElement = match &self.renaming {
                Some((rename_path, input)) if rename_path == &e.path => div()
                    .pl(px(LIST_ICON_TO_NAME))
                    .flex_1()
                    .child(TextField::new(input).appearance(true))
                    .into_any_element(),
                _ => div()
                    .id(("list-name", ix))
                    .pl(px(LIST_ICON_TO_NAME))
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
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            if selected
                                && !this.trash_view
                                && !this.applications_view
                                && !ev.modifiers.platform
                                && !ev.modifiers.shift
                            {
                                cx.stop_propagation();
                                this.rename_start(window, cx);
                            }
                        }),
                    )
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        if selected {
                            cx.stop_propagation();
                        }
                    }))
                    .into_any_element(),
            };

            rows.push(
                accessible_item(
                    div().id(("row", ix)),
                    Role::ListBoxOption,
                    e.name.clone(),
                    selected,
                    position,
                    visible_count,
                    &entity,
                    move |this, window, cx| this.accessible_select(ix, window, cx),
                )
                .flex_none()
                .flex()
                .items_center()
                .h(px(if has_search_detail {
                    38.0
                } else {
                    LIST_ROW_HEIGHT
                }))
                .mx(px(LIST_ROW_INSET))
                .rounded(px(ROW_RADIUS))
                .text_size(rmac_ui::text_px(13.0))
                .when(selected, |el: Stateful<Div>| {
                    el.bg(selection(window_active))
                })
                .when(!selected && striped, |el: Stateful<Div>| el.bg(stripe()))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .w(px(LIST_DISCLOSURE_X + LIST_DISCLOSURE_WIDTH))
                                .flex_none()
                                .flex()
                                .justify_center()
                                .pl(px(LIST_DISCLOSURE_X))
                                .when(e.is_dir, |el: Div| {
                                    el.child(icon(
                                        "icons/chevron-right.svg",
                                        12.0,
                                        if selected {
                                            selected_text(window_active)
                                        } else {
                                            chrome_text()
                                        },
                                    ))
                                }),
                        )
                        .child(row_icon)
                        .child(name_cell),
                )
                .child(
                    div()
                        .w(px(DATE_W))
                        .flex_none()
                        .pl(px(LIST_CELL_TEXT_X))
                        .truncate()
                        .text_color(sub)
                        .child(e.modified.clone()),
                )
                .child(
                    div()
                        .w(px(SIZE_W))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .pr(px(LIST_SIZE_TRAILING))
                        .text_color(sub)
                        .child(e.size.clone()),
                )
                .child(
                    div()
                        .w(px(KIND_W))
                        .flex_none()
                        .pl(px(LIST_CELL_TEXT_X))
                        .text_color(sub)
                        .truncate()
                        .child(e.kind.clone()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        if ev.modifiers.control {
                            this.open_context_menu(Some(ix), ev.position, window, cx);
                            return;
                        }
                        this.handle_click(ix, ev.modifiers.platform, ev.modifiers.shift);
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_context_menu(Some(ix), ev.position, window, cx);
                    }),
                )
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    if ev.click_count() >= 2 {
                        this.open_index(ix, cx);
                    }
                    window.focus(&this.focus, cx);
                }))
                .when(
                    !self.trash_view && !self.applications_view,
                    |el: Stateful<Div>| {
                        el.on_drag(DraggedPaths(drag_paths), move |_, _, _, cx| {
                            cx.new(|_| DragPreview { count: drag_count })
                        })
                    },
                )
                .when(
                    row_is_dir && !self.trash_view && !self.applications_view,
                    |el: Stateful<Div>| {
                        let dd = drop_dir.clone();
                        el.drag_over::<DraggedPaths>(|s, _, _, _| {
                            s.bg(rmac_ui::mac::accent_subtle())
                        })
                        .on_drag_move(cx.listener(
                            move |this, event: &gpui::DragMoveEvent<DraggedPaths>, _, cx| {
                                let inside = event.bounds.contains(&event.event.position);
                                this.spring_hover(spring_dir.clone(), inside, cx);
                            },
                        ))
                        .on_drop(cx.listener(
                            move |this, p: &DraggedPaths, _, cx| {
                                this.drop_into(dd.clone(), &p.0, cx)
                            },
                        ))
                    },
                )
                .into_any_element(),
            );
        }
        // Finder keeps striping the empty area below the last row.
        let filler_start = stripe_index;
        let filler = div()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .v_flex()
            .children((filler_start..filler_start + FILLER_STRIPES).map(|index| {
                div()
                    .h(px(LIST_ROW_HEIGHT))
                    .flex_none()
                    .mx(px(LIST_ROW_INSET))
                    .rounded(px(ROW_RADIUS))
                    .when(index % 2 == 1, |row| row.bg(stripe()))
            }));

        let show_list = self.view == ViewMode::List;
        let show_icons = self.view == ViewMode::Icon;
        let visible_indices = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                (q.is_empty() || entry.name.to_lowercase().contains(&q)).then_some(index)
            })
            .collect::<Vec<_>>();
        let navigation_indices = visible_indices.clone();
        let horizontal_navigation = matches!(self.view, ViewMode::Icon | ViewMode::Gallery);
        let icon_columns = if show_icons { self.icon_columns() } else { 1 };

        // Icon-grid tiles. Gallery owns a distinct preview + filmstrip tree.
        let mut tiles: Vec<gpui::AnyElement> = Vec::new();
        if show_icons {
            let icon_size = self.icon_size;
            let (tile_width, tile_height) = self.icon_cell();
            let label_width = ICON_LABEL_MAX_WIDTH.min(tile_width - 16.0);
            let mut position = 0usize;
            for (ix, e) in self.entries.iter().enumerate() {
                if !q.is_empty() && !e.name.to_lowercase().contains(&q) {
                    continue;
                }
                let tile_position = position;
                position += 1;
                let selected = self.selected.contains(&ix);
                let visual: gpui::AnyElement = if let Some(path) = e
                    .application
                    .as_ref()
                    .and_then(|application| application.icon.clone())
                {
                    img(path)
                        .w(px(icon_size))
                        .h(px(icon_size))
                        .rounded(px(icon_size * 0.22))
                        .into_any_element()
                } else {
                    match self.thumbs.get(&e.path) {
                        Some(t) => img(t.clone())
                            .max_w(px(icon_size))
                            .max_h(px(icon_size - 6.0))
                            .rounded(px(rmac_ui::mac::radius_menu_item()))
                            .into_any_element(),
                        None => item_artwork(e.is_dir, &e.name, icon_size),
                    }
                };
                let drag_paths = if selected {
                    self.selected_paths()
                } else {
                    vec![e.path.clone()]
                };
                let drag_count = drag_paths.len();
                let drop_directory = e.path.clone();
                let spring_dir = e.path.clone();
                let is_directory = e.is_dir;
                let tile_label: gpui::AnyElement = match &self.renaming {
                    Some((rename_path, input)) if rename_path == &e.path => div()
                        .mt(px(ICON_LABEL_GAP - ICON_PLATE_GROW))
                        .w(px(label_width))
                        .child(TextField::new(input).appearance(true))
                        .into_any_element(),
                    _ => div()
                        .id(("tile-name", ix))
                        .mt(px(ICON_LABEL_GAP - ICON_PLATE_GROW))
                        .max_w(px(label_width))
                        .px(px(5.0))
                        .py(px(1.0))
                        .rounded(px(ICON_LABEL_RADIUS))
                        .when(selected, |el: Stateful<Div>| {
                            el.bg(selection(window_active))
                        })
                        .text_size(rmac_ui::text_px(ICON_LABEL_SIZE))
                        .line_height(px(15.0))
                        .text_center()
                        .line_clamp(2)
                        .text_color(if selected {
                            selected_text(window_active)
                        } else {
                            primary_text()
                        })
                        .child(e.name.clone())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                if selected
                                    && !this.trash_view
                                    && !this.applications_view
                                    && !ev.modifiers.platform
                                    && !ev.modifiers.shift
                                {
                                    cx.stop_propagation();
                                    this.rename_start(window, cx);
                                }
                            }),
                        )
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            if selected {
                                cx.stop_propagation();
                            }
                        }))
                        .into_any_element(),
                };
                tiles.push(
                    accessible_item(
                        div().id(("tile", ix)),
                        Role::ListBoxOption,
                        e.name.clone(),
                        selected,
                        tile_position,
                        visible_count,
                        &entity,
                        move |this, window, cx| this.accessible_select(ix, window, cx),
                    )
                    .w(px(tile_width))
                    .h(px(tile_height))
                    .flex_none()
                    .v_flex()
                    .items_center()
                    .child(
                        // The selected plate grows 4 pt around the icon;
                        // the negative margin keeps the icon itself on
                        // the measured grid.
                        div()
                            .mt(px(-ICON_PLATE_GROW))
                            .w(px(icon_size + 2.0 * ICON_PLATE_GROW))
                            .h(px(icon_size + 2.0 * ICON_PLATE_GROW))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(ICON_PLATE_RADIUS))
                            .when(selected, |plate| plate.bg(icon_plate()))
                            .child(visual),
                    )
                    .child(tile_label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if ev.modifiers.control {
                                this.open_context_menu(Some(ix), ev.position, window, cx);
                                return;
                            }
                            this.handle_click(ix, ev.modifiers.platform, ev.modifiers.shift);
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_context_menu(Some(ix), ev.position, window, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        if ev.click_count() >= 2 {
                            this.open_index(ix, cx);
                        }
                        window.focus(&this.focus, cx);
                    }))
                    .when(!self.trash_view && !self.applications_view, |element| {
                        element.on_drag(DraggedPaths(drag_paths), move |_, _, _, cx| {
                            cx.new(|_| DragPreview { count: drag_count })
                        })
                    })
                    .when(
                        is_directory && !self.trash_view && !self.applications_view,
                        |element| {
                            element
                                .drag_over::<DraggedPaths>(|style, _, _, _| {
                                    style.bg(rmac_ui::mac::accent_subtle())
                                })
                                .on_drag_move(cx.listener(
                                    move |this,
                                          event: &gpui::DragMoveEvent<DraggedPaths>,
                                          _,
                                          cx| {
                                        let inside = event.bounds.contains(&event.event.position);
                                        this.spring_hover(spring_dir.clone(), inside, cx);
                                    },
                                ))
                                .on_drop(cx.listener(move |this, paths: &DraggedPaths, _, cx| {
                                    this.drop_into(drop_directory.clone(), &paths.0, cx)
                                }))
                        },
                    )
                    .into_any_element(),
                );
            }
        }

        // Finished searches only: while a ranked search runs, the list is
        // empty and its summary reads "Searching…".
        let no_matching_items = visible_count == 0
            && matches!(self.view, ViewMode::List | ViewMode::Icon)
            && self.search_cancel.is_none()
            && (self.search_summary.is_some() || !q.trim().is_empty());
        let content = if self.trash_view && self.entries.is_empty() {
            let empty_title = format!("{} is Empty", self.file_words.bin());
            let empty_message =
                format!("Items moved to {} will appear here.", self.file_words.bin());
            div()
                .id("trash-empty")
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .child(rmac_ui::EmptyState::new(empty_title).message(empty_message))
                .into_any_element()
        } else if no_matching_items {
            // As Gallery does: a search that found nothing says so, rather
            // than leaving an empty list.
            div()
                .id("search-empty")
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    rmac_ui::EmptyState::new("No Matching Items")
                        .message("Try a different search."),
                )
                .into_any_element()
        } else {
            match self.view {
                ViewMode::List => div()
                    .id("file-list")
                    .role(Role::ListBox)
                    .aria_label(listing_name.clone())
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .min_h(gpui::relative(1.0))
                            .v_flex()
                            .pt(px(LIST_ROWS_TOP))
                            .children(rows)
                            .child(filler),
                    )
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
                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_context_menu(None, ev.position, window, cx);
                        }),
                    )
                    .into_any_element(),
                ViewMode::Column => self.render_columns(window_active, cx).into_any_element(),
                ViewMode::Gallery => self
                    .render_gallery(&visible_indices, window_height, cx)
                    .into_any_element(),
                ViewMode::Icon => {
                    let marquee = self.marquee_rect();
                    div()
                        .id("icon-grid")
                        .role(Role::ListBox)
                        .aria_label(listing_name.clone())
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .track_scroll(&self.icon_scroll)
                        .child(
                            div()
                                .relative()
                                .min_h(gpui::relative(1.0))
                                .w_full()
                                .pl(px(ICON_GRID_LEFT))
                                .pt(px(ICON_GRID_TOP))
                                .flex()
                                .flex_wrap()
                                .content_start()
                                .children(tiles)
                                .when_some(marquee, |grid, bounds| {
                                    grid.child(
                                        div()
                                            .absolute()
                                            .left(bounds.origin.x)
                                            .top(bounds.origin.y)
                                            .w(bounds.size.width)
                                            .h(bounds.size.height)
                                            .bg(marquee_fill())
                                            .border_1()
                                            .border_color(marquee_edge()),
                                    )
                                }),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                this.begin_marquee(
                                    ev.position,
                                    ev.modifiers.platform || ev.modifiers.shift,
                                );
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }),
                        )
                        .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                            if ev.pressed_button == Some(MouseButton::Left) {
                                this.update_marquee(ev.position, cx);
                            }
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.end_marquee(cx)),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.end_marquee(cx)),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_context_menu(None, ev.position, window, cx);
                            }),
                        )
                        .into_any_element()
                }
            }
        };

        div()
            .track_focus(&self.focus)
            .key_context("Finder")
            .on_action(cx.listener(|this, _: &NewFolder, window, cx| this.new_folder(window, cx)))
            .on_action(
                cx.listener(|this, _: &RenameItem, window, cx| this.rename_selected(window, cx)),
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
            .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &GoUp, _, cx| this.go_up(cx)))
            .on_action(cx.listener(|this, _: &GoHome, _, cx| this.go_home(cx)))
            .on_action(cx.listener(|this, _: &GoApplications, _, cx| this.applications_click(cx)))
            .on_action(cx.listener(|this, _: &GoDownloads, _, cx| this.go_downloads(cx)))
            .on_action(cx.listener(|this, _: &GoDesktop, _, cx| this.go_desktop(cx)))
            .on_action(cx.listener(|this, _: &GoDocuments, _, cx| this.go_documents(cx)))
            .on_action(cx.listener(|this, _: &GoRecents, _, cx| this.recents_click(cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.open_search(window, cx)))
            .on_action(cx.listener(|this, _: &CopyAsPathname, _, cx| this.copy_as_pathname(cx)))
            .on_action(cx.listener(|this, _: &MoveItemHere, _, cx| this.move_item_here(cx)))
            .on_action(cx.listener(|this, _: &GoTrash, _, cx| this.trash_click(cx)))
            .on_action(cx.listener(|this, _: &OpenItems, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &OpenWith, _, cx| this.request_open_with(cx)))
            .on_action(cx.listener(|this, _: &ToggleHidden, _, cx| this.toggle_hidden(cx)))
            .on_action(cx.listener(|this, _: &QuickLook, _, cx| this.quick_look(cx)))
            .on_action(cx.listener(|this, _: &Compress, _, cx| this.compress_selection(cx)))
            .on_action(cx.listener(|this, _: &GetInfo, window, cx| this.get_info(window, cx)))
            .on_action(
                cx.listener(|this, _: &ViewAsIcons, _, cx| {
                    this.select_view_mode(ViewMode::Icon, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ViewAsList, _, cx| {
                    this.select_view_mode(ViewMode::List, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ViewAsColumns, _, cx| {
                this.select_view_mode(ViewMode::Column, cx)
            }))
            .on_action(cx.listener(|this, _: &ViewAsGallery, _, cx| {
                this.select_view_mode(ViewMode::Gallery, cx)
            }))
            .on_action(cx.listener(|this, _: &SortByName, _, cx| this.set_sort(SortKey::Name, cx)))
            .on_action(cx.listener(|this, _: &SortByDate, _, cx| this.set_sort(SortKey::Date, cx)))
            .on_action(cx.listener(|this, _: &SortBySize, _, cx| this.set_sort(SortKey::Size, cx)))
            .on_action(cx.listener(|this, _: &SortByKind, _, cx| this.set_sort(SortKey::Kind, cx)))
            .on_action(cx.listener(|this, _: &NewTab, _, cx| this.new_tab(cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.close_tab_or_window(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.select_adjacent_tab(-1, cx)))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.select_adjacent_tab(1, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &TogglePathBar, _, cx| {
                this.show_path_bar = !this.show_path_bar;
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &GoComputer, _, cx| this.navigate(PathBuf::from("/"), cx)),
            )
            .on_action(cx.listener(|this, _: &NewWindow, _, cx| this.new_window(cx)))
            .on_action(
                cx.listener(|this, _: &GoToFolder, window, cx| this.open_go_to_folder(window, cx)),
            )
            .on_action(cx.listener(|this, _: &EmptyTrash, _, cx| this.request_empty_trash(cx)))
            .on_action(cx.listener(|this, _: &ShowHelp, _, cx| {
                this.help_open = true;
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                if this.renaming.is_some() {
                    return;
                }
                // Column view is a browser: ↑/↓ move within the focused
                // column, →/← cross into the child/parent column, and
                // `entries`-based indices (below) don't apply to it.
                if this.view == ViewMode::Column && !this.applications_view && !this.trash_view {
                    match ev.keystroke.key.as_str() {
                        "up" => this.column_move_vertical(-1, cx),
                        "down" => this.column_move_vertical(1, cx),
                        "right" => this.column_move_right(cx),
                        "left" => this.column_move_left(cx),
                        "escape" if this.info.take().is_some() => cx.notify(),
                        _ => {}
                    }
                    return;
                }
                let current = this.anchor.and_then(|anchor| {
                    navigation_indices.iter().position(|index| *index == anchor)
                });
                let last = navigation_indices.len().saturating_sub(1);
                // Icon view moves by whole rows vertically, as in Finder.
                let vertical_step = icon_columns.max(1);
                let select_position = match ev.keystroke.key.as_str() {
                    "down" => Some(
                        current
                            .map(|position| position + vertical_step)
                            .unwrap_or(0)
                            .min(last),
                    ),
                    "right" if horizontal_navigation => {
                        Some(current.map(|position| position + 1).unwrap_or(0).min(last))
                    }
                    "up" => Some(
                        current
                            .map(|position| position.saturating_sub(vertical_step))
                            .unwrap_or(0),
                    ),
                    "left" if horizontal_navigation => Some(
                        current
                            .map(|position| position.saturating_sub(1))
                            .unwrap_or(0),
                    ),
                    "home" => Some(0),
                    "end" => Some(last),
                    _ => None,
                };
                match ev.keystroke.key.as_str() {
                    "escape" => {
                        if this.info.take().is_some() {
                            cx.notify();
                        }
                    }
                    _ => {
                        if let Some(position) = select_position {
                            if !navigation_indices.is_empty() {
                                this.select_single(navigation_indices[position]);
                                cx.notify();
                            }
                        } else if let Some(text) = type_select_text(ev) {
                            this.type_select(&text, &navigation_indices, cx);
                        }
                    }
                }
            }))
            .when(!self.applications_view && !self.trash_view, |element| {
                element
                    .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rmac_ui::mac::accent_subtle()))
                    .on_drop(cx.listener(|this, ep: &ExternalPaths, _, cx| {
                        this.drop_external(ep.paths().to_vec(), cx)
                    }))
            })
            .flex_1()
            .v_flex()
            .overflow_hidden()
            .bg(list_bg())
            .when(show_list, |el: Div| el.child(header))
            .child(content)
            .when(self.show_path_bar, |el: Div| {
                el.child(self.render_path_bar(cx))
            })
            .child(self.render_status_bar())
    }
}
