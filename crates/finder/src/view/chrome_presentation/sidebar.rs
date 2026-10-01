use super::*;
use gpui_component::scroll::ScrollableElement as _;

fn place_is_selected(
    kind: PlaceKind,
    name: &str,
    path: &Path,
    cwd: &Path,
    trash_view: bool,
    applications_view: bool,
    result_title: Option<&str>,
) -> bool {
    match kind {
        PlaceKind::Trash => trash_view,
        PlaceKind::Applications => applications_view && result_title == Some("Applications"),
        PlaceKind::Recents => !trash_view && !applications_view && result_title == Some("Recents"),
        PlaceKind::Tag => {
            !trash_view
                && !applications_view
                && result_title.and_then(|title| title.strip_prefix("Tag: ")) == Some(name)
        }
        _ => !trash_view && !applications_view && result_title.is_none() && cwd == path,
    }
}

impl FinderView {
    fn render_place(
        &self,
        p: &Place,
        favourite_slot: Option<usize>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let is_favourite = favourite_slot.is_some();
        let is_tag = p.kind == PlaceKind::Tag;
        let selected = place_is_selected(
            p.kind,
            p.name.as_ref(),
            &p.path,
            &self.cwd,
            self.trash_view,
            self.applications_view,
            self.result_title.as_ref().map(|title| title.as_ref()),
        );
        let key = format!("{}-{}", p.name, p.path.display());

        // design-lab/finder.html: glyph box centred 19 in, label at 35; a tag
        // dot is centred on the same axis.
        let leading: gpui::AnyElement = if is_tag {
            div()
                .ml(px(SIDEBAR_GLYPH_CENTRE - SIDEBAR_TAG_DOT / 2.0))
                .mr(px(SIDEBAR_TEXT_X
                    - SIDEBAR_GLYPH_CENTRE
                    - SIDEBAR_TAG_DOT / 2.0))
                .w(px(SIDEBAR_TAG_DOT))
                .h(px(SIDEBAR_TAG_DOT))
                .flex_none()
                .rounded_full()
                .bg(p.tint)
                .into_any_element()
        } else {
            div()
                .ml(px(SIDEBAR_GLYPH_CENTRE - SIDEBAR_GLYPH / 2.0))
                .mr(px(SIDEBAR_TEXT_X
                    - SIDEBAR_GLYPH_CENTRE
                    - SIDEBAR_GLYPH / 2.0))
                .flex_none()
                .child(icon(p.icon, SIDEBAR_GLYPH, sidebar_text()))
                .into_any_element()
        };

        let np = p.path.clone();
        let tag_name = p.name.clone();
        let kind = p.kind;
        let a11y_path = p.path.clone();
        let a11y_name = p.name.clone();
        let entity = cx.entity();
        let removable = self.is_removable_favourite(&p.path);
        let label: gpui::AnyElement = match &self.renaming {
            Some((path, input)) if path == &p.path => div()
                .id("sidebar-rename-field")
                .role(Role::TextInput)
                .aria_label("Name")
                .accessible_text_input(input, cx)
                .flex_1()
                .min_w(px(0.0))
                .child(TextField::new(input).appearance(true))
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(rmac_ui::text_px(13.0))
                .font_weight(rmac_ui::mac::REGULAR)
                .text_color(sidebar_text())
                .child(p.name.clone())
                .into_any_element(),
        };
        let main = div()
            .id(SharedString::from(format!("placemain-{key}")))
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .min_w(px(0.0))
            .cursor_pointer()
            .child(leading)
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_place(kind, tag_name.clone(), np.clone(), cx)
            }));

        // The row is the sidebar item assistive technology sees: its name,
        // whether it is the current location, and a Click that goes there
        // without needing the row to be on screen.
        let mut row = div()
            .id(SharedString::from(format!("place-{key}")))
            .role(Role::ListBoxOption)
            .aria_label(p.name.clone())
            .aria_selected(selected)
            .on_a11y_action(AccessibleAction::Click, move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.activate_place(kind, a11y_name.clone(), a11y_path.clone(), cx)
                });
            })
            .flex_none()
            .flex()
            .items_center()
            .h(px(SIDEBAR_ROW_HEIGHT))
            .pr_1()
            .rounded(px(SIDEBAR_ROW_RADIUS))
            // Tahoe: a neutral grey fill, never the accent, and no hover wash.
            .when(selected, |el: Stateful<Div>| el.bg(sidebar_selection()))
            .when(removable && !p.path.exists(), |el: Stateful<Div>| {
                el.opacity(0.5)
            })
            .child(main);

        if matches!(
            kind,
            PlaceKind::Item | PlaceKind::Volume | PlaceKind::Applications
        ) {
            let context_path = p.path.clone();
            row = row.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_sidebar_context_menu(
                        context_path.clone(),
                        is_favourite,
                        event.position,
                        window,
                        cx,
                    );
                }),
            );
        }

        // Dropping onto a folder place moves the items there, as in Finder.
        if let Some(before) = favourite_slot {
            let after = before + 1;
            let favourite_count = self
                .sections
                .iter()
                .find(|section| section.title.as_ref() == self.file_words.favourites())
                .map_or(0, |section| section.places.len());
            row = row
                .when(
                    self.sidebar_drop_index == Some(before),
                    |el: Stateful<Div>| el.border_t_2().border_color(accent()),
                )
                .when(
                    self.sidebar_drop_index == Some(after) && after == favourite_count,
                    |el: Stateful<Div>| el.border_b_2().border_color(accent()),
                )
                .drag_over::<DraggedPaths>(|style, _, _, _| style)
                .on_drag_move(cx.listener(
                    move |this, event: &gpui::DragMoveEvent<DraggedPaths>, _, cx| {
                        let lower = event.event.position.y > event.bounds.center().y;
                        this.sidebar_drop_index = Some(if lower { after } else { before });
                        cx.notify();
                    },
                ))
                .on_drop(cx.listener(move |this, paths: &DraggedPaths, _, cx| {
                    let index = this.sidebar_drop_index.take().unwrap_or(before);
                    for path in paths.0.iter().cloned().rev() {
                        this.insert_sidebar_favourite(path, index, cx);
                    }
                }))
                .drag_over::<DraggedSidebarItem>(|style, _, _, _| style)
                .on_drag_move(cx.listener(
                    move |this, event: &gpui::DragMoveEvent<DraggedSidebarItem>, _, cx| {
                        let lower = event.event.position.y > event.bounds.center().y;
                        this.sidebar_drop_index = Some(if lower { after } else { before });
                        cx.notify();
                    },
                ))
                .on_drop(cx.listener(move |this, item: &DraggedSidebarItem, _, cx| {
                    cx.stop_propagation();
                    let index = this.sidebar_drop_index.take().unwrap_or(before);
                    this.insert_sidebar_favourite(item.0.clone(), index, cx);
                }))
                .drag_over::<ExternalPaths>(|style, _, _, _| style)
                .on_drag_move(cx.listener(
                    move |this, event: &gpui::DragMoveEvent<ExternalPaths>, _, cx| {
                        let lower = event.event.position.y > event.bounds.center().y;
                        this.sidebar_drop_index = Some(if lower { after } else { before });
                        cx.notify();
                    },
                ))
                .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                    let index = this.sidebar_drop_index.take().unwrap_or(before);
                    for path in paths.paths().iter().cloned().rev() {
                        this.insert_sidebar_favourite(path, index, cx);
                    }
                }));
        } else if matches!(p.kind, PlaceKind::Item | PlaceKind::Volume) {
            let destination = p.path.clone();
            row = row
                .drag_over::<DraggedPaths>(|style, _, _, _| style.bg(sidebar_selection()))
                .on_drop(cx.listener(move |this, paths: &DraggedPaths, window, cx| {
                    this.drop_into(
                        destination.clone(),
                        &paths.0,
                        window.modifiers().alt,
                        window.modifiers().platform,
                        cx,
                    )
                }));
        }

        if p.kind == PlaceKind::Volume {
            let ep = p.path.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("eject-{key}")))
                    .role(Role::Button)
                    .aria_label(format!("Eject {}", p.name))
                    .flex_none()
                    .w(px(20.0))
                    .h(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(icon("icons/eject.svg", 14.0, sidebar_section_text()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.eject_volume(ep.clone(), cx);
                    })),
            );
        }
        if is_favourite {
            let dragged = p.path.clone();
            row = row.on_drag(DraggedSidebarItem(dragged), move |_, _, _, cx| {
                cx.new(|_| DragPreview { count: 1 })
            });
        }
        if removable {
            let removed = p.path.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("remove-favourite-{key}")))
                    .role(Role::Button)
                    .aria_label(format!("Remove {} from Sidebar", p.name))
                    .flex_none()
                    .w(px(20.0))
                    .h(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(sidebar_section_text())
                    .child("×")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.remove_sidebar_favourite(&removed, cx);
                    })),
            );
        }
        row
    }

    /// Go to a sidebar place, as a click on its row does.
    fn activate_place(
        &mut self,
        kind: PlaceKind,
        name: SharedString,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.is_removable_favourite(&path) && !path.exists() {
            self.missing_favourite = Some(path);
            cx.notify();
            return;
        }
        match kind {
            PlaceKind::Tag => self.tag_click(name, cx),
            PlaceKind::Recents => self.recents_click(cx),
            PlaceKind::Trash => self.trash_click(cx),
            PlaceKind::Applications => self.applications_click(cx),
            _ if path.is_file() => self.open_paths(vec![path], cx),
            _ => self.navigate(path, cx),
        }
    }

    fn open_sidebar_context_menu(
        &mut self,
        path: PathBuf,
        is_favourite: bool,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_context_path = Some(path);
        self.sidebar_context_is_favourite = is_favourite;
        self.menu_purpose = MenuPurpose::Sidebar;
        self.menu_at = Some(rmac_ui::ContextMenuState::open(
            position,
            &self.focus,
            window,
            cx,
        ));
        cx.notify();
    }

    pub(in crate::view) fn sidebar_remove_context(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.sidebar_context_path.clone() {
            if self.sidebar_context_is_favourite {
                self.remove_sidebar_item(&path, cx);
            }
        }
    }

    pub(in crate::view) fn remove_sidebar_item(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.sidebar_drop_index = None;
        if self.is_removable_favourite(path) {
            self.remove_sidebar_favourite(path, cx);
        } else {
            self.set_builtin_sidebar_visibility(path, false, cx);
        }
    }

    pub(in crate::view) fn sidebar_open_window(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.sidebar_context_path.as_ref() else {
            return;
        };
        if path.is_file() {
            self.open_paths(vec![path.clone()], cx);
        } else if path.is_dir() {
            let path = path.display().to_string();
            if !rmac_ui::open_another_window(vec!["--path".to_owned(), path], cx) {
                self.operation_error = Some("Files could not open another window".into());
                cx.notify();
            }
        }
    }

    pub(in crate::view) fn sidebar_open_tab(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.sidebar_context_path.clone() else {
            return;
        };
        if path.is_dir() {
            self.new_tab(cx);
            self.navigate(path, cx);
        } else if path.is_file() {
            self.open_paths(vec![path], cx);
        }
    }

    pub(in crate::view) fn sidebar_show_enclosing(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.sidebar_context_path.clone() else {
            return;
        };
        if let Some(parent) = path.parent() {
            self.pending_select = Some(path.clone());
            self.navigate(parent.to_path_buf(), cx);
        }
    }

    pub(in crate::view) fn sidebar_get_info(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.sidebar_context_path.clone() {
            self.get_info_for_paths(&[path], window, cx);
        }
    }

    pub(in crate::view) fn sidebar_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.sidebar_context_path.clone() {
            self.rename_sidebar_path(path, window, cx);
        }
    }

    pub(in crate::view) fn sidebar_add_to_dock(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self
            .sidebar_context_path
            .clone()
            .filter(|path| !path.as_os_str().is_empty())
        {
            self.add_paths_to_dock(vec![path], cx);
        }
    }

    pub(in crate::view) fn trash_click(&mut self, cx: &mut Context<Self>) {
        self.applications_view = false;
        self.trash_view = true;
        if self.view == ViewMode::Column {
            self.view = ViewMode::List;
        }
        self.result_title = Some(self.file_words.bin().into());
        self.operation_error = None;
        self.reload_trash(cx);
    }

    pub(in crate::view) fn applications_click(&mut self, cx: &mut Context<Self>) {
        self.application_catalog_click(false, cx);
    }

    pub(in crate::view) fn utilities_click(&mut self, cx: &mut Context<Self>) {
        self.application_catalog_click(true, cx);
    }

    fn application_catalog_click(&mut self, utilities: bool, cx: &mut Context<Self>) {
        self.trash_view = false;
        self.applications_view = true;
        self.cancel_search();
        self.result_title = Some(
            if utilities {
                "Utilities"
            } else {
                "Applications"
            }
            .into(),
        );
        self.operation_error = None;
        self.search_summary = Some(
            if utilities {
                "Loading utilities…"
            } else {
                "Loading applications…"
            }
            .into(),
        );
        self.search_relevance_order = false;
        self.entries.clear();
        self.selected.clear();
        self.anchor = None;
        self.renaming = None;
        self.menu_at = None;
        self.directory_generation = self.directory_generation.wrapping_add(1);
        let generation = self.directory_generation;
        let key = self.sort_key;
        let asc = self.sort_asc;
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `rmac_apps::discover()` can shell out to `gsettings` to read
            // the active icon theme; GPUI's background executor is not
            // safe to spawn child processes from (LINUX-HW-07).
            let result = blocking::unblock(move || {
                let mut entries = suppress_replaced_applications(rmac_apps::discover()?)
                    .into_iter()
                    .filter(|application| {
                        !utilities
                            || application
                                .categories
                                .iter()
                                .any(|category| category.eq_ignore_ascii_case("Utility"))
                    })
                    .map(entry_for_application)
                    .collect::<Vec<_>>();
                sort_entries(&mut entries, key, asc);
                Ok::<_, std::io::Error>(entries)
            })
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if !this.applications_view || this.directory_generation != generation {
                    return;
                }
                match result {
                    Ok(entries) => {
                        this.search_summary = None;
                        this.entries = entries;
                        this.operation_error = None;
                    }
                    Err(error) => {
                        this.search_summary = None;
                        this.operation_error =
                            Some(format!("Could not load applications: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::view) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut contents = div()
            .h_full()
            .v_flex()
            .px(px(SIDEBAR_ROW_INSET))
            .pb(px(SIDEBAR_ROW_INSET));
        for section in &self.sections {
            if section.places.is_empty() && section.title.as_ref() != self.file_words.favourites() {
                continue;
            }
            if !section.title.is_empty() {
                let is_favourites = section.title.as_ref() == self.file_words.favourites();
                let mut header = div()
                    .id(SharedString::from(format!("section-{}", section.title)))
                    .role(Role::Heading)
                    .aria_label(section.title.clone())
                    .aria_level(2)
                    .flex_none()
                    .mt(px(SIDEBAR_SECTION_GAP))
                    .h(px(SIDEBAR_SECTION_HEIGHT))
                    .pl(px(SIDEBAR_SECTION_TEXT_X))
                    .pt(px(2.0))
                    .flex()
                    .items_center()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(sidebar_section_text())
                    .child(section.title.clone());
                // Dragging a folder onto Favourites adds it, as it does in
                // Finder — the row area itself keeps its own drop (move the
                // items there), so the header is the add target.
                if is_favourites {
                    header = header
                        .when(self.sidebar_drop_index == Some(0), |el: Stateful<Div>| {
                            el.border_b_2().border_color(accent())
                        })
                        .drag_over::<DraggedPaths>(|style, _, _, _| style.bg(sidebar_selection()))
                        .on_drop(cx.listener(|this, paths: &DraggedPaths, _, cx| {
                            this.sidebar_drop_index = None;
                            for path in paths.0.iter().cloned().rev() {
                                this.insert_sidebar_favourite(path, 0, cx);
                            }
                        }))
                        .drag_over::<DraggedSidebarItem>(|style, _, _, _| {
                            style.bg(sidebar_selection())
                        })
                        .on_drop(cx.listener(|this, item: &DraggedSidebarItem, _, cx| {
                            cx.stop_propagation();
                            this.sidebar_drop_index = None;
                            this.insert_sidebar_favourite(item.0.clone(), 0, cx);
                        }))
                        .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(sidebar_selection()))
                        .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                            this.sidebar_drop_index = None;
                            for path in paths.paths().iter().cloned().rev() {
                                this.insert_sidebar_favourite(path, 0, cx);
                            }
                        }));
                }
                contents = contents.child(header);
            }
            for (index, p) in section.places.iter().enumerate() {
                let is_favourite = section.title.as_ref() == self.file_words.favourites();
                contents = contents.child(self.render_place(p, is_favourite.then_some(index), cx));
            }
        }

        // Traffic lights live inside the floating panel (window-relative
        // centres 26 / 49 / 72, y 26), so place the cluster by its centre.
        let traffic_left = TRAFFIC_LIGHT_FIRST_CENTRE
            - rmac_ui::mac::traffic_light_hit_width() / 2.0
            - SIDEBAR_INSET;
        let traffic_top = TRAFFIC_LIGHT_FIRST_CENTRE
            - rmac_ui::mac::traffic_light_hit_height() / 2.0
            - SIDEBAR_INSET;
        let titlebar = div()
            .id("sidebar-titlebar")
            .h(px(TOOLBAR_HEIGHT - SIDEBAR_INSET))
            .flex_none()
            .relative()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.dragging = Some(event.position)
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = None),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, _| {
                if event.pressed_button == Some(MouseButton::Left)
                    && this.dragging.is_some_and(|press| {
                        let delta = event.position - press;
                        delta.x.abs() > px(4.0) || delta.y.abs() > px(4.0)
                    })
                {
                    this.dragging = None;
                    window.start_window_move();
                }
            }))
            .on_click(|event, _, cx| {
                if event.click_count() == 2 {
                    rmac_ui::double_click_title_bar_action(cx);
                }
            })
            .child(
                div()
                    .absolute()
                    .left(px(traffic_left))
                    .top(px(traffic_top))
                    .child(rmac_ui::traffic_lights()),
            );

        div()
            .w(px(self.sidebar_width))
            .h_full()
            .flex_shrink_0()
            .relative()
            .child(
                div()
                    .id("sidebar-panel")
                    .absolute()
                    .left(px(SIDEBAR_INSET))
                    .top(px(SIDEBAR_INSET))
                    .bottom(px(SIDEBAR_BOTTOM_INSET))
                    .right_0()
                    .v_flex()
                    .rounded(px(SIDEBAR_RADIUS))
                    .bg(sidebar_panel())
                    .border_1()
                    .border_color(sidebar_panel_edge())
                    .overflow_hidden()
                    .child(titlebar)
                    .child(
                        div()
                            .id("sidebar-places")
                            .role(Role::ListBox)
                            .aria_label("Sidebar")
                            .flex_1()
                            .min_h(px(0.0))
                            .child(contents.overflow_y_scrollbar()),
                    ),
            )
            .child(
                div()
                    .id("sidebar-resizer")
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .w(px(5.0))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.begin_sidebar_resize()),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            this.resize_sidebar(f32::from(event.position.x), cx);
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.finish_sidebar_resize()),
                    ),
            )
    }
}

#[cfg(test)]
mod selection_tests {
    use super::{place_is_selected, PlaceKind};
    use std::path::Path;

    #[test]
    fn sidebar_selection_tracks_recents_and_the_exact_tag_view() {
        let cwd = Path::new("/home/user");
        let empty = Path::new("");
        let selected =
            |kind, name, title| place_is_selected(kind, name, empty, cwd, false, false, title);

        assert!(selected(PlaceKind::Recents, "Recents", Some("Recents")));
        assert!(!selected(PlaceKind::Recents, "Recents", Some("Tag: Blue")));
        assert!(selected(PlaceKind::Tag, "Blue", Some("Tag: Blue")));
        assert!(!selected(PlaceKind::Tag, "Blue", Some("Tag: Red")));
        assert!(!selected(PlaceKind::Tag, "Blue", Some("Tag: Blue extra")));
    }

    #[test]
    fn ordinary_places_keep_path_selection_and_special_views_suppress_it() {
        let home = Path::new("/home/user");
        assert!(!place_is_selected(
            PlaceKind::Item,
            "Home",
            home,
            home,
            false,
            false,
            Some("Recents"),
        ));
        assert!(place_is_selected(
            PlaceKind::Item,
            "Home",
            home,
            home,
            false,
            false,
            None,
        ));
        assert!(!place_is_selected(
            PlaceKind::Item,
            "Home",
            home,
            home,
            false,
            true,
            None,
        ));
        assert!(place_is_selected(
            PlaceKind::Applications,
            "Applications",
            Path::new(""),
            home,
            false,
            true,
            Some("Applications"),
        ));
        assert!(!place_is_selected(
            PlaceKind::Applications,
            "Applications",
            Path::new(""),
            home,
            false,
            true,
            Some("Utilities"),
        ));
    }
}
