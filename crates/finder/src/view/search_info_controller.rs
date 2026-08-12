use super::*;
use gpui_component::scroll::ScrollableElement as _;

impl FinderView {
    pub(super) fn get_info(&mut self, cx: &mut Context<Self>) {
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
        if self.applications_view {
            self.operation_notice =
                Some("Applications are filtered as you type in the search field".into());
            cx.notify();
            return;
        }
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

    pub(super) fn tag_click(&mut self, name: SharedString, cx: &mut Context<Self>) {
        self.trash_view = false;
        self.applications_view = false;
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
    pub(super) fn recents_click(&mut self, cx: &mut Context<Self>) {
        self.trash_view = false;
        self.applications_view = false;
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

    pub(super) fn render_info(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.entries.get(ix) else {
            return div();
        };
        let glyph = if e.is_dir {
            "icons/folder-artwork.svg"
        } else {
            "icons/file-fill.svg"
        };
        let glyph_color = if e.is_dir { folder_blue() } else { secondary() };

        let mut card = div()
            .w(px(300.0))
            .max_h(px(500.0))
            .overflow_hidden()
            .rounded(px(12.0))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(sep())
            .shadow_lg()
            .child(
                // header bar with close
                div().h(px(28.0)).flex().items_center().px_2().child(
                    Button::new("info-close", "Close")
                        .ghost()
                        .xsmall()
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
                            .max_w(px(260.0))
                            .text_size(rmac_ui::text_px(15.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .text_center()
                            .child(e.name.clone()),
                    ),
            );

        let mut details = div().v_flex().min_h(px(0.0)).overflow_y_scrollbar();
        for (k, v) in file_info(e) {
            details = details.child(
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
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_color(label())
                            .child(v),
                    ),
            );
        }
        card = card.child(details);

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
