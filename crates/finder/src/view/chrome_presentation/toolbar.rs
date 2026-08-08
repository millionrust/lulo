use super::*;

impl FinderView {
    pub(in crate::view) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let view_control = div()
            .flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded(px(7.0))
            .bg(rmac_ui::mac::control_fill())
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
            ));

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
                Button::new("toggle-sidebar", "")
                    .icon(
                        Icon::new(if self.sidebar_visible {
                            IconName::PanelLeftClose
                        } else {
                            IconName::PanelLeftOpen
                        })
                        .text_color(rmac_ui::mac::text()),
                    )
                    .ghost()
                    .with_size(Size::Small)
                    .selected(self.sidebar_visible)
                    .tooltip(if self.sidebar_visible {
                        "Hide Sidebar"
                    } else {
                        "Show Sidebar"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        nav(
                            "back",
                            IconName::ChevronLeft,
                            "Back",
                            self.trash_view || !self.back.is_empty(),
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
            // The ⋯ button opens the item context menu (anchored below itself).
            .child(
                Button::new("more", "")
                    .icon(Icon::new(IconName::Ellipsis).text_color(rmac_ui::mac::text()))
                    .ghost()
                    .with_size(Size::Small)
                    .tooltip("More Actions")
                    .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                        this.menu_at = Some(rmac_ui::ContextMenuState::open(
                            ev.position(),
                            &this.focus,
                            window,
                            cx,
                        ));
                        cx.notify();
                    })),
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
