use super::*;

fn get_info_entry(selected_entry: Option<&Entry>, current_directory: &Path) -> Option<Entry> {
    selected_entry
        .cloned()
        .or_else(|| entry_for(current_directory))
}

fn get_info_entries(
    selected_paths: &[PathBuf],
    current_directory: &Path,
    applications_view: bool,
) -> Vec<Entry> {
    if selected_paths.is_empty() {
        return if applications_view {
            Vec::new()
        } else {
            get_info_entry(None, current_directory)
                .into_iter()
                .collect()
        };
    }
    selected_paths
        .iter()
        .filter_map(|path| entry_for(path))
        .collect()
}

impl FinderView {
    pub(super) fn get_info(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.trash_view {
            self.operation_error =
                Some("Restore an item before viewing its file information".into());
            cx.notify();
            return;
        }
        let selected_paths = self.selected_paths();
        let entries = get_info_entries(&selected_paths, &self.cwd, self.applications_view);
        let owner = cx.entity().downgrade();
        for entry in entries {
            let thumbnail = self.thumbs.get(&entry.path).cloned();
            let title = format!("{} Info", entry.name);
            let (width, height) = rmac_ui::outer_window_size(INFO_WIDTH, INFO_MAX_HEIGHT);
            let mut options = rmac_ui::window_options_for_app_with_title(
                rmac_ui::app_id::FILES,
                title,
                width,
                height,
                cx,
            );
            options.window_bounds = Some(gpui::WindowBounds::centered(
                gpui::size(px(width), px(height)),
                cx,
            ));
            options.window_min_size = Some(gpui::size(px(width), px(height)));
            let info_owner = owner.clone();
            let opened = cx.open_window(options, move |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                let view = cx.new(|cx| InfoWindow::new(entry, thumbnail, info_owner, window, cx));
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
            });
            match opened {
                Ok(handle) => self.info_windows.push(handle),
                Err(_) => {
                    self.operation_error = Some("Files could not open the Info window".into());
                }
            }
        }
        self.menu_at = None;
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
                    v.sort_by_key(|(_, when)| std::cmp::Reverse(*when));
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
}

struct InfoWindow {
    entry: Entry,
    details: Vec<(&'static str, String)>,
    thumbnail: Option<PathBuf>,
    name_input: Option<gpui::Entity<InputState>>,
    owner: gpui::WeakEntity<FinderView>,
    focus: FocusHandle,
}

impl InfoWindow {
    fn new(
        entry: Entry,
        thumbnail: Option<PathBuf>,
        owner: gpui::WeakEntity<FinderView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = format!("{} Info", entry.name);
        window.set_window_title(&title);
        let name_input = entry.application.is_none().then(|| {
            let input =
                cx.new(|cx| InputState::new(window, cx).default_value(entry.name.to_string()));
            cx.subscribe_in(
                &input,
                window,
                |this, input, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.commit_name(input.clone(), window, cx);
                    }
                },
            )
            .detach();
            input
        });
        let focus = cx.focus_handle();
        Self {
            details: file_info(&entry),
            entry,
            thumbnail,
            name_input,
            owner,
            focus,
        }
    }

    fn commit_name(
        &mut self,
        input: gpui::Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.entry.path.clone();
        let new_name = input.read(cx).value().to_string();
        let renamed = self.owner.update(cx, |owner, cx| {
            let destination = owner.rename_path_to(&path, &new_name, cx);
            if destination.is_some() {
                owner.reload(cx);
            }
            destination
        });
        match renamed {
            Ok(Some(destination)) => {
                if let Some(entry) = entry_for(&destination) {
                    self.entry = entry;
                    self.details = file_info(&self.entry);
                    let name = self.entry.name.clone();
                    let title = format!("{} Info", name);
                    window.set_window_title(&title);
                    input.update(cx, |state, cx| state.set_value(name, window, cx));
                    cx.notify();
                }
            }
            Err(_) => window.remove_window(),
            Ok(None) => {}
        }
    }

    /// Finder's information panel, hosted in its own resizable app window.
    fn render_info(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let e = &self.entry;
        let details = &self.details;
        let value_of = |key: &str| {
            details
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        };
        let rows = |keys: &[&'static str], label_width: f32| {
            keys.iter()
                .filter_map(|key| value_of(key).map(|value| (*key, value)))
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
                                .w(px(label_width))
                                .whitespace_nowrap()
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

        let header_artwork = match &self.thumbnail {
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
                    .on_click(|_, window, _| window.remove_window()),
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

        let general = block().child(section("General:")).children(rows(
            &["Kind", "Size", "Where", "Created", "Modified"],
            INFO_LABEL_RIGHT - INFO_SECTION_INSET,
        ));

        // Finder's Name & Extension field: edit and press Return to rename.
        let name_field = self.name_input.as_ref().map(|input| {
            block()
                .id("info-name")
                .role(Role::Group)
                .aria_label("Name & Extension")
                .child(section("Name & Extension:"))
                .child(TextField::new(input).small())
        });

        let preview = self.thumbnail.as_ref().map(|thumbnail| {
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
            .children(rows(
                &["Owner", "Group", "Permissions"],
                INFO_PERMISSIONS_LABEL,
            ));

        let card = div()
            .id("info-panel")
            .role(Role::Group)
            .aria_label(format!("{} Info", e.name))
            .track_focus(&self.focus)
            .w(px(INFO_WIDTH))
            .h_full()
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

        card.on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key.as_str() == "escape" {
                cx.stop_propagation();
                window.remove_window();
            }
        }))
    }
}

impl Focusable for InfoWindow {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for InfoWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_info(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_info_without_a_selection_uses_the_current_directory() {
        let current_directory = std::env::current_dir().unwrap();

        let entry = get_info_entry(None, &current_directory).unwrap();

        assert_eq!(entry.path, current_directory);
        assert!(entry.is_dir);
    }

    #[test]
    fn get_info_keeps_the_selected_entry_when_one_exists() {
        let selected_path = std::env::current_exe().unwrap();
        let selected = entry_for(&selected_path).unwrap();
        let current_directory = std::env::current_dir().unwrap();

        let entry = get_info_entry(Some(&selected), &current_directory).unwrap();

        assert_eq!(entry.path, selected_path);
    }

    #[test]
    fn get_info_opens_each_selected_item_and_uses_the_current_folder_in_background() {
        let folder = std::env::current_dir().unwrap();
        let file = std::env::current_exe().unwrap();

        let selected = get_info_entries(std::slice::from_ref(&file), &folder, false);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].path, file);

        let multiple = get_info_entries(&[file.clone(), folder.clone()], &folder, false);
        assert_eq!(multiple.len(), 2);
        assert_eq!(multiple[0].path, file);
        assert_eq!(multiple[1].path, folder);

        let background = get_info_entries(&[], &folder, false);
        assert_eq!(background.len(), 1);
        assert_eq!(background[0].path, folder);

        assert!(get_info_entries(&[], &folder, true).is_empty());
    }

    #[test]
    fn get_info_can_describe_the_filesystem_root() {
        let root = Path::new(std::path::MAIN_SEPARATOR_STR);

        let entry = get_info_entry(None, root).unwrap();

        assert_eq!(entry.path.as_path(), root);
        assert_eq!(entry.name.as_ref(), root.display().to_string());
        assert!(entry.is_dir);
    }
}
