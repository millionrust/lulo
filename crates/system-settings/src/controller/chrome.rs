//! Window chrome and category-sidebar rendering.

use super::*;

impl Settings {
    pub(super) fn render_topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_back = !self.nav.is_empty();
        let back = div()
            .id(rmac_system_settings::accessibility::BACK_ID)
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.0))
            .h(px(26.0))
            .rounded(px(6.0))
            .when(can_back, |el: Stateful<Div>| {
                el.hover(|h| h.bg(rmac_ui::mac::hover()))
                    .cursor_pointer()
                    .on_click(cx.listener(|t, _, _, cx| t.go_back(cx)))
            })
            .child(glyph(
                "icons/chevron-left.svg",
                17.0,
                if can_back {
                    accent()
                } else {
                    rmac_ui::mac::text_tertiary()
                },
            ));

        div()
            .id("topbar")
            .h(px(52.0))
            .flex_none()
            .w_full()
            .flex()
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
            .child(
                div()
                    .w(px(SIDEBAR_W))
                    .h_full()
                    .bg(sidebar_bg())
                    .flex()
                    .items_center()
                    .pl(px(13.0))
                    .child(rmac_ui::traffic_lights()),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .bg(pane_bg())
                    .flex()
                    .items_center()
                    .pl_3()
                    .gap_1()
                    .child(back)
                    .child(glyph(
                        "icons/chevron-right.svg",
                        17.0,
                        rmac_ui::mac::text_tertiary(),
                    )),
            )
    }

    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let search = div()
            .id(rmac_system_settings::accessibility::SEARCH_ID)
            .mx_2()
            .mt_1()
            .mb_2()
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded(px(7.0))
            .bg(rmac_ui::mac::control_fill())
            .child(glyph("icons/search.svg", 13.0, secondary()))
            .child(
                div()
                    .flex_1()
                    .child(SearchField::new(&self.search).appearance(false)),
            );
        let query = self.search.read(cx).value().to_string();

        let account = div()
            .flex()
            .items_center()
            .gap_2p5()
            .mx_2()
            .mb_2()
            .px_2()
            .py_1p5()
            .rounded(px(8.0))
            .child(
                div()
                    .w(px(38.0))
                    .h(px(38.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rmac_ui::mac::control_fill())
                    .child(glyph("icons/user.svg", 22.0, secondary())),
            )
            .child(
                div()
                    .v_flex()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(self.account.clone()),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(secondary())
                            .child(rmac_system_settings::accessibility::LOCAL_ACCOUNT_LABEL),
                    ),
            );

        let mut col = div()
            .id(rmac_system_settings::accessibility::SIDEBAR_ID)
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_1()
            .bg(sidebar_bg())
            .border_r_1()
            .border_color(sep())
            .overflow_y_scroll()
            .child(search)
            .child(account);

        let mut first_section = true;
        for (si, section) in self.sections.iter().enumerate() {
            let matching: Vec<(usize, &Category)> = section
                .iter()
                .enumerate()
                .filter(|(_, category)| {
                    rmac_system_settings::accessibility::category_matches(
                        &query,
                        category.name.as_ref(),
                    )
                })
                .collect();
            if matching.is_empty() {
                continue;
            }
            if !first_section {
                col = col.child(div().h(px(14.0)));
            }
            first_section = false;
            for (ci, cat) in matching {
                let selected = self.selected == (si, ci);
                col = col.child(
                    ListRow::new(
                        SharedString::from(format!("cat-{si}-{ci}")),
                        div()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .child(tile(cat.icon, cat.color, 20.0))
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .text_color(if selected { on_accent() } else { label() })
                                    .child(cat.name.clone()),
                            ),
                    )
                    .selected(selected)
                    .mx_2()
                    .px_2()
                    .on_activate(cx.listener(move |t, _, _, cx| {
                        t.selected = (si, ci);
                        t.nav.clear();
                        cx.notify();
                    })),
                );
            }
        }
        col
    }
}
