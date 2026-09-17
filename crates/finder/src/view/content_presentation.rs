use super::*;

impl FinderView {
    pub(super) fn render_columns(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.applications_view {
            return self.render_application_columns(cx).into_any_element();
        }
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
                    "icons/folder-artwork.svg"
                } else {
                    "icons/file-fill.svg"
                };
                let icol = if is_sel {
                    white()
                } else if is_dir {
                    folder_blue()
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
                        .rounded(px(mac::radius_menu_item()))
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
        row.into_any_element()
    }

    fn render_application_columns(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut applications = div()
            .id("applications-column")
            .w(px(300.0))
            .h_full()
            .flex_none()
            .overflow_y_scroll()
            .v_flex()
            .py_1()
            .border_r_1()
            .border_color(sep());

        for (index, entry) in self.entries.iter().enumerate() {
            let selected = self.selected.contains(&index);
            let icon_element = entry
                .application
                .as_ref()
                .and_then(|application| application.icon.clone())
                .map_or_else(
                    || icon("icons/layout-grid.svg", 22.0, secondary()).into_any_element(),
                    |path| {
                        img(path)
                            .w(px(24.0))
                            .h(px(24.0))
                            .rounded(px(mac::radius_menu_item()))
                            .into_any_element()
                    },
                );
            applications = applications.child(
                div()
                    .id(("application-column-row", index))
                    .h(px(32.0))
                    .mx_1()
                    .px_2()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(mac::radius_menu_item()))
                    .when(selected, |element: Stateful<Div>| element.bg(sel()))
                    .when(!selected, |element: Stateful<Div>| {
                        element.hover(|hover| hover.bg(rmac_ui::mac::hover()))
                    })
                    .child(icon_element)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(if selected { white() } else { label() })
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
                            window.focus(&this.focus);
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
                                .rounded(px(mac::radius_popover()))
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
                    window.focus(&this.focus);
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
            .relative()
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
            .when(self.view == ViewMode::Icon, |bar| {
                bar.child(
                    div()
                        .absolute()
                        .right(px(12.0))
                        .w(px(104.0))
                        .h_full()
                        .flex()
                        .items_center()
                        .child(Slider::new(&self.icon_size_slider).w_full()),
                )
            })
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
        self.start_transfer_with_conflicts("Moving", tasks, false, cx);
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
        self.start_transfer_with_conflicts("Copying", tasks, false, cx);
    }
}
