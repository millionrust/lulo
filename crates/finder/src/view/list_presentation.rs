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
        self.load_open_with_menu(cx);
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
        content_width: f32,
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
        let options = self.current_options();
        let list_icon = if options.list_large_icons {
            24.0
        } else {
            LIST_ICON
        };
        let list_row_height = if options.list_large_icons {
            30.0
        } else {
            LIST_ROW_HEIGHT
        };

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

        let plain_head = |id: &'static str, text: &'static str, width: f32| {
            div()
                .id(id)
                .h_full()
                .w(px(width))
                .flex_none()
                .flex()
                .items_center()
                .pl(px(LIST_CELL_TEXT_X))
                .truncate()
                .text_size(rmac_ui::text_px(LIST_HEADER_TEXT))
                .text_color(chrome_text())
                .child(text)
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
                        LIST_DISCLOSURE_X + LIST_DISCLOSURE_WIDTH + list_icon + LIST_ICON_TO_NAME,
                    ))
                    .when(options.columns[0], |header| {
                        header.child(head(
                            "head-date",
                            "Date Modified",
                            SortKey::Date,
                            Some(DATE_W),
                            true,
                            LIST_CELL_TEXT_X,
                        ))
                    })
                    .when(options.columns[1], |header| {
                        header.child(plain_head("head-created", "Date Created", DATE_W))
                    })
                    .when(options.columns[2], |header| {
                        header.child(plain_head("head-opened", "Date Last Opened", DATE_W))
                    })
                    .when(options.columns[3], |header| {
                        header.child(plain_head("head-added", "Date Added", DATE_W))
                    })
                    .when(options.columns[4], |header| {
                        header.child(head(
                            "head-size",
                            "Size",
                            SortKey::Size,
                            Some(SIZE_W),
                            true,
                            LIST_CELL_TEXT_X,
                        ))
                    })
                    .when(options.columns[5], |header| {
                        header.child(head(
                            "head-kind",
                            "Kind",
                            SortKey::Kind,
                            Some(KIND_W),
                            true,
                            LIST_CELL_TEXT_X,
                        ))
                    })
                    .when(options.columns[6], |header| {
                        header.child(plain_head("head-version", "Version", SIZE_W))
                    })
                    .when(options.columns[7], |header| {
                        header.child(plain_head("head-comments", "Comments", DATE_W))
                    })
                    .when(options.columns[8], |header| {
                        header.child(plain_head("head-tags", "Tags", KIND_W))
                    }),
            )
            .child(div().h(px(0.5)).flex_none().bg(hairline()));

        let visible_list_indices = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                (q.is_empty() || entry.name.to_lowercase().contains(&q)).then_some(index)
            })
            .collect::<Vec<_>>();
        let stripe_index = visible_list_indices.len();
        let row_height = if self
            .entries
            .iter()
            .any(|entry| entry.search_detail.is_some())
        {
            38.0
        } else {
            list_row_height
        };
        let mut group_titles = Vec::with_capacity(visible_list_indices.len());
        let mut offsets = Vec::with_capacity(visible_list_indices.len() + 1);
        offsets.push(0.0_f32);
        let mut previous_group = None;
        for &index in &visible_list_indices {
            let group = view_options::group_title(&self.entries[index], options.group_by);
            let title = if group.is_some() && group != previous_group {
                group.clone()
            } else {
                None
            };
            let header_height = if title.is_some() { 28.0 } else { 0.0 };
            group_titles.push(title);
            offsets.push(offsets.last().copied().unwrap_or(0.0) + row_height + header_height);
            previous_group = group;
        }
        let measured_height = f32::from(self.list_scroll.bounds().size.height);
        let viewport_height = if measured_height > 1.0 {
            measured_height
        } else {
            window_height
        };
        let total_height = *offsets.last().unwrap_or(&0.0);
        let max_scroll = (LIST_ROWS_TOP + total_height - viewport_height).max(0.0);
        let scroll_top = (-f32::from(self.list_scroll.offset().y))
            .max(0.0)
            .min(max_scroll);
        let first_row = offsets
            .partition_point(|offset| *offset <= scroll_top)
            .saturating_sub(1)
            .saturating_sub(8)
            .min(visible_list_indices.len());
        let last_row = offsets
            .partition_point(|offset| *offset < scroll_top + viewport_height + 8.0 * row_height)
            .min(visible_list_indices.len());
        let list_rows = if self.view == ViewMode::List {
            (first_row..last_row)
                .map(|position| {
                    let row = self.render_list_row(
                        visible_list_indices[position],
                        position,
                        visible_list_indices.len(),
                        row_height,
                        window_active,
                        cx,
                    );
                    div()
                        .v_flex()
                        .when_some(group_titles[position].clone(), |container, title| {
                            container.child(
                                div()
                                    .h(px(28.0))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .font_weight(rmac_ui::mac::SEMIBOLD)
                                    .text_color(rmac_ui::mac::text_secondary())
                                    .child(title),
                            )
                        })
                        .child(row)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let filler = div()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .v_flex()
            .children((stripe_index..stripe_index + FILLER_STRIPES).map(|index| {
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
        // Scroll bounds still describe the previous view on the frame that
        // handles ⌘1. Use the current layout width for keyboard grid moves.
        let icon_columns = if show_icons {
            let (cell_width, _) = self.icon_cell();
            ((content_width - ICON_GRID_LEFT) / cell_width)
                .floor()
                .max(1.0) as usize
        } else {
            1
        };

        // Icon-grid tiles. Gallery owns a distinct preview + filmstrip tree.
        let mut tiles: Vec<gpui::AnyElement> = Vec::new();
        if show_icons {
            let icon_size = self.icon_size;
            let (tile_width, tile_height) = self.icon_cell();
            let label_width = ICON_LABEL_MAX_WIDTH.min(tile_width - 16.0);
            let mut position = 0usize;
            let mut previous_icon_group: Option<String> = None;
            for (ix, e) in self.entries.iter().enumerate() {
                if !q.is_empty() && !e.name.to_lowercase().contains(&q) {
                    continue;
                }
                if let Some(group) = view_options::group_title(e, options.group_by) {
                    if previous_icon_group.as_ref() != Some(&group) {
                        tiles.push(
                            div()
                                .w_full()
                                .h(px(28.0))
                                .flex()
                                .items_center()
                                .text_size(rmac_ui::text_px(11.0))
                                .font_weight(rmac_ui::mac::SEMIBOLD)
                                .text_color(rmac_ui::mac::text_secondary())
                                .child(group.clone())
                                .into_any_element(),
                        );
                        previous_icon_group = Some(group);
                    }
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
                    match self
                        .thumbs
                        .get(&e.path)
                        .filter(|_| options.show_icon_preview)
                    {
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
                        .id("rename-field")
                        .role(Role::TextInput)
                        .aria_label("Name")
                        .key_context("FinderRename")
                        .on_action(cx.listener(|this, _: &RenameNextItem, window, cx| {
                            this.rename_next(window, cx)
                        }))
                        .accessible_text_input(input, cx)
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
                        .text_size(rmac_ui::text_px(f32::from(options.text_size)))
                        .line_height(px(15.0))
                        .text_center()
                        // Mac wraps a long name onto two lines and only
                        // middle-ellipsizes what still doesn't fit
                        // ("Screenshot 2026-09-…8.26.03 PM"), never clipping
                        // from the left. `line_clamp` alone just crops
                        // overflow with no ellipsis affix at all, which left
                        // a left-clipped fragment ("9-30 at 8.26.03") on
                        // screen instead.
                        .whitespace_normal()
                        .text_ellipsis_middle()
                        .line_clamp(2)
                        .text_color(if selected {
                            selected_text(window_active)
                        } else {
                            primary_text()
                        })
                        .child(displayed_name(e))
                        .into_any_element(),
                };
                tiles.push(
                    accessible_item(
                        div().id(("tile", ix)),
                        Role::ListBoxOption,
                        e,
                        selected,
                        tile_position,
                        visible_count,
                        &entity,
                        move |this, window, cx| this.accessible_select(ix, window, cx),
                    )
                    .w(px(tile_width))
                    .h(px(tile_height))
                    .flex_none()
                    .flex()
                    .when(!options.label_right, |tile| tile.flex_col())
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
                    .when(options.show_item_info, |tile| {
                        tile.child(
                            div()
                                .text_size(rmac_ui::text_px(10.0))
                                .text_color(rmac_ui::mac::text_secondary())
                                .child(if e.is_dir {
                                    "Folder".to_string()
                                } else {
                                    e.size.to_string()
                                }),
                        )
                    })
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
                        element.on_drag(DraggedPaths(drag_paths.clone()), move |_, _, _, cx| {
                            #[cfg(target_os = "linux")]
                            gpui_linux::stage_external_file_drag(drag_paths.clone());
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
                                .on_drop(cx.listener(
                                    move |this, paths: &DraggedPaths, window, cx| {
                                        this.drop_into(
                                            drop_directory.clone(),
                                            &paths.0,
                                            window.modifiers().alt,
                                            window.modifiers().platform,
                                            cx,
                                        )
                                    },
                                ))
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
                    .track_scroll(&self.list_scroll)
                    .on_scroll_wheel(
                        cx.listener(|_, _: &gpui::ScrollWheelEvent, _, cx| cx.notify()),
                    )
                    .child(
                        div()
                            .min_h(gpui::relative(1.0))
                            .v_flex()
                            .pt(px(LIST_ROWS_TOP))
                            .child(div().h(px(offsets[first_row])).flex_none())
                            .children(list_rows)
                            .child(div().h(px(total_height - offsets[last_row])).flex_none())
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
                        .when(
                            options.background == view_options::Background::Colour,
                            |grid| grid.bg(rmac_ui::mac::accent_subtle()),
                        )
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
                                .when_some(
                                    if options.background == view_options::Background::Picture {
                                        options.picture_path.clone()
                                    } else {
                                        None
                                    },
                                    |grid, picture| grid.child(img(picture).absolute().size_full()),
                                )
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
            .id("finder-content")
            .role(Role::ListBox)
            .track_focus(&self.focus)
            .key_context("Finder")
            .on_action(cx.listener(|this, _: &NewFolder, window, cx| this.new_folder(window, cx)))
            .on_action(cx.listener(|this, _: &NewFolderWithSelection, _, cx| {
                this.new_folder_with_selection(cx)
            }))
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
            .on_action(cx.listener(|this, _: &MakeAlias, _, cx| this.make_alias(cx)))
            .on_action(cx.listener(|this, _: &ShowOriginal, _, cx| this.show_original(cx)))
            .on_action(cx.listener(|this, _: &TagRed, _, cx| this.set_selected_tag("red", cx)))
            .on_action(
                cx.listener(|this, _: &TagOrange, _, cx| this.set_selected_tag("orange", cx)),
            )
            .on_action(
                cx.listener(|this, _: &TagYellow, _, cx| this.set_selected_tag("yellow", cx)),
            )
            .on_action(cx.listener(|this, _: &TagGreen, _, cx| this.set_selected_tag("green", cx)))
            .on_action(cx.listener(|this, _: &TagBlue, _, cx| this.set_selected_tag("blue", cx)))
            .on_action(
                cx.listener(|this, _: &TagPurple, _, cx| this.set_selected_tag("purple", cx)),
            )
            .on_action(cx.listener(|this, _: &TagGray, _, cx| this.set_selected_tag("gray", cx)))
            .on_action(cx.listener(|this, _: &ShareItems, _, cx| {
                this.menu_unavailable("Sharing is not available", cx)
            }))
            .on_action(cx.listener(|this, _: &QuickActions, _, cx| {
                this.menu_unavailable("No Quick Actions are available", cx)
            }))
            .on_action(cx.listener(|this, _: &UseGroups, _, cx| {
                this.change_options(
                    |options| {
                        options.group_by = if options.group_by == view_options::GroupBy::None {
                            view_options::GroupBy::Name
                        } else {
                            view_options::GroupBy::None
                        };
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ImportFromIphone, _, cx| {
                this.menu_unavailable("No iPhone is available to import from", cx)
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &DeselectAll, _, cx| this.deselect_all(cx)))
            .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &GoUp, _, cx| this.go_up(cx)))
            .on_action(
                cx.listener(|this, _: &GoUpInNewWindow, _, cx| this.open_parent_in_new_window(cx)),
            )
            .on_action(cx.listener(|this, _: &GoHome, _, cx| this.go_home(cx)))
            .on_action(cx.listener(|this, _: &GoApplications, _, cx| this.applications_click(cx)))
            .on_action(cx.listener(|this, _: &GoUtilities, _, cx| this.utilities_click(cx)))
            .on_action(cx.listener(|this, _: &GoDownloads, _, cx| this.go_downloads(cx)))
            .on_action(cx.listener(|this, _: &GoDesktop, _, cx| this.go_desktop(cx)))
            .on_action(cx.listener(|this, _: &GoDocuments, _, cx| this.go_documents(cx)))
            .on_action(cx.listener(|this, _: &GoRecents, _, cx| this.recents_click(cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.open_search(window, cx)))
            .on_action(
                cx.listener(|this, _: &FindByName, window, cx| this.open_name_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CopyAsPathname, _, cx| this.copy_as_pathname(cx)))
            .on_action(cx.listener(|this, _: &CopyAsLink, _, cx| this.copy_as_link(cx)))
            .on_action(cx.listener(|this, _: &MoveItemHere, _, cx| this.move_item_here(cx)))
            .on_action(cx.listener(|this, _: &GoTrash, _, cx| this.trash_click(cx)))
            .on_action(cx.listener(|this, _: &OpenItems, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &OpenSelectionInNewTab, _, cx| {
                this.open_selection_in_new_tab(cx)
            }))
            .on_action(cx.listener(|this, _: &OpenSelectionInNewWindow, _, cx| {
                this.open_selection_in_new_window(cx)
            }))
            .on_action(
                cx.listener(|this, _: &OpenSelectionInNewWindowAndClose, window, cx| {
                    this.open_selection_in_new_window_and_close(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &AddToSidebar, _, cx| {
                for path in this.selected_paths() {
                    this.add_sidebar_favourite(path, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &SidebarRemove, _, cx| this.sidebar_remove_context(cx)),
            )
            .on_action(
                cx.listener(|this, _: &SidebarOpenWindow, _, cx| this.sidebar_open_window(cx)),
            )
            .on_action(cx.listener(|this, _: &SidebarOpenTab, _, cx| this.sidebar_open_tab(cx)))
            .on_action(
                cx.listener(|this, _: &SidebarShowEnclosing, _, cx| {
                    this.sidebar_show_enclosing(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SidebarGetInfo, window, cx| {
                    this.sidebar_get_info(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SidebarRename, window, cx| this.sidebar_rename(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SidebarAddToDock, _, cx| this.sidebar_add_to_dock(cx)),
            )
            .on_action(cx.listener(|this, _: &AddToDock, _, cx| {
                this.add_paths_to_dock(this.selected_paths(), cx)
            }))
            .on_action(cx.listener(|this, _: &OpenWith, _, cx| this.request_open_with(cx)))
            .on_action(
                cx.listener(|this, _: &AlwaysOpenWithOther, _, cx| {
                    this.request_always_open_with(cx)
                }),
            )
            .on_action(cx.listener(|this, action: &OpenWithHandlerAction, _, cx| {
                this.open_with_menu_handler(action.index, action.make_default, cx)
            }))
            .on_action(cx.listener(|this, action: &GoToTitlePathAction, _, cx| {
                this.navigate(action.path.clone(), cx)
            }))
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
            .on_action(cx.listener(|_, _: &CloseAll, _, cx| {
                cx.defer(super::settings::close_all_windows);
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.select_adjacent_tab(-1, cx)))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.select_adjacent_tab(1, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &TogglePathBar, _, cx| {
                this.show_path_bar = !this.show_path_bar;
                this.publish_app_menu_state(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleStatusBar, _, cx| {
                this.show_status_bar = !this.show_status_bar;
                this.publish_app_menu_state(cx);
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
            .on_action(cx.listener(|this, _: &EmptyTrashImmediately, _, cx| {
                this.request_empty_trash_immediately(cx)
            }))
            .on_action(cx.listener(|this, _: &ShowSettings, _, cx| this.show_settings(cx)))
            .on_action(cx.listener(|this, _: &ShowHelp, _, cx| {
                this.help_open = true;
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                if this.renaming.is_some() {
                    if ev.keystroke.key.as_str() == "escape" {
                        this.rename_cancel(window, cx);
                    }
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
                        _ => {}
                    }
                    return;
                }
                if this.view == ViewMode::List {
                    match ev.keystroke.key.as_str() {
                        "right" => {
                            this.list_folder_key(true, cx);
                            return;
                        }
                        "left" => {
                            this.list_folder_key(false, cx);
                            return;
                        }
                        _ => {}
                    }
                }
                let current_index = this.selection_lead();
                let current = current_index.and_then(|index| {
                    navigation_indices
                        .iter()
                        .position(|visible| *visible == index)
                });
                let last = navigation_indices.len().saturating_sub(1);
                // Icon view moves by whole rows vertically, as in Finder.
                let vertical_step = icon_columns.max(1);
                let select_position = match ev.keystroke.key.as_str() {
                    "down" if this.view == ViewMode::Gallery => None,
                    "up" if this.view == ViewMode::Gallery => None,
                    "down" => Some(
                        current
                            .map(|position| {
                                let below = position + vertical_step;
                                if below <= last {
                                    below
                                } else {
                                    position
                                }
                            })
                            .unwrap_or(0),
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
                if let Some(position) = select_position {
                    if !navigation_indices.is_empty() {
                        let index = navigation_indices[position];
                        if ev.keystroke.modifiers.shift {
                            this.handle_click(index, false, true);
                        } else {
                            this.select_single(index);
                        }
                        if this.view == ViewMode::List {
                            this.scroll_list_row_into_view(position);
                        }
                        cx.notify();
                    }
                } else if let Some(text) = type_select_text(ev) {
                    this.type_select(&text, &navigation_indices, cx);
                }
            }))
            .when(!self.applications_view && !self.trash_view, |element| {
                element
                    .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rmac_ui::mac::accent_subtle()))
                    .on_drop(cx.listener(|this, ep: &ExternalPaths, window, cx| {
                        this.drop_external(ep.paths().to_vec(), window.modifiers().platform, cx)
                    }))
            })
            .flex_1()
            .v_flex()
            .overflow_hidden()
            .bg(list_bg())
            .when(show_list, |el: Stateful<Div>| el.child(header))
            .child(content)
            .when(self.show_path_bar, |el: Stateful<Div>| {
                el.child(self.render_path_bar(cx))
            })
            .when(self.show_status_bar, |el: Stateful<Div>| {
                el.child(self.render_status_bar())
            })
    }
    fn render_list_row(
        &self,
        ix: usize,
        position: usize,
        visible_count: usize,
        row_height: f32,
        window_active: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let e = &self.entries[ix];
        let options = self.current_options();
        let list_icon = if options.list_large_icons {
            24.0
        } else {
            LIST_ICON
        };
        let entity = cx.entity();
        let depth = if self.trash_view || self.applications_view || self.search_summary.is_some() {
            0
        } else {
            self.list_depths.get(ix).copied().unwrap_or(0)
        };
        let striped = position % 2 == 1;
        let row_path = e.path.clone();
        let accessible_row_path = row_path.clone();
        let left_row_path = row_path.clone();
        let right_row_path = row_path.clone();
        let open_row_path = row_path.clone();
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
                || match self
                    .thumbs
                    .get(&e.path)
                    .filter(|_| options.show_icon_preview)
                {
                    Some(thumbnail) => div()
                        .size(px(list_icon))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            img(thumbnail.clone())
                                .max_w(px(list_icon))
                                .max_h(px(list_icon - 4.0))
                                .rounded(px(2.0)),
                        )
                        .into_any_element(),
                    None => item_artwork(e.is_dir, &e.name, list_icon),
                },
                |path| {
                    img(path)
                        .w(px(list_icon))
                        .h(px(list_icon))
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

        let rename_click_path = e.path.clone();
        let name_cell: gpui::AnyElement = match &self.renaming {
            Some((rename_path, input)) if rename_path == &e.path => {
                div()
                    .id("rename-field")
                    .role(Role::TextInput)
                    .aria_label("Name")
                    .key_context("FinderRename")
                    .on_action(cx.listener(|this, _: &RenameNextItem, window, cx| {
                        this.rename_next(window, cx)
                    }))
                    .accessible_text_input(input, cx)
                    .pl(px(LIST_ICON_TO_NAME))
                    .flex_1()
                    .child(TextField::new(input).appearance(true))
                    .into_any_element()
            }
            _ => div()
                .id(SharedString::from(format!(
                    "list-name-{}",
                    e.path.display()
                )))
                .pl(px(LIST_ICON_TO_NAME))
                .flex_1()
                .min_w(px(0.0))
                .v_flex()
                .justify_center()
                .child(
                    div()
                        .truncate()
                        .text_color(primary)
                        .child(displayed_name(e)),
                )
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
                        this.rename_click_generation = this.rename_click_generation.wrapping_add(1);
                        if selected
                            && !this.trash_view
                            && !this.applications_view
                            && !ev.modifiers.platform
                            && !ev.modifiers.shift
                            && ev.click_count == 1
                        {
                            cx.stop_propagation();
                            let generation = this.rename_click_generation;
                            let path = rename_click_path.clone();
                            let cwd = this.cwd.clone();
                            let window_handle = window.window_handle();
                            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                                // Defer rename past the platform's double-click interval.
                                // A second press cancels it and lets the row open normally.
                                cx.background_executor()
                                    .timer(std::time::Duration::from_millis(450))
                                    .await;
                                let _ = cx.update_window(window_handle, |_, window, cx| {
                                    let _ = this.update(cx, |this: &mut FinderView, cx| {
                                        if this.rename_click_generation == generation
                                            && this.cwd == cwd
                                            && this.renaming.is_none()
                                            && this
                                                .selected_entry()
                                                .is_some_and(|entry| entry.path == path)
                                        {
                                            this.rename_start(window, cx);
                                        }
                                    });
                                });
                            })
                            .detach();
                        }
                    }),
                )
                .on_click(cx.listener(move |_, ev: &ClickEvent, _, cx| {
                    if selected && ev.click_count() == 1 {
                        cx.stop_propagation();
                    }
                }))
                .into_any_element(),
        };

        accessible_item(
            div().id(SharedString::from(format!("row-{}", e.path.display()))),
            Role::ListBoxOption,
            e,
            selected,
            position,
            visible_count,
            &entity,
            move |this, window, cx| {
                if let Some(index) = this.list_row_index(&accessible_row_path) {
                    this.accessible_select(index, window, cx);
                }
            },
        )
        .flex_none()
        .flex()
        .items_center()
        .h(px(row_height))
        .mx(px(LIST_ROW_INSET))
        .rounded(px(ROW_RADIUS))
        .text_size(rmac_ui::text_px(f32::from(options.list_text_size)))
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
                        .w(px(LIST_DISCLOSURE_X
                            + LIST_DISCLOSURE_WIDTH
                            + depth as f32 * 16.0))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .when(
                            e.is_dir
                                && !self.trash_view
                                && !self.applications_view
                                && self.search_summary.is_none(),
                            |el: Div| {
                                let path = e.path.clone();
                                let accessible_path = path.clone();
                                let accessible_entity = entity.clone();
                                el.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "disclosure-{}",
                                            e.path.display()
                                        )))
                                        .role(Role::Button)
                                        .aria_label(format!(
                                            "{} {}",
                                            if self.expanded.contains(&e.path) {
                                                "Collapse"
                                            } else {
                                                "Expand"
                                            },
                                            e.name
                                        ))
                                        .aria_expanded(self.expanded.contains(&e.path))
                                        .w(px(LIST_DISCLOSURE_WIDTH))
                                        .h(px(row_height))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .child(icon(
                                            if self.expanded.contains(&e.path) {
                                                "icons/chevron-down.svg"
                                            } else {
                                                "icons/chevron-right.svg"
                                            },
                                            12.0,
                                            if selected {
                                                selected_text(window_active)
                                            } else {
                                                chrome_text()
                                            },
                                        ))
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click(cx.listener(
                                            move |this, event: &ClickEvent, _, cx| {
                                                cx.stop_propagation();
                                                if event.modifiers().alt {
                                                    this.toggle_list_folder_tree(path.clone(), cx);
                                                } else {
                                                    this.toggle_list_folder(path.clone(), cx);
                                                }
                                            },
                                        ))
                                        .on_a11y_action(
                                            AccessibleAction::Click,
                                            move |_, _, cx| {
                                                accessible_entity.update(cx, |this, cx| {
                                                    this.toggle_list_folder(
                                                        accessible_path.clone(),
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                )
                            },
                        ),
                )
                .child(row_icon)
                .child(name_cell),
        )
        .when(options.columns[0], |row| {
            row.child(
                div()
                    .w(px(DATE_W))
                    .flex_none()
                    .pl(px(LIST_CELL_TEXT_X))
                    .truncate()
                    .text_color(sub)
                    .child(if options.relative_dates {
                        e.modified.clone()
                    } else {
                        e.modified_absolute.clone()
                    }),
            )
        })
        .when(options.columns[1], |row| {
            row.child(
                div()
                    .w(px(DATE_W))
                    .flex_none()
                    .pl(px(LIST_CELL_TEXT_X))
                    .truncate()
                    .text_color(sub)
                    .child(if options.relative_dates {
                        e.created.clone()
                    } else {
                        e.created_absolute.clone()
                    }),
            )
        })
        .when(options.columns[2], |row| {
            row.child(
                div()
                    .w(px(DATE_W))
                    .flex_none()
                    .pl(px(LIST_CELL_TEXT_X))
                    .truncate()
                    .text_color(sub)
                    .child(if options.relative_dates {
                        e.last_opened.clone()
                    } else {
                        e.last_opened_absolute.clone()
                    }),
            )
        })
        .when(options.columns[3], |row| {
            row.child(
                div()
                    .w(px(DATE_W))
                    .flex_none()
                    .pl(px(LIST_CELL_TEXT_X))
                    .truncate()
                    .text_color(sub)
                    .child(if options.relative_dates {
                        e.added.clone()
                    } else {
                        e.added_absolute.clone()
                    }),
            )
        })
        .when(options.columns[4], |row| {
            row.child(
                div()
                    .w(px(SIZE_W))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .pr(px(LIST_SIZE_TRAILING))
                    .text_color(sub)
                    .child(if options.calculate_sizes && e.is_dir {
                        self.directory_sizes
                            .get(&e.path)
                            .map(|size| human_size(*size).into())
                            .unwrap_or_else(|| "--".into())
                    } else {
                        e.size.clone()
                    }),
            )
        })
        .when(options.columns[5], |row| {
            row.child(
                div()
                    .w(px(KIND_W))
                    .flex_none()
                    .pl(px(LIST_CELL_TEXT_X))
                    .text_color(sub)
                    .truncate()
                    .child(e.kind.clone()),
            )
        })
        .when(options.columns[6], |row| {
            row.child(
                div()
                    .w(px(SIZE_W))
                    .flex_none()
                    .pl(px(LIST_CELL_TEXT_X))
                    .text_color(sub)
                    .child("--"),
            )
        })
        .when(options.columns[7], |row| {
            row.child(
                div()
                    .w(px(DATE_W))
                    .flex_none()
                    .pl(px(LIST_CELL_TEXT_X))
                    .text_color(sub)
                    .child("--"),
            )
        })
        .when(options.columns[8], |row| {
            row.child(
                div()
                    .w(px(KIND_W))
                    .flex_none()
                    .pl(px(LIST_CELL_TEXT_X))
                    .text_color(sub)
                    .child("--"),
            )
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                this.rename_click_generation = this.rename_click_generation.wrapping_add(1);
                cx.stop_propagation();
                if ev.modifiers.control {
                    if let Some(index) = this.list_row_index(&left_row_path) {
                        this.open_context_menu(Some(index), ev.position, window, cx);
                    }
                    return;
                }
                if let Some(index) = this.list_row_index(&left_row_path) {
                    this.handle_click(index, ev.modifiers.platform, ev.modifiers.shift);
                }
                window.focus(&this.focus, cx);
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if let Some(index) = this.list_row_index(&right_row_path) {
                    this.open_context_menu(Some(index), ev.position, window, cx);
                }
            }),
        )
        .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
            if ev.click_count() >= 2 {
                if let Some(index) = this.list_row_index(&open_row_path) {
                    this.open_index(index, cx);
                }
            }
            window.focus(&this.focus, cx);
        }))
        .when(
            !self.trash_view && !self.applications_view,
            |el: Stateful<Div>| {
                el.on_drag(DraggedPaths(drag_paths.clone()), move |_, _, _, cx| {
                    #[cfg(target_os = "linux")]
                    gpui_linux::stage_external_file_drag(drag_paths.clone());
                    cx.new(|_| DragPreview { count: drag_count })
                })
            },
        )
        .when(
            row_is_dir && !self.trash_view && !self.applications_view,
            |el: Stateful<Div>| {
                let dd = drop_dir.clone();
                el.drag_over::<DraggedPaths>(|s, _, _, _| s.bg(rmac_ui::mac::accent_subtle()))
                    .on_drag_move(cx.listener(
                        move |this, event: &gpui::DragMoveEvent<DraggedPaths>, _, cx| {
                            let inside = event.bounds.contains(&event.event.position);
                            this.spring_hover(spring_dir.clone(), inside, cx);
                        },
                    ))
                    .on_drop(cx.listener(move |this, p: &DraggedPaths, window, cx| {
                        this.drop_into(
                            dd.clone(),
                            &p.0,
                            window.modifiers().alt,
                            window.modifiers().platform,
                            cx,
                        )
                    }))
            },
        )
    }
}
