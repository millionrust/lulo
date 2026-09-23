//! Launcher icon, grid/list result, section, and empty-state projection.

use super::*;

impl LauncherView {
    pub(super) fn result_icon(row: &Row, size: f32) -> AnyElement {
        if let Some(icon) = row.icon.clone() {
            return img(icon)
                .size(px(size))
                .rounded(px(size * 0.2))
                .flex_none()
                .into_any_element();
        }
        if let Some(asset) = category_icon(row.category) {
            return img(asset).size(px(size)).flex_none().into_any_element();
        }
        let (glyph, color): (&str, Hsla) = match row.category {
            Category::Applications => ("A", mac::system_blue()),
            Category::Settings => ("⚙", mac::system_gray()),
            Category::Calculator | Category::Clock => ("=", mac::system_orange()),
            // The Mac shows its red Dictionary icon; rmac has no dictionary
            // app, so the same "Aa" is drawn on red.
            Category::Dictionary => ("Aa", mac::system_red()),
            Category::Files => ("▤", mac::system_teal()),
            Category::Other | Category::SearchIn => ("•", mac::system_indigo()),
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
            .role(Role::Button)
            .aria_label(row.title.clone())
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

    /// One Spotlight result row: icon, name and a tertiary " — kind" suffix
    /// on one line; the selected row takes the accent fill.
    fn list_row(&self, row: &Row, index: usize, cx: &Context<Self>) -> AnyElement {
        let primary_id = row.id.clone();
        let alternate_id = row.id.clone();
        let selected = row.selected;
        let title_color = if selected {
            mac::on_accent()
        } else {
            mac::text()
        };
        let detail_color = if selected {
            mac::on_accent().opacity(0.75)
        } else {
            mac::text_tertiary()
        };
        div()
            .id(SharedString::from(format!("launcher-row-{index}")))
            .role(Role::Button)
            .aria_label(match &row.subtitle {
                Some(subtitle) => format!("{}, {subtitle}", row.title),
                None => row.title.clone(),
            })
            .h(px(metrics::ROW_HEIGHT))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(metrics::ROW_GAP))
            .px(px(metrics::ROW_INSET))
            .rounded(px(metrics::ROW_RADIUS))
            .cursor_pointer()
            .text_size(rmac_ui::text_px(metrics::ROW_TEXT))
            .when(selected, |item| item.bg(mac::accent()))
            .when(!selected, |item| item.hover(|hover| hover.bg(mac::hover())))
            .child(Self::result_icon(row, metrics::ROW_ICON))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(metrics::ROW_TITLE_MAX))
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(title_color)
                            .child(row.title.clone()),
                    )
                    .when_some(row.subtitle.clone(), |text, subtitle| {
                        text.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_color(detail_color)
                                .child(format!("— {subtitle}")),
                        )
                    }),
            )
            .when(
                selected && row.has_alternate && self.browse_mode != Some(BrowseMode::Applications),
                |item| {
                    item.child(
                        div()
                            .id(SharedString::from(format!("launcher-alternate-{index}")))
                            .role(Role::Button)
                            .aria_label(row.alternate_label.unwrap_or("Show in Folder"))
                            .size(px(22.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .bg(mac::on_accent().opacity(0.18))
                            .hover(|hover| hover.bg(mac::on_accent().opacity(0.28)))
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(mac::on_accent())
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

        let labels = rows
            .iter()
            .map(|row| row.category_label)
            .collect::<Vec<_>>();
        let mut content = div().v_flex();
        for (label, range) in completion::query_sections(&labels) {
            content = content.child(section_label(label));
            let start = range.start;
            for (offset, row) in rows[range].iter().enumerate() {
                content = content.child(self.list_row(row, start + offset, cx));
            }
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

/// The rmac app that answers a category, as the Mac shows Calculator,
/// Clock and Finder beside answers and "Search in Finder".
pub(super) fn category_icon(category: Category) -> Option<&'static str> {
    match category {
        Category::Calculator => Some("spotlight/apps/org.rmac.Calculator.svg"),
        Category::Clock => Some("spotlight/apps/org.rmac.Clock.svg"),
        Category::SearchIn => Some("spotlight/apps/org.rmac.Files.svg"),
        _ => None,
    }
}

/// Section header: 11 pt semibold label tertiary, set on the row inset.
fn section_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .h(px(metrics::SECTION_HEIGHT))
        .flex_none()
        .flex()
        .items_end()
        .pb(px(4.0))
        .px(px(metrics::ROW_INSET))
        .text_size(rmac_ui::text_px(11.0))
        .font_weight(mac::SEMIBOLD)
        .text_color(mac::text_tertiary())
        .child(label.into())
}
