//! Launcher icon, grid/list result, section, and empty-state projection.

use super::*;

impl LauncherView {
    fn result_icon(row: &Row, size: f32) -> AnyElement {
        if let Some(icon) = row.icon.clone() {
            return img(icon)
                .size(px(size))
                .rounded(px(size * 0.2))
                .flex_none()
                .into_any_element();
        }
        let (glyph, color): (&str, Hsla) = match row.category {
            Category::Applications => ("A", mac::system_blue()),
            Category::Settings => ("⚙", mac::system_gray()),
            Category::Calculator => ("=", mac::system_orange()),
            Category::Files => ("▤", mac::system_teal()),
            Category::Other => ("•", mac::system_indigo()),
        };
        div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size * 0.22))
            .bg(color)
            .text_color(mac::on_accent())
            .font_weight(mac::SEMIBOLD)
            .text_size(rmac_ui::text_px(size * 0.42))
            .child(glyph)
            .into_any_element()
    }

    fn grid_tile(&self, row: &Row, index: usize, cx: &Context<Self>) -> AnyElement {
        let id = row.id.clone();
        div()
            .id(SharedString::from(format!("launcher-grid-{index}")))
            .w(px(104.0))
            .h(px(112.0))
            .v_flex()
            .items_center()
            .justify_center()
            .gap_2()
            .rounded(px(mac::radius_card()))
            .cursor_pointer()
            .when(row.selected, |tile| tile.bg(mac::accent_subtle()))
            .when(!row.selected, |tile| {
                tile.hover(|hover| hover.bg(mac::hover()))
            })
            .child(Self::result_icon(row, 54.0))
            .child(
                div()
                    .w_full()
                    .px_1()
                    .text_center()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text())
                    .child(row.title.clone()),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_and_activate(id.clone(), ActivationMode::Primary, window, cx);
            }))
            .into_any_element()
    }

    fn list_row(&self, row: &Row, index: usize, cx: &Context<Self>) -> AnyElement {
        let primary_id = row.id.clone();
        let alternate_id = row.id.clone();
        div()
            .id(SharedString::from(format!("launcher-row-{index}")))
            .h(px(56.0))
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .rounded(px(mac::radius_control()))
            .cursor_pointer()
            .when(row.selected, |item| item.bg(mac::accent_subtle()))
            .when(!row.selected, |item| {
                item.hover(|hover| hover.bg(mac::hover()))
            })
            .child(Self::result_icon(row, 36.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(mac::MEDIUM)
                            .text_color(mac::text())
                            .child(row.title.clone()),
                    )
                    .when_some(row.subtitle.clone(), |text, subtitle| {
                        text.child(
                            div()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text_secondary())
                                .child(subtitle),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(10.0))
                    .text_color(mac::text_tertiary())
                    .child(row.category_label),
            )
            .when(
                row.has_alternate && self.browse_mode != Some(BrowseMode::Applications),
                |item| {
                    item.child(
                        div()
                            .id(SharedString::from(format!("launcher-alternate-{index}")))
                            .size(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(mac::radius_control()))
                            .bg(mac::control_fill())
                            .hover(|hover| hover.bg(mac::control_fill_hover()))
                            .child("•••")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.select_and_activate(
                                    alternate_id.clone(),
                                    ActivationMode::Alternate,
                                    window,
                                    cx,
                                );
                            })),
                    )
                },
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_and_activate(primary_id.clone(), ActivationMode::Primary, window, cx);
            }))
            .into_any_element()
    }

    pub(super) fn results(&self, rows: &[Row], query: &str, cx: &Context<Self>) -> AnyElement {
        if self.browse_mode == Some(BrowseMode::Applications) {
            return self.application_results(rows, cx);
        }
        if query.is_empty() {
            let applications = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.category == Category::Applications)
                .map(|(index, row)| self.grid_tile(row, index, cx))
                .collect::<Vec<_>>();
            let remaining = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.category != Category::Applications)
                .map(|(index, row)| self.list_row(row, index, cx))
                .collect::<Vec<_>>();
            return div()
                .v_flex()
                .gap_3()
                .when(!applications.is_empty(), |content| {
                    content
                        .child(section_label(APPLICATIONS_SECTION_NAME))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .justify_center()
                                .children(applications),
                        )
                })
                .when(!remaining.is_empty(), |content| {
                    content
                        .child(section_label(SUGGESTIONS_SECTION_NAME))
                        .child(div().v_flex().gap_0p5().children(remaining))
                })
                .into_any_element();
        }

        let mut content = div().v_flex().gap_2();
        let mut last_category = None;
        for (index, row) in rows.iter().enumerate() {
            if last_category != Some(row.category) {
                content = content.child(section_label(row.category_label));
                last_category = Some(row.category);
            }
            content = content.child(self.list_row(row, index, cx));
        }
        content.into_any_element()
    }

    fn application_results(&self, rows: &[Row], cx: &Context<Self>) -> AnyElement {
        let mut content = div().v_flex().gap_4();
        for group in ApplicationGroup::ORDER {
            let matching = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.application_group == Some(group))
                .collect::<Vec<_>>();
            if matching.is_empty() {
                continue;
            }
            let section = match self.application_view {
                ApplicationView::Grid => div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(
                        matching
                            .into_iter()
                            .map(|(index, row)| self.grid_tile(row, index, cx)),
                    )
                    .into_any_element(),
                ApplicationView::List => div()
                    .v_flex()
                    .gap_0p5()
                    .children(
                        matching
                            .into_iter()
                            .map(|(index, row)| self.list_row(row, index, cx)),
                    )
                    .into_any_element(),
            };
            content = content.child(section_label(group.label())).child(section);
        }
        content.into_any_element()
    }
}

fn section_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .pt_1()
        .px_2()
        .text_size(rmac_ui::text_px(10.0))
        .font_weight(mac::SEMIBOLD)
        .text_color(mac::text_tertiary())
        .child(label.into())
}
