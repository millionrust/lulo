use super::*;

impl FinderView {
    pub(super) fn render_columns(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

    pub(super) fn render_path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                Button::new(SharedString::from(format!("crumb-{i}")), name)
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |this, _, _, cx| this.navigate(path.clone(), cx))),
            );
            if i + 1 < n {
                bar = bar.child(icon("icons/chevron-right.svg", 9.0, tertiary()));
            }
        }
        bar
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
