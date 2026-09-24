use super::*;

impl FinderView {
    fn render_gallery_inspector(&self, active_index: Option<usize>) -> impl IntoElement {
        let contents = active_index
            .and_then(|index| self.entries.get(index))
            .map(|entry| {
                let glyph = if entry.is_dir {
                    "icons/folder-artwork.svg"
                } else {
                    "icons/file-fill.svg"
                };
                let artwork = entry
                    .application
                    .as_ref()
                    .and_then(|application| application.icon.clone())
                    .map_or_else(
                        || {
                            icon(
                                glyph,
                                40.0,
                                if entry.is_dir {
                                    folder_blue()
                                } else {
                                    secondary()
                                },
                            )
                            .into_any_element()
                        },
                        |path| {
                            img(path)
                                .w(px(40.0))
                                .h(px(40.0))
                                .rounded(px(rmac_ui::mac::radius_menu()))
                                .into_any_element()
                        },
                    );
                let information = [
                    ("Kind", entry.kind.clone()),
                    ("Size", entry.size.clone()),
                    ("Modified", entry.modified.clone()),
                ];
                div()
                    .v_flex()
                    .gap_4()
                    .child(
                        div().flex().items_center().gap_3().child(artwork).child(
                            div()
                                .min_w(px(0.0))
                                .v_flex()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rmac_ui::text_px(13.0))
                                        .font_weight(rmac_ui::mac::SEMIBOLD)
                                        .text_color(label())
                                        .child(entry.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(rmac_ui::text_px(11.0))
                                        .text_color(secondary())
                                        .child(entry.kind.clone()),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .child(
                                div()
                                    .h(px(rmac_ui::mac::list_row_height()))
                                    .flex()
                                    .items_center()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .font_weight(rmac_ui::mac::SEMIBOLD)
                                    .text_color(label())
                                    .child("Information"),
                            )
                            .children(information.into_iter().map(|(name, value)| {
                                div()
                                    .h(px(rmac_ui::mac::list_row_height()))
                                    .flex()
                                    .items_center()
                                    .border_t_1()
                                    .border_color(sep())
                                    .text_size(rmac_ui::text_px(11.0))
                                    .child(
                                        div()
                                            .w(px(68.0))
                                            .flex_none()
                                            .text_color(tertiary())
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .min_w(px(0.0))
                                            .flex_1()
                                            .truncate()
                                            .text_color(label())
                                            .child(value),
                                    )
                            })),
                    )
                    .into_any_element()
            })
            .unwrap_or_else(|| {
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary())
                    .child("Select an item to see its information.")
                    .into_any_element()
            });

        div()
            .id("gallery-inspector")
            .w(px(GALLERY_INSPECTOR_WIDTH))
            .h_full()
            .flex_none()
            .p_3()
            .border_l_1()
            .border_color(dark_rule())
            .child(contents)
    }

    /// macOS-style Gallery view: one large selected-item preview above a
    /// horizontally scrollable filmstrip. It consumes only the same bounded
    /// thumbnail cache and file metadata already admitted by Icon/List views.
    pub(super) fn render_gallery(
        &self,
        visible_indices: &[usize],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let search_active =
            self.search_summary.is_some() || !self.query.read(cx).value().trim().is_empty();
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
                            rmac_ui::EmptyState::new(
                                if visible_indices.is_empty() && search_active {
                                    "No Matching Items"
                                } else if visible_indices.is_empty() {
                                    "This Folder Is Empty"
                                } else {
                                    "Select an Item"
                                },
                            )
                            .message(
                                if visible_indices.is_empty() && search_active {
                                    "Try a different search."
                                } else if visible_indices.is_empty() {
                                    "Items added to this folder will appear here."
                                } else {
                                    "Choose an item in the filmstrip to preview it."
                                },
                            ),
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
                                .w(px(132.0))
                                .h(px(132.0))
                                .rounded(px(rmac_ui::mac::radius_pill()))
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
                                        .rounded(px(rmac_ui::mac::radius_control()))
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

        let entity = cx.entity();
        let visible_count = visible_indices.len();
        let filmstrip = visible_indices
            .iter()
            .enumerate()
            .filter_map(|(position, &index)| {
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
                            .w(px(48.0))
                            .h(px(48.0))
                            .rounded(px(rmac_ui::mac::radius_menu()))
                            .into_any_element()
                    })
                    .unwrap_or_else(|| {
                        self.thumbs.get(&entry.path).map_or_else(
                            || {
                                icon(
                                    glyph,
                                    GALLERY_THUMB,
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
                                    .rounded(px(rmac_ui::mac::radius_menu_item()))
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
                    accessible_item(
                        div().id(("gallery-item", index)),
                        Role::ListBoxOption,
                        entry.name.clone(),
                        selected,
                        position,
                        visible_count,
                        &entity,
                        move |this, window, cx| this.accessible_select(index, window, cx),
                    )
                    .w(px(GALLERY_THUMB + 6.0))
                    .h(px(GALLERY_THUMB + 6.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ICON_PLATE_RADIUS))
                    .when(selected, |element: Stateful<Div>| element.bg(icon_plate()))
                    .child(
                        div()
                            .h(px(GALLERY_THUMB))
                            .w(px(GALLERY_THUMB))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(visual),
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
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            this.open_index(index, cx);
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
                                .on_drop(cx.listener(move |this, paths: &DraggedPaths, _, cx| {
                                    this.drop_into(drop_directory.clone(), &paths.0, cx)
                                }))
                        },
                    )
                    .into_any_element(),
                )
            });

        let browser = div()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .v_flex()
            .child(stage)
            .child(
                div()
                    .id("gallery-filmstrip")
                    .role(Role::ListBox)
                    .aria_label(self.title())
                    .h(px(GALLERY_THUMB + 16.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(GALLERY_THUMB_PITCH - GALLERY_THUMB - 6.0))
                    .px(px(5.0))
                    .overflow_x_scroll()
                    .children(filmstrip),
            );

        div()
            .id("gallery")
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .bg(list_bg())
            .child(browser)
            .child(self.render_gallery_inspector(active_index))
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
}
