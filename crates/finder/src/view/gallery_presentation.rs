use super::*;

impl FinderView {
    /// Gallery inspector (design-lab/finder.html): the item's name and
    /// "kind – size" centred at the top, then an "Information" table whose
    /// values sit right-aligned in semibold, rows split by hairlines.
    fn render_gallery_inspector(&self, active_index: Option<usize>) -> impl IntoElement {
        let contents = active_index
            .and_then(|index| self.entries.get(index))
            .map(|entry| {
                let information = [
                    ("Kind", entry.kind.clone()),
                    ("Size", entry.size.clone()),
                    ("Modified", entry.modified.clone()),
                ];
                let summary = if entry.size.as_ref() == "--" {
                    entry.kind.clone()
                } else {
                    SharedString::from(format!("{} – {}", entry.kind, entry.size))
                };
                div()
                    .v_flex()
                    .child(
                        div()
                            .pt(px(GALLERY_INSPECTOR_TITLE_TOP))
                            .pb(px(GALLERY_INSPECTOR_SECTION_GAP))
                            .v_flex()
                            .items_center()
                            .child(
                                div()
                                    .max_w(gpui::relative(1.0))
                                    .truncate()
                                    .text_size(rmac_ui::text_px(15.0))
                                    .font_weight(rmac_ui::mac::SEMIBOLD)
                                    .text_color(label())
                                    .child(entry.name.clone()),
                            )
                            .child(
                                div()
                                    .max_w(gpui::relative(1.0))
                                    .truncate()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .text_color(secondary_text())
                                    .child(summary),
                            ),
                    )
                    .child(
                        div()
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
            })
            .unwrap_or_else(|| {
                div()
                    .pt(px(GALLERY_INSPECTOR_TITLE_TOP))
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
            .pl(px(GALLERY_INSPECTOR_INSET))
            .pr(px(GALLERY_INSPECTOR_TRAILING))
            .border_l_1()
            .border_color(dark_rule())
            .child(contents)
    }

    /// Like Finder, Gallery view always previews something: opening it on a
    /// folder with nothing selected selects the first visible item.
    pub(super) fn select_first_for_gallery(&mut self, cx: &gpui::App) {
        if self.view != ViewMode::Gallery || !self.selected.is_empty() {
            return;
        }
        let query = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };
        if let Some(first) = self
            .entries
            .iter()
            .position(|entry| query.is_empty() || entry.name.to_lowercase().contains(&query))
        {
            self.select_single(first);
        }
    }

    /// macOS-style Gallery view: one large selected-item preview above a
    /// horizontally scrollable filmstrip. It consumes only the same bounded
    /// thumbnail cache and file metadata already admitted by Icon/List views.
    pub(super) fn render_gallery(
        &self,
        visible_indices: &[usize],
        window_height: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The preview fills about 70 % of the stage above the filmstrip, as
        // Finder's does (design-lab/finder.html).
        let stage_height = window_height
            - TOOLBAR_HEIGHT
            - STATUS_BAR_HEIGHT
            - if self.show_path_bar {
                PATH_BAR_HEIGHT
            } else {
                0.0
            }
            - (GALLERY_THUMB + 16.0);
        let preview_size = (stage_height * 0.7).clamp(96.0, 512.0);
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
                    let visual = entry
                        .application
                        .as_ref()
                        .and_then(|application| application.icon.clone())
                        .map(|path| {
                            img(path)
                                .w(px(preview_size))
                                .h(px(preview_size))
                                .rounded(px(preview_size * 0.22))
                                .into_any_element()
                        })
                        .unwrap_or_else(|| {
                            self.thumbs.get(&entry.path).map_or_else(
                                || item_artwork(entry.is_dir, &entry.name, preview_size),
                                |thumbnail| {
                                    img(thumbnail.clone())
                                        .max_w(px(preview_size * 1.6))
                                        .max_h(px(preview_size))
                                        .rounded(px(rmac_ui::mac::radius_control()))
                                        .into_any_element()
                                },
                            )
                        });
                    // Finder names the item in the inspector, not under the
                    // preview; only a multiple selection is spelled out here.
                    let multiple = (selected_count > 1).then(|| {
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(secondary_text())
                            .child(format!("{selected_count} items selected"))
                    });

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
                        .child(visual)
                        .children(multiple)
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
                            || item_artwork(entry.is_dir, &entry.name, GALLERY_THUMB),
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
                        entry,
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
