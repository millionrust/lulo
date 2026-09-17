//! Window chrome and category-sidebar rendering.

use super::*;

impl Settings {
    pub(super) fn render_topbar(
        &self,
        layout: crate::responsive_layout::SettingsLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let can_back = !self.nav.is_empty();
        let back = ListRow::new(
            rmac_system_settings::accessibility::BACK_ID,
            glyph(
                "icons/chevron-left.svg",
                17.0,
                if can_back {
                    accent()
                } else {
                    rmac_ui::mac::text_tertiary()
                },
            ),
        )
        .disabled(!can_back)
        .w(px(26.0))
        .h(px(26.0))
        .justify_center()
        .on_activate(cx.listener(|t, _, _, cx| t.go_back(cx)));
        let sidebar_toggle =
            Button::new(rmac_system_settings::accessibility::SIDEBAR_TOGGLE_ID, "")
                .icon(
                    Icon::new(if layout.sidebar_visible {
                        IconName::PanelLeftClose
                    } else {
                        IconName::PanelLeftOpen
                    })
                    .text_color(label()),
                )
                .ghost()
                .xsmall()
                .selected(layout.sidebar_visible)
                .tooltip(if layout.sidebar_visible {
                    rmac_system_settings::accessibility::HIDE_SIDEBAR_NAME
                } else {
                    rmac_system_settings::accessibility::SHOW_SIDEBAR_NAME
                })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_compact_sidebar(cx)));

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
                    .w(px(if layout.compact { 132.0 } else { SIDEBAR_W }))
                    .h_full()
                    .bg(if layout.compact {
                        pane_bg()
                    } else {
                        sidebar_bg()
                    })
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(13.0))
                    .child(rmac_ui::traffic_lights())
                    .when(layout.compact, |leading| leading.child(sidebar_toggle)),
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

    pub(super) fn render_sidebar(&self, compact: bool, cx: &Context<Self>) -> impl IntoElement {
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
            .rounded(px(mac::radius_segmented()))
            .bg(rmac_ui::mac::control_fill())
            .child(glyph("icons/search.svg", 13.0, secondary()))
            .child(
                div()
                    .flex_1()
                    .child(SearchField::new(&self.search).appearance(false)),
            );
        let query = self.search.read(cx).value().to_string();
        let searching = !query.trim().is_empty();
        let search_matches = self.search_matches(cx);
        let no_search_results = searching && search_matches.is_empty();
        let active_search_result = self
            .search_selection
            .min(search_matches.len().saturating_sub(1));

        let account = div()
            .flex()
            .items_center()
            .gap_2p5()
            .mx_2()
            .mb_2()
            .px_2()
            .py_1p5()
            .rounded(px(mac::radius_control()))
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
            .when(compact, |sidebar| sidebar.w_full())
            .when(!compact, |sidebar| sidebar.w(px(SIDEBAR_W)))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_1()
            .bg(sidebar_bg())
            .border_r_1()
            .border_color(sep())
            .overflow_y_scroll()
            .child(search)
            .child(account)
            .when(no_search_results, |sidebar| {
                sidebar.child(
                    div()
                        .mx_4()
                        .mt_6()
                        .v_flex()
                        .items_center()
                        .gap_1()
                        .text_center()
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(13.0))
                                .font_weight(rmac_ui::mac::SEMIBOLD)
                                .text_color(label())
                                .child("No Settings Found"),
                        )
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(secondary())
                                .child("Try a different search."),
                        ),
                )
            });

        let mut first_section = true;
        let mut search_result_index = 0;
        for (si, section) in self.sections.iter().enumerate() {
            let matching: Vec<(usize, &Category)> = section
                .iter()
                .enumerate()
                .filter(|(_, category)| crate::settings_search::matches(category, &query))
                .collect();
            if matching.is_empty() {
                continue;
            }
            if !first_section {
                col = col.child(div().h(px(14.0)));
            }
            first_section = false;
            for (ci, cat) in matching {
                let search_context = searching.then(|| {
                    crate::settings_search::match_hint(cat, &query)
                        .map(SharedString::from)
                        .unwrap_or_else(|| cat.desc.clone())
                });
                let selected = if searching {
                    search_result_index == active_search_result
                } else {
                    self.selected == (si, ci)
                };
                search_result_index += 1;
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
                                    .min_w_0()
                                    .v_flex()
                                    .child(
                                        div()
                                            .text_size(rmac_ui::text_px(13.0))
                                            .text_color(if selected {
                                                on_accent()
                                            } else {
                                                label()
                                            })
                                            .child(cat.name.clone()),
                                    )
                                    .when_some(search_context, |content, context| {
                                        content.child(
                                            div()
                                                .max_w(px(if compact {
                                                    480.0
                                                } else {
                                                    SIDEBAR_W - 68.0
                                                }))
                                                .overflow_hidden()
                                                .text_size(rmac_ui::text_px(9.5))
                                                .text_color(if selected {
                                                    on_accent()
                                                } else {
                                                    secondary()
                                                })
                                                .child(context),
                                        )
                                    }),
                            ),
                    )
                    .selected(selected)
                    .mx_2()
                    .px_2()
                    .h(px(if searching { 42.0 } else { 30.0 }))
                    .on_activate(cx.listener(move |t, _, window, cx| {
                        t.select_position((si, ci), cx);
                        if searching {
                            t.clear_search(window, cx);
                            window.focus(&t.focus);
                        }
                    })),
                );
            }
        }
        col
    }
}
