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
        self.menu_at = Some(rmac_ui::ContextMenuState::open(
            position,
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    pub(super) fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Recursive content matches may not contain the query in their names.
        // Local filtering remains active until Return starts a ranked search.
        let q = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };

        let head = |w: Option<f32>, text: &'static str, key: SortKey, pl: bool| {
            let active = !self.search_relevance_order && self.sort_key == key;
            let title: SharedString = if active {
                format!("{text} {}", if self.sort_asc { "↑" } else { "↓" }).into()
            } else {
                text.into()
            };
            Button::new(text, title)
                .ghost()
                .xsmall()
                .selected(active)
                .when(pl, |button| button.pl_4())
                .when_some(w, |el, w| el.w(px(w)))
                .when(w.is_none(), |el| el.flex_1())
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
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
                "icons/folder-artwork.svg"
            } else {
                "icons/file-fill.svg"
            };
            let icon_color = if selected {
                white()
            } else if e.is_dir {
                folder_blue()
            } else {
                secondary()
            };
            let row_icon: gpui::AnyElement = e
                .application
                .as_ref()
                .and_then(|application| application.icon.clone())
                .map_or_else(
                    || icon(glyph, 16.0, icon_color).into_any_element(),
                    |path| {
                        img(path)
                            .w(px(17.0))
                            .h(px(17.0))
                            .rounded(px(4.0))
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
                            .child(row_icon)
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
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if ev.modifiers.control {
                                this.open_context_menu(Some(ix), ev.position, window, cx);
                                return;
                            }
                            this.handle_click(ix, ev.modifiers.platform, ev.modifiers.shift);
                            window.focus(&this.focus);
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
                        window.focus(&this.focus);
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

        // Icon-grid tiles. Gallery owns a distinct preview + filmstrip tree.
        let mut tiles: Vec<gpui::AnyElement> = Vec::new();
        if show_icons {
            let icon_size = self.icon_size;
            let tile_width = icon_size + 52.0;
            let label_width = icon_size + 32.0;
            for (ix, e) in self.entries.iter().enumerate() {
                if !q.is_empty() && !e.name.to_lowercase().contains(&q) {
                    continue;
                }
                let selected = self.selected.contains(&ix);
                let glyph = if e.is_dir {
                    "icons/folder-artwork.svg"
                } else {
                    "icons/file-fill.svg"
                };
                let icon_color = if e.is_dir { folder_blue() } else { secondary() };
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
                            .rounded(px(3.0))
                            .into_any_element(),
                        None => icon(glyph, icon_size, icon_color).into_any_element(),
                    }
                };
                let drag_paths = if selected {
                    self.selected_paths()
                } else {
                    vec![e.path.clone()]
                };
                let drag_count = drag_paths.len();
                let drop_directory = e.path.clone();
                let is_directory = e.is_dir;
                tiles.push(
                    div()
                        .id(("tile", ix))
                        .w(px(tile_width))
                        .h(px(icon_size + 48.0))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .px_1()
                        .py_2()
                        .child(
                            div()
                                .w(px(icon_size))
                                .h(px(icon_size))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(visual),
                        )
                        .child(
                            div()
                                .max_w(px(label_width))
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(4.0))
                                .when(selected, |el: Div| el.bg(sel()))
                                .text_size(rmac_ui::text_px(13.0))
                                .text_center()
                                .truncate()
                                .text_color(if selected { white() } else { label() })
                                .child(e.name.clone()),
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
                                window.focus(&this.focus);
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
                            window.focus(&this.focus);
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
                                    .on_drop(cx.listener(
                                        move |this, paths: &DraggedPaths, _, cx| {
                                            this.drop_into(drop_directory.clone(), &paths.0, cx)
                                        },
                                    ))
                            },
                        )
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
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.selected.clear();
                            this.anchor = None;
                            window.focus(&this.focus);
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
                ViewMode::Column => self.render_columns(cx).into_any_element(),
                ViewMode::Gallery => self.render_gallery(&visible_indices, cx).into_any_element(),
                ViewMode::Icon => div()
                    .id("icon-grid")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .p_3()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_wrap()
                            .content_start()
                            .gap_2()
                            .children(tiles),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.selected.clear();
                            this.anchor = None;
                            window.focus(&this.focus);
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
            }
        };

        div()
            .track_focus(&self.focus)
            .key_context("Finder")
            .on_action(cx.listener(|this, _: &NewFolder, window, cx| this.new_folder(window, cx)))
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
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                let a = this.active;
                this.close_tab(a, cx);
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                let current = this.anchor.and_then(|anchor| {
                    navigation_indices.iter().position(|index| *index == anchor)
                });
                let select_position = match ev.keystroke.key.as_str() {
                    "down" => Some(
                        current
                            .map(|position| position + 1)
                            .unwrap_or(0)
                            .min(navigation_indices.len().saturating_sub(1)),
                    ),
                    "right" if horizontal_navigation => Some(
                        current
                            .map(|position| position + 1)
                            .unwrap_or(0)
                            .min(navigation_indices.len().saturating_sub(1)),
                    ),
                    "up" => Some(
                        current
                            .map(|position| position.saturating_sub(1))
                            .unwrap_or(0),
                    ),
                    "left" if horizontal_navigation => Some(
                        current
                            .map(|position| position.saturating_sub(1))
                            .unwrap_or(0),
                    ),
                    "home" => Some(0),
                    "end" => Some(navigation_indices.len().saturating_sub(1)),
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
            .child(self.render_status_bar())
    }
}
