use super::*;

impl FinderView {
    pub(in crate::view) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
}
