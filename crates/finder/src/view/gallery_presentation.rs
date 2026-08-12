use super::*;

impl FinderView {
    /// macOS-style Gallery view: one large selected-item preview above a
    /// horizontally scrollable filmstrip. It consumes only the same bounded
    /// thumbnail cache and file metadata already admitted by Icon/List views.
    pub(super) fn render_gallery(
        &self,
        visible_indices: &[usize],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_index = self
            .anchor
            .filter(|index| self.selected.contains(index) && visible_indices.contains(index))
            .or_else(|| {
                self.selected
                    .iter()
                    .copied()
                    .find(|index| visible_indices.contains(index))
            });
        let selected_count = self
            .selected
            .iter()
            .filter(|index| visible_indices.contains(index))
            .count();

        let stage = active_index
            .and_then(|index| self.entries.get(index).map(|entry| (index, entry)))
            .map_or_else(
                || {
                    div()
                        .flex_1()
                        .min_h(px(0.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            rmac_ui::EmptyState::new(if visible_indices.is_empty() {
                                "No Matching Items"
                            } else {
                                "Select an Item"
                            })
                            .message(if visible_indices.is_empty() {
                                "Try a different search."
                            } else {
                                "Choose an item in the filmstrip to preview it."
                            }),
                        )
                        .into_any_element()
                },
                |(index, entry)| {
                    let glyph = if entry.is_dir {
                        "icons/folder-artwork.svg"
                    } else {
                        "icons/file-fill.svg"
                    };
                    let visual = entry
                        .application
                        .as_ref()
                        .and_then(|application| application.icon.clone())
                        .map(|path| {
                            img(path)
                                .max_w(px(132.0))
                                .max_h(px(132.0))
                                .rounded(px(29.0))
                                .into_any_element()
                        })
                        .unwrap_or_else(|| {
                            self.thumbs.get(&entry.path).map_or_else(
                                || {
                                    icon(
                                        glyph,
                                        132.0,
                                        if entry.is_dir {
                                            folder_blue()
                                        } else {
                                            secondary()
                                        },
                                    )
                                    .into_any_element()
                                },
                                |thumbnail| {
                                    img(thumbnail.clone())
                                        .max_w(px(440.0))
                                        .max_h(px(280.0))
                                        .rounded(px(8.0))
                                        .into_any_element()
                                },
                            )
                        });
                    let name = if selected_count > 1 {
                        SharedString::from(format!("{} items selected", selected_count))
                    } else {
                        entry.name.clone()
                    };
                    let detail = if selected_count > 1 {
                        SharedString::from("The focused item is shown")
                    } else {
                        SharedString::from(format!(
                            "{}  •  {}  •  {}",
                            entry.kind, entry.size, entry.modified
                        ))
                    };

                    div()
                        .id(("gallery-preview", index))
                        .flex_1()
                        .min_h(px(0.0))
                        .v_flex()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .px_6()
                        .py_4()
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
                                .max_w(px(560.0))
                                .text_size(rmac_ui::text_px(15.0))
                                .font_weight(rmac_ui::mac::SEMIBOLD)
                                .text_color(label())
                                .truncate()
                                .child(name),
                        )
                        .child(
                            div()
                                .max_w(px(560.0))
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(secondary())
                                .truncate()
                                .child(detail),
                        )
                        .into_any_element()
                },
            );

        let filmstrip = visible_indices.iter().filter_map(|&index| {
            let entry = self.entries.get(index)?;
            let selected = self.selected.contains(&index);
            let glyph = if entry.is_dir {
                "icons/folder-artwork.svg"
            } else {
                "icons/file-fill.svg"
            };
            let visual = entry
                .application
                .as_ref()
                .and_then(|application| application.icon.clone())
                .map(|path| {
                    img(path)
                        .max_w(px(48.0))
                        .max_h(px(48.0))
                        .rounded(px(10.0))
                        .into_any_element()
                })
                .unwrap_or_else(|| {
                    self.thumbs.get(&entry.path).map_or_else(
                        || {
                            icon(
                                glyph,
                                42.0,
                                if entry.is_dir {
                                    folder_blue()
                                } else {
                                    secondary()
                                },
                            )
                            .into_any_element()
                        },
                        |thumbnail| {
                            img(thumbnail.clone())
                                .max_w(px(58.0))
                                .max_h(px(48.0))
                                .rounded(px(4.0))
                                .into_any_element()
                        },
                    )
                });
            let drag_paths = if selected {
                self.selected_paths()
            } else {
                vec![entry.path.clone()]
            };
            let drag_count = drag_paths.len();
            let drop_directory = entry.path.clone();
            let is_directory = entry.is_dir;

            Some(
                div()
                    .id(("gallery-item", index))
                    .w(px(82.0))
                    .h(px(82.0))
                    .flex_none()
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .px_1()
                    .rounded(px(8.0))
                    .border_2()
                    .border_color(if selected { accent() } else { list_bg() })
                    .when(!selected, |element: Stateful<Div>| {
                        element.hover(|hover| hover.bg(rmac_ui::mac::hover()))
                    })
                    .child(
                        div()
                            .h(px(50.0))
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(visual),
                    )
                    .child(
                        div()
                            .max_w(px(72.0))
                            .text_size(rmac_ui::text_px(10.0))
                            .text_color(label())
                            .truncate()
                            .child(entry.name.clone()),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if !this.selected.contains(&index) {
                                this.select_single(index);
                            }
                            window.focus(&this.focus);
                            this.menu_at = Some(rmac_ui::ContextMenuState::open(
                                event.position,
                                &this.focus,
                                window,
                                cx,
                            ));
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.modifiers().control {
                            cx.stop_propagation();
                            this.open_context_menu(Some(index), event.position(), window, cx);
                            return;
                        }
                        if event.click_count() >= 2 {
                            this.open_index(index, cx);
                            return;
                        }
                        let modifiers = event.modifiers();
                        this.handle_click(index, modifiers.platform, modifiers.shift);
                        window.focus(&this.focus);
                        cx.notify();
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
                                .on_drop(cx.listener(move |this, paths: &DraggedPaths, _, cx| {
                                    this.drop_into(drop_directory.clone(), &paths.0, cx)
                                }))
                        },
                    )
                    .into_any_element(),
            )
        });

        div()
            .id("gallery")
            .flex_1()
            .min_h(px(0.0))
            .v_flex()
            .bg(list_bg())
            .child(stage)
            .child(
                div()
                    .id("gallery-filmstrip")
                    .h(px(104.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .overflow_x_scroll()
                    .border_t_1()
                    .border_color(sep())
                    .bg(toolbar_bg())
                    .children(filmstrip),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_context_menu(None, event.position, window, cx);
                }),
            )
    }
}
