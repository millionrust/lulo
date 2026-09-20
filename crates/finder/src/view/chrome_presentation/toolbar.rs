use super::*;

impl FinderView {
    pub(in crate::view) fn render_toolbar(
        &self,
        layout: crate::view::responsive_layout::ResponsiveLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let nav = |id: &'static str, icon_name: IconName, tooltip: &'static str, enabled: bool| {
            Button::new(id, "")
                .icon(Icon::new(icon_name).text_color(rmac_ui::mac::text()))
                .ghost()
                .with_size(Size::Small)
                .disabled(!enabled)
                .tooltip(tooltip)
        };
        let cur = self.view;
        let seg = |id: &'static str, icon_name: IconName, tooltip: &'static str, mode: ViewMode| {
            let active = cur == mode;
            Button::new(id, "")
                .icon(Icon::new(icon_name).text_color(rmac_ui::mac::text()))
                .ghost()
                .with_size(Size::Small)
                .selected(active)
                .tooltip(tooltip)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_view_mode(mode, cx);
                }))
        };
        let view_control = rmac_ui::toolbar_group(
            div()
                .flex()
                .items_center()
                .gap_0p5()
                .child(seg(
                    "v-icon",
                    IconName::LayoutDashboard,
                    "Icon View",
                    ViewMode::Icon,
                ))
                .child(seg("v-list", IconName::Menu, "List View", ViewMode::List))
                .child(seg(
                    "v-col",
                    IconName::PanelLeft,
                    "Column View",
                    ViewMode::Column,
                ))
                .child(seg(
                    "v-gal",
                    IconName::GalleryVerticalEnd,
                    "Gallery View",
                    ViewMode::Gallery,
                )),
        );

        let search = div()
            .w(px(layout.search_width))
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded(px(rmac_ui::mac::radius_segmented()))
            .bg(rmac_ui::mac::field_fill())
            .child(icon("icons/search.svg", 14.0, tertiary()))
            .child(
                div()
                    .flex_1()
                    .child(SearchField::new(&self.query).appearance(false)),
            );

        let sidebar_toggle = Button::new("toggle-sidebar", "")
            .icon(
                Icon::new(if layout.sidebar_visible {
                    IconName::PanelLeftClose
                } else {
                    IconName::PanelLeftOpen
                })
                .text_color(rmac_ui::mac::text()),
            )
            .ghost()
            .with_size(Size::Small)
            .selected(layout.sidebar_visible)
            .disabled(!layout.sidebar_available)
            .tooltip(if !layout.sidebar_available {
                "Sidebar hidden until the window is wider"
            } else if layout.sidebar_visible {
                "Hide Sidebar"
            } else {
                "Show Sidebar"
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)));

        let leading = div()
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .pl(px(13.0))
            .pr_2()
            .child(rmac_ui::traffic_lights())
            .child(div().flex_1())
            .child(sidebar_toggle)
            .when(layout.sidebar_visible, |leading| {
                leading.w(px(self.sidebar_width))
            })
            .when(!layout.sidebar_visible, |leading| leading.w(px(108.0)));

        let navigation_control = rmac_ui::toolbar_group(
            div()
                .flex()
                .items_center()
                .gap_0p5()
                .child(
                    nav(
                        "back",
                        IconName::ChevronLeft,
                        "Back",
                        self.trash_view || self.applications_view || !self.back.is_empty(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
                )
                .child(
                    nav(
                        "fwd",
                        IconName::ChevronRight,
                        "Forward",
                        !self.fwd.is_empty(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
                ),
        );
        let sort_control = rmac_ui::toolbar_group(
            Button::new("sort", "")
                .icon(Icon::new(IconName::SortDescending).text_color(rmac_ui::mac::text()))
                .ghost()
                .with_size(Size::Small)
                .tooltip("Sort")
                .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                    this.menu_purpose = MenuPurpose::Sort;
                    this.menu_at = Some(rmac_ui::ContextMenuState::open(
                        event.position(),
                        &this.focus,
                        window,
                        cx,
                    ));
                    cx.notify();
                })),
        );

        div()
            .id("toolbar")
            .h(px(rmac_ui::mac::toolbar_height()))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .pr_3()
            .relative()
            .bg(toolbar_bg())
            .when(layout.sidebar_visible, |toolbar| {
                toolbar.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(self.sidebar_width))
                        .bg(sidebar_bg()),
                )
            })
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
            .child(leading)
            .child(navigation_control)
            .when(layout.title_visible, |toolbar| {
                toolbar.child(
                    div()
                        .pl_1()
                        .text_size(rmac_ui::text_px(13.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(label())
                        .child(self.title()),
                )
            })
            .when(layout.view_control_visible, |toolbar| {
                toolbar.child(div().ml(px(16.0)).child(view_control))
            })
            .when(layout.view_control_visible, |toolbar| toolbar.child(sort_control))
            .child(div().flex_1())
            // The ⋯ button opens the item context menu (anchored below itself).
            .child(
                rmac_ui::toolbar_group(
                    Button::new("more", "")
                        .icon(Icon::new(IconName::Ellipsis).text_color(rmac_ui::mac::text()))
                        .ghost()
                        .with_size(Size::Small)
                        .tooltip("More Actions")
                        .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                            this.menu_purpose = MenuPurpose::Context;
                            this.menu_at = Some(rmac_ui::ContextMenuState::open(
                                ev.position(),
                                &this.focus,
                                window,
                                cx,
                            ));
                            cx.notify();
                        })),
                ),
            )
            .child(search)
    }

    pub(in crate::view) fn title(&self) -> SharedString {
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
}
