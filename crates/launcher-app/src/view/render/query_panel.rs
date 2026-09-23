//! Query results as macOS 26.2 draws them: the bar grows downwards into one
//! glass panel holding the answer card, a rule and 56 pt rows, with the
//! "Search in Files" row last (`design-lab/spotlight.html` records every
//! measured value). Tahoe has no preview pane; ⌘Y opens Quick Look.

use super::*;

mod query_metrics {
    /// The panel grows with its content up to the measured 467 pt.
    pub(super) const MAX_HEIGHT: f32 = 467.0;
    pub(super) const RADIUS: f32 = 28.0;
    pub(super) const HEADER: f32 = 56.0;
    /// Rule and plates from the panel's outer edge.
    pub(super) const RULE_INSET: f32 = 20.0;
    pub(super) const PLATE_INSET: f32 = 10.0;
    pub(super) const LIST_PADDING: f32 = 10.0;
    pub(super) const ROW: f32 = 56.0;
    pub(super) const ROW_RADIUS: f32 = 17.0;
    /// The top hit is a section of its own, 2 pt above the next row.
    pub(super) const TOP_HIT_GAP: f32 = 2.0;
    pub(super) const ICON: f32 = 36.0;
    pub(super) const ICON_LEFT: f32 = 18.0;
    pub(super) const TEXT_LEFT: f32 = 71.5;
    pub(super) const TEXT_RIGHT: f32 = 20.0;
    pub(super) const TITLE: f32 = 17.0;
    pub(super) const TITLE_LINE: f32 = 21.0;
    pub(super) const SUBTITLE: f32 = 15.0;
    pub(super) const SUBTITLE_LINE: f32 = 19.0;
    /// One-line rows ("Search in Files").
    pub(super) const SINGLE_TITLE: f32 = 18.0;
    /// Answer card: under the bar, 18 in from the panel's edges.
    pub(super) const CARD_INSET: f32 = 18.0;
    pub(super) const CARD_TOP: f32 = 1.0;
    pub(super) const CARD_RADIUS: f32 = 16.0;
    pub(super) const CARD_GAP: f32 = 8.0;
    pub(super) const CALCULATION: f32 = 62.0;
    /// A currency answer adds the rate source's line.
    pub(super) const CURRENCY: f32 = 84.0;
    pub(super) const CLOCK: f32 = 60.0;
    pub(super) const DEFINITION: f32 = 60.0;
    pub(super) const LABEL_LEFT: f32 = 18.5;
    pub(super) const LABEL: f32 = 13.0;
    pub(super) const VALUE: f32 = 17.0;
    pub(super) const COPY: f32 = 26.0;
    pub(super) const COPY_RIGHT: f32 = 16.0;
    pub(super) const COPY_GLYPH: (f32, f32) = (12.0, 14.0);
    pub(super) const CARD_TITLE: f32 = 18.0;
    pub(super) const CARD_SUBTITLE: f32 = 16.0;
    pub(super) const SOURCE: f32 = 11.0;
    pub(super) const EMPTY: f32 = 56.0;
}
use query_metrics as qm;

// Measured dark colours (2026-09-23 captures). Light appearance keeps the
// structure with estimated tints, marked S.

fn panel_fill() -> Hsla {
    if is_dark() {
        // (28,28,29) in the list area over a dark backdrop.
        Hsla::from(gpui::rgba(0x1c1c1de6))
    } else {
        Hsla::from(gpui::rgba(0xf6f6f8e6))
    }
}

/// The bar zone reads (35,35,36) against the list's (28,28,29).
fn bar_zone() -> Hsla {
    if is_dark() {
        gpui::hsla(0.0, 0.0, 1.0, 0.03)
    } else {
        gpui::transparent_black()
    }
}

fn white(alpha: f32) -> Hsla {
    if is_dark() {
        gpui::hsla(0.0, 0.0, 1.0, alpha)
    } else {
        // S: light appearance.
        gpui::hsla(0.0, 0.0, 0.0, alpha * 0.55)
    }
}

/// Rule (64,64,65) and the unmoved top hit's plate (64,65,65).
fn rule() -> Hsla {
    white(0.155)
}

fn top_hit_plate() -> Hsla {
    white(0.16)
}

/// The keyboard selection (47,98,179) and its subtitle (112,163,245).
fn keyboard_plate() -> Hsla {
    if is_dark() {
        Hsla::from(gpui::rgb(0x2f62b3))
    } else {
        mac::accent()
    }
}

fn keyboard_subtitle() -> Hsla {
    if is_dark() {
        Hsla::from(gpui::rgb(0x70a3f5))
    } else {
        mac::on_accent().opacity(0.8)
    }
}

/// Row subtitle (128,130,130).
fn subtitle_color() -> Hsla {
    if is_dark() {
        Hsla::from(gpui::rgb(0x808282))
    } else {
        mac::text_secondary()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowState {
    Plain,
    TopHit,
    Keyboard,
}

impl LauncherView {
    /// The whole query view: bar, answer card, rule and list.
    pub(super) fn query_panel(
        &self,
        query: &str,
        rows: &[Row],
        activating: bool,
        phase_message: SharedString,
        cx: &Context<Self>,
    ) -> AnyElement {
        let card = rows
            .first()
            .filter(|row| row.category.is_answer() || row.category == Category::Dictionary);
        let list_rows = if card.is_some() { &rows[1..] } else { rows };
        let card_height = card.map(card_height);
        let reserved = qm::HEADER
            + card_height.map_or(0.0, |height| qm::CARD_TOP + height + qm::CARD_GAP)
            + 1.0;
        let list_height = (qm::MAX_HEIGHT - reserved).max(qm::ROW);
        let start = usize::from(card.is_some());

        let mut list = Vec::new();
        for (offset, row) in list_rows.iter().enumerate() {
            let index = start + offset;
            let state = if !row.selected {
                RowState::Plain
            } else if self.keyboard_selection {
                RowState::Keyboard
            } else {
                RowState::TopHit
            };
            // The top hit stands alone as its section.
            let gap = index == 0 && list_rows.len() > 1;
            list.push(self.query_row(row, index, state, gap, cx));
        }

        glass(
            div()
                .w(px(metrics::GROUP_WIDTH))
                .max_h(px(qm::MAX_HEIGHT))
                .flex_none()
                .v_flex()
                .overflow_hidden(),
            qm::RADIUS,
        )
        .bg(panel_fill())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.press_inside = true),
        )
        .child(self.query_header(query, rows, activating, cx))
        .when_some(card, |panel, row| panel.child(self.answer_card(row, cx)))
        .child(
            div()
                .flex_none()
                .h(px(1.0))
                .mt(px(card.map_or(0.0, |_| qm::CARD_GAP)))
                .mx(px(qm::RULE_INSET - metrics::RIM))
                .bg(rule()),
        )
        .when_some(self.settings_error.clone(), |panel, error| {
            panel.child(
                div()
                    .flex_none()
                    .mx(px(qm::RULE_INSET - metrics::RIM))
                    .mt(px(8.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::warning_text())
                    .child(error),
            )
        })
        .when(!list.is_empty(), |panel| {
            panel.child(
                div()
                    .id(RESULTS_ID)
                    .flex_none()
                    .max_h(px(list_height))
                    .overflow_y_scroll()
                    .v_flex()
                    .py(px(qm::LIST_PADDING))
                    .children(list),
            )
        })
        .when(rows.is_empty(), |panel| {
            panel.child(
                div()
                    .id(RESULTS_ID)
                    .flex_none()
                    .h(px(qm::EMPTY))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rmac_ui::text_px(qm::TITLE))
                    .text_color(mac::text_secondary())
                    .child(phase_message),
            )
        })
        .into_any_element()
    }

    /// The bar inside the panel: glyph, query with its completion, and the
    /// selected result's icon at the right end.
    fn query_header(
        &self,
        query: &str,
        rows: &[Row],
        activating: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let selected = rows.iter().position(|row| row.selected);
        let completion = selected.and_then(|index| completion_for(query, &rows[index], index));
        let icon_row = selected.map(|index| &rows[index]).or(rows.first());
        let trailing = if icon_row.is_some() {
            metrics::TOP_HIT_RIGHT
        } else {
            metrics::TEXT_TRAIL
        };
        div()
            .relative()
            .flex_none()
            .h(px(qm::HEADER - metrics::RIM))
            .flex()
            .items_center()
            .bg(bar_zone())
            .pl(px(metrics::TEXT_LEFT - metrics::RIM))
            .pr(px(trailing - metrics::RIM))
            .child(
                svg()
                    .path("spotlight/search.svg")
                    .absolute()
                    .left(px(metrics::SEARCH_GLYPH_LEFT - metrics::RIM))
                    .top(px(
                        (metrics::BAR_HEIGHT - metrics::SEARCH_GLYPH) / 2.0 - metrics::RIM
                    ))
                    .size(px(metrics::SEARCH_GLYPH))
                    .text_color(mac::text_tertiary()),
            )
            .child(self.query_field(query, QUERY_NAME, completion, activating, cx))
            .when_some(icon_row, |header, row| {
                header.child(
                    div()
                        .flex_none()
                        .ml(px(8.0))
                        .child(Self::result_icon(row, metrics::TOP_HIT_ICON)),
                )
            })
            .into_any_element()
    }

    /// One result row: 36 pt icon, 17 pt title over a 15 pt subtitle (or
    /// one 18 pt line), on the grey top-hit or blue keyboard plate.
    fn query_row(
        &self,
        row: &Row,
        index: usize,
        state: RowState,
        gap: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let primary_id = row.id.clone();
        let subtitle = row.subtitle.clone().filter(|subtitle| !subtitle.is_empty());
        let detail_color = if state == RowState::Keyboard {
            keyboard_subtitle()
        } else {
            subtitle_color()
        };
        let title_color = if state == RowState::Keyboard {
            mac::on_accent()
        } else {
            mac::text()
        };
        let text = div()
            .absolute()
            .left(px(qm::TEXT_LEFT - qm::PLATE_INSET))
            .right(px(qm::TEXT_RIGHT - qm::PLATE_INSET))
            .top_0()
            .bottom_0()
            .v_flex()
            .justify_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(
                div()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(rmac_ui::text_px(if subtitle.is_some() {
                        qm::TITLE
                    } else {
                        qm::SINGLE_TITLE
                    }))
                    .line_height(px(qm::TITLE_LINE))
                    .text_color(title_color)
                    .child(row.title.clone()),
            )
            .when_some(subtitle, |text, subtitle| {
                text.child(Self::row_subtitle(row.category, subtitle, detail_color))
            });
        div()
            .id(SharedString::from(format!("spotlight-result-{index}")))
            .relative()
            .flex_none()
            .h(px(qm::ROW))
            .mx(px(qm::PLATE_INSET - metrics::RIM))
            .when(gap, |item| item.mb(px(qm::TOP_HIT_GAP)))
            .rounded(px(qm::ROW_RADIUS))
            .cursor_pointer()
            .map(|item| match state {
                RowState::Keyboard => item.bg(keyboard_plate()),
                RowState::TopHit => item.bg(top_hit_plate()),
                RowState::Plain => item.hover(|hover| hover.bg(mac::hover())),
            })
            .child(
                div()
                    .absolute()
                    .left(px(qm::ICON_LEFT - qm::PLATE_INSET))
                    .top(px((qm::ROW - qm::ICON) / 2.0))
                    .child(Self::result_icon(row, qm::ICON)),
            )
            .child(text)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_and_activate(primary_id.clone(), ActivationMode::Primary, window, cx);
            }))
            .into_any_element()
    }

    /// A file row's "24 KB · Today, 9:08 PM · rmac" draws a folder glyph
    /// before the folder's name, as the Mac does.
    fn row_subtitle(category: Category, subtitle: String, color: Hsla) -> AnyElement {
        let line = div()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(rmac_ui::text_px(qm::SUBTITLE))
            .line_height(px(qm::SUBTITLE_LINE))
            .text_color(color);
        let split = subtitle
            .rsplit_once(" · ")
            .map(|(details, folder)| (details.to_owned(), folder.to_owned()));
        match (category, split) {
            (Category::Files, Some((details, folder))) => line
                .child(div().flex_none().child(format!("{details} · ")))
                .child(
                    svg()
                        .path("spotlight/folder-inline.svg")
                        .flex_none()
                        .w(px(13.0))
                        .h(px(10.0))
                        .mr(px(3.0))
                        .text_color(color),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(folder),
                )
                .into_any_element(),
            _ => line
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(subtitle),
                )
                .into_any_element(),
        }
    }

    /// The answer card under the bar: a calculation or conversion (label,
    /// value, copy button; currency adds its source), the time in a city,
    /// or a definition.
    fn answer_card(&self, row: &Row, cx: &Context<Self>) -> AnyElement {
        let height = card_height(row);
        let id = row.id.clone();
        let card = div()
            .id("spotlight-answer")
            .relative()
            .flex_none()
            .h(px(height))
            .mt(px(qm::CARD_TOP))
            .mx(px(qm::CARD_INSET - metrics::RIM))
            .rounded(px(qm::CARD_RADIUS))
            .border_1()
            .border_color(white(0.30))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_and_activate(id.clone(), ActivationMode::Primary, window, cx);
            }));
        match row.category {
            Category::Clock => card
                .child(
                    div()
                        .absolute()
                        .left(px(qm::CARD_INSET - 1.0))
                        .top(px((height - qm::ICON) / 2.0 - 1.0))
                        .child(Self::result_icon(row, qm::ICON)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(qm::TEXT_LEFT - qm::CARD_INSET - 1.0))
                        .right(px(120.0))
                        .top_0()
                        .bottom_0()
                        .v_flex()
                        .justify_center()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(qm::CARD_TITLE))
                                .line_height(px(22.0))
                                .text_color(mac::text())
                                .child(row.title.clone()),
                        )
                        .when_some(row.subtitle.clone(), |text, subtitle| {
                            text.child(
                                div()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(rmac_ui::text_px(qm::CARD_SUBTITLE))
                                    .line_height(px(20.0))
                                    // (93,93,95)
                                    .text_color(if is_dark() {
                                        Hsla::from(gpui::rgb(0x5d5d5f))
                                    } else {
                                        mac::text_tertiary()
                                    })
                                    .child(subtitle),
                            )
                        }),
                )
                .when_some(row.detail.clone(), |card, time| {
                    card.child(
                        div()
                            .absolute()
                            .right(px(20.0))
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .text_size(rmac_ui::text_px(qm::VALUE))
                            .font_weight(mac::MEDIUM)
                            .text_color(mac::text())
                            .child(time),
                    )
                })
                .into_any_element(),
            Category::Dictionary => card
                .bg(white(0.135))
                .child(
                    div()
                        .absolute()
                        .left(px(qm::CARD_INSET - 1.0))
                        .top(px((height - qm::ICON) / 2.0 - 1.0))
                        .child(Self::result_icon(row, qm::ICON)),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(qm::TEXT_LEFT - qm::CARD_INSET - 1.0))
                        .right(px(20.0))
                        .top_0()
                        .bottom_0()
                        .v_flex()
                        .justify_center()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(
                            div()
                                .flex()
                                .overflow_hidden()
                                .text_size(rmac_ui::text_px(qm::CARD_TITLE))
                                .line_height(px(22.0))
                                .text_color(mac::text())
                                .child(
                                    div()
                                        .flex_none()
                                        .font_weight(mac::SEMIBOLD)
                                        .child(row.title.clone()),
                                )
                                .when_some(row.detail.clone(), |title, source| {
                                    title.child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(format!(" — {source}")),
                                    )
                                }),
                        )
                        .when_some(row.subtitle.clone(), |text, definition| {
                            text.child(
                                div()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(rmac_ui::text_px(qm::CARD_SUBTITLE))
                                    .line_height(px(20.0))
                                    // (130,131,134)
                                    .text_color(if is_dark() {
                                        Hsla::from(gpui::rgb(0x828386))
                                    } else {
                                        mac::text_secondary()
                                    })
                                    .child(definition),
                            )
                        }),
                )
                .into_any_element(),
            _ => {
                let value = row.title.clone();
                let label = format!("{} =", row.subtitle.clone().unwrap_or_default());
                card.bg(white(0.16))
                    .child(
                        div()
                            .absolute()
                            .left(px(qm::LABEL_LEFT - 1.0))
                            .right(px(qm::COPY_RIGHT + qm::COPY + 8.0))
                            .top(px(11.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(rmac_ui::text_px(qm::LABEL))
                            .line_height(px(16.0))
                            // (188,189,189)
                            .text_color(if is_dark() {
                                Hsla::from(gpui::rgb(0xbcbdbd))
                            } else {
                                mac::text_secondary()
                            })
                            .child(label),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(qm::LABEL_LEFT - 1.0))
                            .right(px(qm::COPY_RIGHT + qm::COPY + 8.0))
                            .top(px(28.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(rmac_ui::text_px(qm::VALUE))
                            .line_height(px(21.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text())
                            .child(value.clone()),
                    )
                    .child(
                        div()
                            .id("spotlight-answer-copy")
                            .absolute()
                            .right(px(qm::COPY_RIGHT - 1.0))
                            .top(px((qm::CALCULATION - qm::COPY) / 2.0 - 1.0))
                            .size(px(qm::COPY))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(white(0.18))
                            .hover(|hover| hover.bg(white(0.26)))
                            .child(
                                svg()
                                    .path("spotlight/copy.svg")
                                    .w(px(qm::COPY_GLYPH.0))
                                    .h(px(qm::COPY_GLYPH.1))
                                    // (239,239,239)
                                    .text_color(if is_dark() {
                                        Hsla::from(gpui::rgb(0xefefef))
                                    } else {
                                        mac::text()
                                    }),
                            )
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    value.clone(),
                                ));
                            })),
                    )
                    .when_some(row.detail.clone(), |card, source| {
                        card.child(
                            div()
                                .absolute()
                                .right(px(19.5))
                                .bottom(px(13.0))
                                .text_size(rmac_ui::text_px(qm::SOURCE))
                                .font_weight(mac::SEMIBOLD)
                                // (141,142,143)
                                .text_color(if is_dark() {
                                    Hsla::from(gpui::rgb(0x8d8e8f))
                                } else {
                                    mac::text_tertiary()
                                })
                                .child(source),
                        )
                    })
                    .into_any_element()
            }
        }
    }
}

fn card_height(row: &Row) -> f32 {
    match row.category {
        Category::Clock => qm::CLOCK,
        Category::Dictionary => qm::DEFINITION,
        _ if row.detail.is_some() => qm::CURRENCY,
        _ => qm::CALCULATION,
    }
}

/// The completion for the selected row (macOS 26.2): the rest of the top
/// hit's name flush with the typed text, "—  Open" for an exact top hit,
/// "=  84" for a calculation, and "—  <name>" for anything else.
fn completion_for(query: &str, row: &Row, index: usize) -> Option<Completion> {
    if row.category == Category::Calculator {
        return Some(Completion::Plate {
            mark: "=",
            text: row.title.clone(),
        });
    }
    if index == 0 {
        match completion::inline_completion(query, &row.title) {
            Some("") => {
                return Some(Completion::Plate {
                    mark: "—",
                    text: row.primary_label.to_owned(),
                })
            }
            Some(suffix) => {
                return Some(Completion::Flush(completion::completion_label(
                    suffix,
                    row.primary_label,
                )))
            }
            None => {}
        }
    }
    Some(Completion::Plate {
        mark: "—",
        text: row.title.clone(),
    })
}
