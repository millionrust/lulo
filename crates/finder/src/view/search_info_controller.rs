use super::*;

impl FinderView {
    pub(super) fn get_info(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error =
                Some("Restore an item before viewing its file information".into());
            cx.notify();
            return;
        }
        self.info = self.selected_entry().cloned();
        self.info_details = self.info.as_ref().map(file_info).unwrap_or_default();
        self.info_name = None;
        if let Some(entry) = self.info.clone() {
            if entry.application.is_none() {
                self.info_name_field(&entry, window, cx);
            }
        }
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

    /// Get Info, laid out as Finder's info window (design-lab/finder.html):
    /// a 265 pt panel with a title strip, the icon / name / size header,
    /// then disclosure-style sections whose labels right-align at 67 pt.
    pub(super) fn render_info(&self, e: &Entry, cx: &mut Context<Self>) -> impl IntoElement {
        let details = &self.info_details;
        let value_of = |key: &str| {
            details
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        };
        let rows = |keys: &[&'static str]| {
            keys.iter()
                .filter_map(|key| value_of(*key).map(|value| (*key, value)))
                .map(|(key, value)| {
                    div()
                        .flex()
                        .items_start()
                        .gap(px(INFO_LABEL_GAP))
                        .py(px((INFO_ROW_PITCH - INFO_ROW_LINE) / 2.0))
                        .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
                        .line_height(px(INFO_ROW_LINE))
                        .text_color(label())
                        .child(
                            div()
                                .w(px(INFO_LABEL_RIGHT - INFO_SECTION_INSET))
                                .flex_none()
                                .text_right()
                                .child(format!("{key}:")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .whitespace_normal()
                                .child(value),
                        )
                })
                .collect::<Vec<_>>()
        };
        let section = |title: &'static str| {
            div()
                .h(px(INFO_SECTION_HEADER))
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_size(rmac_ui::text_px(INFO_SECTION_TEXT))
                .text_color(label())
                .child(icon("icons/chevron-down.svg", 10.0, secondary_text()))
                .child(title)
        };
        let block = || {
            div()
                .v_flex()
                .px(px(INFO_SECTION_INSET))
                .pb(px(8.0))
                .border_t_1()
                .border_color(header_divider())
        };

        let header_artwork = match self.thumbs.get(&e.path) {
            Some(thumbnail) => div()
                .size(px(INFO_HEADER_ICON))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(thumbnail.clone())
                        .max_w(px(INFO_HEADER_ICON))
                        .max_h(px(INFO_HEADER_ICON)),
                )
                .into_any_element(),
            None => item_artwork(e.is_dir, &e.name, INFO_HEADER_ICON),
        };
        let title_strip = div()
            .h(px(INFO_TITLE_HEIGHT))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("info-close")
                    .role(Role::Button)
                    .aria_label("Close")
                    .absolute()
                    .left(px(INFO_CLOSE_CENTRE - INFO_CLOSE / 2.0))
                    .top(px((INFO_TITLE_HEIGHT - INFO_CLOSE) / 2.0))
                    .size(px(INFO_CLOSE))
                    .rounded_full()
                    .bg(gpui::rgb(0xff5f57))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.info = None;
                        this.info_details.clear();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .max_w(px(INFO_WIDTH - 2.0 * INFO_TITLE_TEXT_INSET))
                    .truncate()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary_text())
                    .child(format!("{} Info", e.name)),
            );
        let header = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(INFO_SECTION_INSET))
            .pb(px(10.0))
            .child(header_artwork)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .v_flex()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(rmac_ui::mac::BOLD)
                            .text_color(label())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .child(e.name.clone()),
                            )
                            .when(!e.is_dir, |line| {
                                line.child(div().flex_none().child(e.size.clone()))
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(rmac_ui::text_px(INFO_ROW_TEXT))
                            .text_color(secondary_text())
                            .child(format!("Modified: {}", e.modified)),
                    ),
            );

        let general = block()
            .child(section("General:"))
            .children(rows(&["Kind", "Size", "Where", "Created", "Modified"]));

        // Finder's Name & Extension field: edit and press Return to rename.
        let name_field = self
            .info_name
            .as_ref()
            .filter(|(path, _)| *path == e.path)
            .map(|(_, input)| {
                block()
                    .id("info-name")
                    .role(Role::Group)
                    .aria_label("Name & Extension")
                    .child(section("Name & Extension:"))
                    .child(TextField::new(input).small())
            });

        let preview = self.thumbs.get(&e.path).map(|thumbnail| {
            block().child(section("Preview:")).child(
                div()
                    .h(px(INFO_PREVIEW_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        img(thumbnail.clone())
                            .max_w(gpui::relative(1.0))
                            .max_h(px(INFO_PREVIEW_HEIGHT))
                            .rounded(px(rmac_ui::mac::radius_control())),
                    ),
            )
        });

        let permissions = block()
            .child(section("Sharing & Permissions:"))
            .children(rows(&["Owner", "Group", "Permissions"]));

        let card = div()
            .id("info-panel")
            .role(Role::Dialog)
            .aria_label(format!("{} Info", e.name))
            .w(px(INFO_WIDTH))
            .max_h(px(INFO_MAX_HEIGHT))
            .v_flex()
            .overflow_hidden()
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(sep())
            .shadow_lg()
            .child(title_strip)
            .child(
                div()
                    .id("info-body")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .v_flex()
                    .child(header)
                    .child(general)
                    .children(name_field)
                    .children(preview)
                    .child(permissions),
            );

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rmac_ui::mac::scrim())
            .child(card)
    }
}
