use super::*;
use gpui_component::scroll::ScrollableElement as _;

impl FinderView {
    fn render_place(&self, p: &Place, cx: &Context<Self>) -> impl IntoElement {
        let is_tag = p.kind == PlaceKind::Tag;
        let selected = match p.kind {
            PlaceKind::Trash => self.trash_view,
            PlaceKind::Applications => self.applications_view,
            _ => !is_tag && !self.trash_view && !self.applications_view && self.cwd == p.path,
        };
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
        let main = div()
            .id(SharedString::from(format!("placemain-{key}")))
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .min_w(px(0.0))
            .cursor_pointer()
            .child(leading)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(rmac_ui::mac::REGULAR)
                    .text_color(sidebar_text())
                    .child(p.name.clone()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.activate_place(kind, tag_name.clone(), np.clone(), cx)
            }));

        // The row is the sidebar item assistive technology sees: its name,
        // whether it is the current location, and a Click that goes there
        // without needing the row to be on screen.
        let mut row = div()
            .id(SharedString::from(format!("place-{key}")))
            .role(Role::ListItem)
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
            .child(main);

        // Dropping onto a folder place moves the items there, as in Finder.
        if matches!(p.kind, PlaceKind::Item | PlaceKind::Volume) {
            let destination = p.path.clone();
            row = row
                .drag_over::<DraggedPaths>(|style, _, _, _| style.bg(sidebar_selection()))
                .on_drop(cx.listener(move |this, paths: &DraggedPaths, _, cx| {
                    this.drop_into(destination.clone(), &paths.0, cx)
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
        match kind {
            PlaceKind::Tag => self.tag_click(name, cx),
            PlaceKind::Recents => self.recents_click(cx),
            PlaceKind::Trash => self.trash_click(cx),
            PlaceKind::Applications => self.applications_click(cx),
            _ => self.navigate(path, cx),
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
        self.trash_view = false;
        self.applications_view = true;
        self.cancel_search();
        self.result_title = Some("Applications".into());
        self.operation_error = None;
        self.search_summary = Some("Loading applications…".into());
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
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut entries = suppress_replaced_applications(rmac_apps::discover()?)
                        .into_iter()
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
            if section.places.is_empty() {
                continue;
            }
            if !section.title.is_empty() {
                contents = contents.child(
                    div()
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
                        .child(section.title.clone()),
                );
            }
            for p in &section.places {
                contents = contents.child(self.render_place(p, cx));
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
                cx.listener(|this, _, _, _| this.dragging = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.dragging {
                    this.dragging = false;
                    window.start_window_move();
                }
            }))
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
                            .role(Role::List)
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
