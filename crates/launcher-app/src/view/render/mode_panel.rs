//! The Actions and Clipboard panel, measured from macOS 26.2
//! (`design-lab/spotlight-actions.html` records every source value).

use super::*;
use crate::view::panel::{ClipboardState, PanelIcon, PanelMode, PanelRow};

mod panel_metrics {
    /// The panel ends at the measured 546 pt (y 190–736.5).
    pub(super) const MAX_HEIGHT: f32 = 546.0;
    pub(super) const RADIUS: f32 = 28.0;
    pub(super) const HEADER: f32 = 56.0;
    /// Mode glyph box (ink 24 × 27) from the panel's outer edge.
    pub(super) const GLYPH: f32 = 31.0;
    pub(super) const GLYPH_LEFT: f32 = 16.5;
    pub(super) const GLYPH_TOP: f32 = 12.5;
    pub(super) const RULE_INSET: f32 = 20.0;
    pub(super) const SECTION: f32 = 33.0;
    pub(super) const SECTION_TOP: f32 = 11.0;
    pub(super) const SECTION_TEXT: f32 = 14.0;
    pub(super) const ROW: f32 = 56.0;
    pub(super) const ICON: f32 = 32.0;
    pub(super) const ICON_LEFT: f32 = 20.0;
    pub(super) const TITLE_LEFT: f32 = 72.0;
    pub(super) const TITLE: f32 = 17.0;
    pub(super) const TITLE_LINE: f32 = 21.0;
    pub(super) const TITLE_TOP: f32 = 6.0;
    pub(super) const SUBTITLE: f32 = 14.5;
    pub(super) const SUBTITLE_LINE: f32 = 18.0;
    pub(super) const SUBTITLE_TOP: f32 = 28.0;
    pub(super) const LIST_PADDING: f32 = 10.0;
    pub(super) const PLATE_INSET: f32 = 11.0;
    /// S: the plate's corner.
    pub(super) const PLATE_RADIUS: f32 = 12.0;
    pub(super) const EMPTY: f32 = 92.0;
    pub(super) const EMPTY_TEXT: f32 = 17.0;
    /// First-use clipboard prompt (panel 446.5 tall).
    pub(super) const PROMPT: f32 = 389.5;
    pub(super) const PROMPT_TITLE_TOP: f32 = 112.0;
    pub(super) const PROMPT_BODY_TOP: f32 = 136.0;
    pub(super) const PROMPT_BODY_INSET: f32 = 94.0;
    pub(super) const PROMPT_BUTTONS_TOP: f32 = 209.0;
    pub(super) const BUTTON_WIDTH: f32 = 110.0;
    pub(super) const BUTTON_HEIGHT: f32 = 28.0;
    pub(super) const BUTTON_GAP: f32 = 8.0;
}
use panel_metrics as pm;

/// The measured (29,29,30) panel over a dark backdrop.
fn panel_fill() -> Hsla {
    if is_dark() {
        Hsla::from(gpui::rgba(0x1f1f21e6))
    } else {
        Hsla::from(gpui::rgba(0xf6f6f8e6))
    }
}

/// Header and section rules and the selected-row plate: white 15.5 %.
fn rule() -> Hsla {
    if is_dark() {
        gpui::hsla(0.0, 0.0, 1.0, 0.155)
    } else {
        gpui::hsla(0.0, 0.0, 0.0, 0.10)
    }
}

fn subtitle_color(selected: bool) -> Hsla {
    match (is_dark(), selected) {
        (true, false) => gpui::hsla(0.0, 0.0, 1.0, 0.28),
        (true, true) => gpui::hsla(0.0, 0.0, 1.0, 0.34),
        (false, _) => gpui::hsla(0.0, 0.0, 0.0, 0.45),
    }
}

fn rule_line() -> gpui::Div {
    div()
        .flex_none()
        .h(px(1.0))
        .mx(px(pm::RULE_INSET - metrics::RIM))
        .bg(rule())
}

impl LauncherView {
    fn panel_icon(icon: &PanelIcon, size: f32) -> AnyElement {
        match icon {
            PanelIcon::Image(path) => img(path.clone())
                .size(px(size))
                .rounded(px(size * 0.2))
                .flex_none()
                .into_any_element(),
            PanelIcon::Glyph(glyph) => div()
                .size(px(size))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(*glyph)
                        .size(px(size * 0.875))
                        .text_color(mac::text_tertiary()),
                )
                .into_any_element(),
        }
    }

    /// The header row: mode glyph, query or the mode's name as placeholder,
    /// the top row's inline completion, and its icon at the right end.
    fn panel_header(
        &self,
        mode: PanelMode,
        query: &str,
        rows: &[PanelRow],
        cx: &Context<Self>,
    ) -> AnyElement {
        let top = if query.is_empty() { None } else { rows.first() };
        let completion = top.map(
            |row| match completion::inline_completion(query, &row.title) {
                Some(suffix) => suffix.to_owned(),
                None => format!(" — {}", row.title),
            },
        );
        let trailing = if top.is_some() {
            metrics::TOP_HIT_RIGHT
        } else {
            metrics::TEXT_TRAIL
        };
        div()
            .relative()
            .flex_none()
            .h(px(pm::HEADER - metrics::RIM))
            .flex()
            .items_center()
            .pl(px(metrics::TEXT_LEFT - metrics::RIM))
            .pr(px(trailing - metrics::RIM))
            .child(
                svg()
                    .path(mode.glyph())
                    .absolute()
                    .left(px(pm::GLYPH_LEFT - metrics::RIM))
                    .top(px(pm::GLYPH_TOP - metrics::RIM))
                    .size(px(pm::GLYPH))
                    .text_color(mac::text_tertiary()),
            )
            .child(self.query_field(
                query,
                mode.name(),
                completion.map(Completion::Flush),
                false,
                cx,
            ))
            .when_some(top, |header, row| {
                header.child(
                    div()
                        .flex_none()
                        .ml(px(8.0))
                        .child(Self::panel_icon(&row.icon, metrics::TOP_HIT_ICON)),
                )
            })
            .into_any_element()
    }

    fn panel_row(
        &self,
        row: &PanelRow,
        index: usize,
        selected: bool,
        plated: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        // Rows start inside the rim, or at the plate's inset once filtered.
        let start = if plated {
            pm::PLATE_INSET
        } else {
            metrics::RIM
        };
        div()
            .id(SharedString::from(format!("spotlight-panel-row-{index}")))
            .relative()
            .flex_none()
            .h(px(pm::ROW))
            .when(plated, |item| {
                item.mx(px(pm::PLATE_INSET - metrics::RIM))
                    .rounded(px(pm::PLATE_RADIUS))
            })
            .when(selected, |item| item.bg(rule()))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .left(px(pm::ICON_LEFT - start))
                    .top(px((pm::ROW - pm::ICON) / 2.0))
                    .child(Self::panel_icon(&row.icon, pm::ICON)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(pm::TITLE_LEFT - start))
                    .right(px(pm::RULE_INSET))
                    .top(px(pm::TITLE_TOP))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(rmac_ui::text_px(pm::TITLE))
                    .line_height(px(pm::TITLE_LINE))
                    .text_color(mac::text())
                    .child(row.title.clone()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(pm::TITLE_LEFT - start))
                    .right(px(pm::RULE_INSET))
                    .top(px(pm::SUBTITLE_TOP))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(rmac_ui::text_px(pm::SUBTITLE))
                    .line_height(px(pm::SUBTITLE_LINE))
                    .text_color(subtitle_color(selected))
                    .child(row.subtitle.clone()),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_panel_row(index, window, cx);
            }))
            .into_any_element()
    }

    fn panel_section(label: &str) -> AnyElement {
        div()
            .flex_none()
            .v_flex()
            .child(
                div()
                    .h(px(pm::SECTION))
                    .pt(px(pm::SECTION_TOP))
                    .pl(px(pm::RULE_INSET - metrics::RIM))
                    .text_size(rmac_ui::text_px(pm::SECTION_TEXT))
                    .line_height(px(18.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text())
                    .child(label.to_owned()),
            )
            .child(rule_line())
            .into_any_element()
    }

    fn panel_message(message: &'static str) -> AnyElement {
        div()
            .flex_none()
            .h(px(pm::EMPTY))
            .flex()
            .items_center()
            .justify_center()
            .text_size(rmac_ui::text_px(pm::EMPTY_TEXT))
            .font_weight(mac::SEMIBOLD)
            .text_color(mac::text())
            .child(message)
            .into_any_element()
    }

    fn clipboard_prompt(&self, cx: &Context<Self>) -> AnyElement {
        let button = |id: &'static str, label: &'static str, fill: Hsla, allow: bool| {
            div()
                .id(id)
                .w(px(pm::BUTTON_WIDTH))
                .h(px(pm::BUTTON_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(pm::BUTTON_HEIGHT / 2.0))
                .bg(fill)
                .cursor_pointer()
                .text_size(rmac_ui::text_px(13.0))
                .font_weight(mac::MEDIUM)
                .text_color(gpui::white())
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.answer_clipboard_prompt(allow, window, cx);
                }))
        };
        div()
            .relative()
            .flex_none()
            .h(px(pm::PROMPT))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(pm::PROMPT_TITLE_TOP))
                    .text_center()
                    .text_size(rmac_ui::text_px(17.0))
                    .line_height(px(22.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text())
                    .child("Allow search results from clipboard"),
            )
            .child(
                // The Mac's last sentence points at a Clipboard switch in
                // Spotlight Settings; rmac has no such switch yet, so the
                // sentence is left out rather than promise one.
                div()
                    .absolute()
                    .left(px(pm::PROMPT_BODY_INSET))
                    .right(px(pm::PROMPT_BODY_INSET))
                    .top(px(pm::PROMPT_BODY_TOP))
                    .text_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .line_height(px(16.0))
                    .text_color(mac::text_tertiary())
                    .child(
                        "Spotlight can search and display items you’ve copied to your \
                         clipboard. Personal and sensitive information may appear in \
                         search results.",
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(pm::PROMPT_BUTTONS_TOP))
                    .flex()
                    .justify_center()
                    .gap(px(pm::BUTTON_GAP))
                    .child(button(
                        "clipboard-not-now",
                        "Not Now",
                        Hsla::from(gpui::rgb(0x343537)),
                        false,
                    ))
                    .child(button(
                        "clipboard-allow",
                        "Allow",
                        Hsla::from(gpui::rgb(0x3478f6)),
                        true,
                    )),
            )
            .into_any_element()
    }

    fn panel_list(
        &self,
        rows: &[PanelRow],
        query: &str,
        selected: usize,
        navigated: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let sectioned = query.is_empty() && rows.iter().any(|row| !row.section.is_empty());
        let mut children = Vec::new();
        let mut section: Option<&str> = None;
        for (index, row) in rows.iter().enumerate() {
            if sectioned && section != Some(row.section.as_str()) {
                section = Some(row.section.as_str());
                children.push(Self::panel_section(&row.section));
            }
            let highlight = index == selected && (!sectioned || navigated);
            children.push(self.panel_row(row, index, highlight, !sectioned, cx));
        }
        let list_height = pm::MAX_HEIGHT - pm::HEADER - 1.0 - metrics::RIM;
        div()
            .id(RESULTS_ID)
            .flex_none()
            .max_h(px(list_height))
            .overflow_y_scroll()
            .v_flex()
            .when(!sectioned, |list| list.py(px(pm::LIST_PADDING)))
            .children(children)
            .into_any_element()
    }

    pub(super) fn mode_panel(&self, query: &str, cx: &Context<Self>) -> AnyElement {
        let Some(panel) = self.panel.as_ref() else {
            return div().into_any_element();
        };
        let rows = self.panel_rows(cx);
        let body = match (panel.mode, &panel.clipboard) {
            (PanelMode::Clipboard, ClipboardState::Disabled) => Some(self.clipboard_prompt(cx)),
            (PanelMode::Clipboard, ClipboardState::Unavailable) => {
                Some(Self::panel_message("Clipboard History Is Unavailable"))
            }
            (PanelMode::Clipboard, ClipboardState::Loading) => None,
            _ if rows.is_empty() && !query.is_empty() => Some(Self::panel_message("No Results")),
            _ if rows.is_empty() => None,
            _ => Some(self.panel_list(&rows, query, panel.selected, panel.navigated, cx)),
        };
        glass(
            div()
                .w(px(metrics::GROUP_WIDTH))
                .max_h(px(pm::MAX_HEIGHT))
                .flex_none()
                .v_flex()
                .overflow_hidden(),
            pm::RADIUS,
        )
        .bg(panel_fill())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.press_inside = true),
        )
        .child(self.panel_header(panel.mode, query, &rows, cx))
        .child(rule_line())
        .children(body)
        .when_some(panel.error.clone(), |card, error| {
            card.child(
                div()
                    .flex_none()
                    .px(px(pm::RULE_INSET))
                    .pb(px(pm::LIST_PADDING))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::warning_text())
                    .child(error),
            )
        })
        .into_any_element()
    }
}
