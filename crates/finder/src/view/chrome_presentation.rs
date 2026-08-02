use super::*;

impl FinderView {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let nav = |id: &'static str, glyph: &'static str, enabled: bool| {
            div()
                .id(id)
                .w(px(26.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .when(enabled, |el: Stateful<Div>| {
                    el.hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
                })
                .child(icon(
                    glyph,
                    17.0,
                    if enabled { label() } else { tertiary() },
                ))
        };
        let cur = self.view;
        let seg = |id: &'static str, glyph: &'static str, mode: ViewMode| {
            let active = cur == mode;
            div()
                .id(id)
                .w(px(34.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .when(active, |el: Stateful<Div>| el.bg(rmac_ui::mac::raised()))
                .child(icon(
                    glyph,
                    15.0,
                    if active { label() } else { secondary() },
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.trash_view && mode == ViewMode::Column {
                        this.operation_error = Some("Column view is unavailable in Trash".into());
                        cx.notify();
                        return;
                    }
                    this.view = mode;
                    cx.notify();
                }))
        };
        let view_control = div()
            .flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded(px(7.0))
            .bg(rmac_ui::mac::control_fill())
            .child(seg("v-icon", "icons/layout-grid.svg", ViewMode::Icon))
            .child(seg("v-list", "icons/list.svg", ViewMode::List))
            .child(seg("v-col", "icons/columns-3.svg", ViewMode::Column))
            .child(seg("v-gal", "icons/image.svg", ViewMode::Gallery));

        let tool = |glyph: &'static str| {
            div()
                .w(px(30.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(glyph, 16.0, secondary()))
        };

        let search = div()
            .w(px(200.0))
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded(px(7.0))
            .bg(rmac_ui::mac::control_fill())
            .child(icon("icons/search.svg", 14.0, tertiary()))
            .child(
                div()
                    .flex_1()
                    .child(SearchField::new(&self.query).appearance(false)),
            );

        div()
            .id("toolbar")
            .h(px(52.0))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .pl(px(13.0))
            .pr_3()
            .bg(toolbar_bg())
            .border_b_1()
            .border_color(sep())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.dragging = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.dragging = false),
            )
            .on_mouse_move(cx.listener(|t, _, window, _| {
                if t.dragging {
                    t.dragging = false;
                    window.start_window_move();
                }
            }))
            .child(div().mr_1().child(rmac_ui::traffic_lights()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        nav(
                            "back",
                            "icons/chevron-left.svg",
                            self.trash_view || !self.back.is_empty(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
                    )
                    .child(
                        nav("fwd", "icons/chevron-right.svg", !self.fwd.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
                    ),
            )
            .child(
                div()
                    .pl_1()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(label())
                    .child(self.title()),
            )
            .child(div().flex_1())
            .child(view_control)
            .child(tool("icons/share-2.svg"))
            .child(tool("icons/tag.svg"))
            // The ⋯ button opens the item context menu (anchored below itself).
            .child(
                div()
                    .id("more")
                    .w(px(30.0))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
                    .child(icon("icons/ellipsis.svg", 16.0, secondary()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            this.menu_at = Some(ev.position);
                            cx.notify();
                        }),
                    ),
            )
            .child(search)
    }

    fn title(&self) -> SharedString {
        if let Some(rt) = &self.result_title {
            return rt.clone();
        }
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_volume_name().to_string())
            .into()
    }

    // ---- sidebar ----
    fn render_place(&self, p: &Place, cx: &Context<Self>) -> impl IntoElement {
        let is_tag = p.kind == PlaceKind::Tag;
        let selected = if p.kind == PlaceKind::Trash {
            self.trash_view
        } else {
            !is_tag && !self.trash_view && self.cwd == p.path
        };
        let key = format!("{}-{}", p.name, p.path.display());

        let leading: gpui::AnyElement = if is_tag {
            div()
                .w(px(12.0))
                .h(px(12.0))
                .flex_none()
                .rounded_full()
                .bg(p.tint)
                .into_any_element()
        } else {
            icon(p.icon, 17.0, p.tint).into_any_element()
        };

        let np = p.path.clone();
        let tag_name = p.name.clone();
        let kind = p.kind;
        let main = div()
            .id(SharedString::from(format!("placemain-{key}")))
            .flex_1()
            .flex()
            .items_center()
            .gap_2()
            .min_w(px(0.0))
            .child(leading)
            .child(
                div()
                    .flex_1()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(label())
                    .truncate()
                    .child(p.name.clone()),
            )
            .on_click(cx.listener(move |this, _, _, cx| match kind {
                PlaceKind::Tag => this.tag_click(tag_name.clone(), cx),
                PlaceKind::Recents => this.recents_click(cx),
                PlaceKind::Trash => this.trash_click(cx),
                _ => this.navigate(np.clone(), cx),
            }));

        let mut row = div()
            .id(SharedString::from(format!("place-{key}")))
            .flex()
            .items_center()
            .gap_2()
            .h(px(28.0))
            .px_2()
            .rounded(px(6.0))
            .when(selected, |el: Stateful<Div>| {
                el.bg(rmac_ui::mac::sidebar_selection())
            })
            .when(!selected && !is_tag, |el: Stateful<Div>| {
                el.hover(|h| h.bg(rmac_ui::mac::hover()))
            })
            .child(main);

        if p.kind == PlaceKind::Volume {
            let ep = p.path.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("eject-{key}")))
                    .w(px(18.0))
                    .h(px(18.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .hover(|h| h.bg(rmac_ui::mac::hover()))
                    .child(icon("icons/eject.svg", 11.0, secondary()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.eject_volume(ep.clone(), cx);
                    })),
            );
        }
        row
    }

    fn trash_click(&mut self, cx: &mut Context<Self>) {
        self.trash_view = true;
        if self.view == ViewMode::Column {
            self.view = ViewMode::List;
        }
        self.result_title = Some("Trash".into());
        self.operation_error = None;
        self.reload_trash(cx);
    }

    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let mut col = div()
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_2()
            .px_2()
            .gap_0p5()
            .bg(sidebar_bg())
            .border_r_1()
            .border_color(sep());
        for (si, section) in self.sections.iter().enumerate() {
            col = col.child(
                div()
                    .px_2()
                    .pt(px(if si == 0 { 2.0 } else { 12.0 }))
                    .pb_1()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(rmac_ui::mac::SEMIBOLD)
                    .text_color(secondary())
                    .child(section.title.clone()),
            );
            for p in &section.places {
                col = col.child(self.render_place(p, cx));
            }
        }
        col
    }

    pub(super) fn build_context_menu(
        pos: Point<Pixels>,
        has_selection: bool,
        can_open_with: bool,
        can_paste: bool,
        trash_view: bool,
        undo_label: Option<String>,
    ) -> rmac_ui::ContextMenu {
        let mut m = rmac_ui::ContextMenu::new(pos);
        if let Some(label) = undo_label {
            m = m
                .command_item(label, rmac_ui::shortcuts::UNDO, Box::new(UndoOperation))
                .separator();
        }
        if trash_view {
            if has_selection {
                m = m
                    .item("Restore", Box::new(RestoreItems))
                    .separator()
                    .danger_command_item(
                        "Delete Permanently…",
                        rmac_ui::shortcuts::DELETE_PERMANENT,
                        Box::new(DeletePermanently),
                    );
            }
            return m;
        }
        if has_selection {
            m = m.command_item(
                "Open",
                rmac_ui::shortcuts::OPEN_SELECTION,
                Box::new(OpenItems),
            );
            if can_open_with {
                m = m.item("Open With…", Box::new(OpenWith));
            }
            m = m
                .command_item("Rename", rmac_ui::shortcuts::ENTER, Box::new(RenameItem))
                .command_item(
                    "Duplicate",
                    rmac_ui::shortcuts::DUPLICATE,
                    Box::new(Duplicate),
                )
                .separator()
                .command_item("Copy", rmac_ui::shortcuts::COPY, Box::new(CopyItems))
                .command_item("Cut", rmac_ui::shortcuts::CUT, Box::new(CutItems));
        }
        if can_paste {
            m = m.command_item(
                "Paste Item",
                rmac_ui::shortcuts::PASTE,
                Box::new(PasteItems),
            );
        }
        m = m.separator().command_item(
            "New Folder",
            rmac_ui::shortcuts::NEW_FOLDER,
            Box::new(NewFolder),
        );
        if has_selection {
            m = m
                .separator()
                .command_item(
                    "Move to Trash",
                    rmac_ui::shortcuts::DELETE,
                    Box::new(MoveToTrash),
                )
                .danger_item("Delete Immediately", Box::new(DeleteItem));
        }
        m
    }

    // ---- list ----

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .h(px(30.0))
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(rmac_ui::mac::chrome())
            .border_b_1()
            .border_color(sep());
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let name = tab
                .cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Macintosh HD".into());
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("tab-{i}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(22.0))
                    .px_2()
                    .rounded(px(5.0))
                    .when(active, |el: Stateful<Div>| el.bg(rmac_ui::mac::raised()))
                    .when(!active, |el: Stateful<Div>| {
                        el.hover(|h| h.bg(rmac_ui::mac::hover()))
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("tabname-{i}")))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(label())
                            .child(name)
                            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(i, cx))),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("tabclose-{i}")))
                            .w(px(18.0))
                            .h(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| this.close_tab(i, cx))),
                    ),
            );
        }
        bar.child(div().flex_1()).child(
            div()
                .id("newtab")
                .w(px(26.0))
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .text_size(rmac_ui::text_px(16.0))
                .text_color(secondary())
                .hover(|h| h.bg(rmac_ui::mac::hover()))
                .child("+")
                .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
        )
    }
}
