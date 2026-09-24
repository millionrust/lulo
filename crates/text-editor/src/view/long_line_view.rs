//! Read-only view for documents with lines too long for the editable view.
//!
//! See [`crate::long_lines`] for why these documents leave the editor. The
//! text is held once, rows are wrapped by character count in the window's
//! monospace column width, and a uniform list lays out only the rows on
//! screen, so memory stays near the document size and idle frames do no
//! work proportional to it.

use std::rc::Rc;

use gpui::{
    div, font, px, uniform_list, Context, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollStrategy, SharedString, Styled as _, StyledText, TextRun,
    UniformListScrollHandle, Window,
};
use rmac_ui::mac;

use super::EditorView;
use crate::long_lines;

pub(super) struct LongLineDocument {
    pub(super) text: SharedString,
    pub(super) longest_line: usize,
    /// Row starts for `columns`; recomputed only when the column count
    /// changes (window width or font size).
    rows: Rc<Vec<u32>>,
    columns: usize,
    scroll: UniformListScrollHandle,
}

impl LongLineDocument {
    pub(super) fn new(text: String, longest_line: usize) -> Self {
        Self {
            text: text.into(),
            longest_line,
            rows: Rc::new(Vec::new()),
            columns: 0,
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn ensure_columns(&mut self, columns: usize) {
        if self.columns != columns || self.rows.is_empty() {
            self.columns = columns;
            self.rows = Rc::new(long_lines::wrap_rows(&self.text, columns));
        }
    }

    /// Scroll so byte `offset` is on screen (Find Next / Previous).
    pub(super) fn reveal_offset(&self, offset: usize) {
        if !self.rows.is_empty() {
            self.scroll.scroll_to_item(
                long_lines::row_for_offset(&self.rows, offset),
                ScrollStrategy::Center,
            );
        }
    }
}

/// "12.3 MB" for the banner, in the decimal units Finder uses.
fn size_label(bytes: usize) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        format!("{} KB", bytes.div_ceil(1000))
    }
}

impl EditorView {
    pub(super) fn render_long_line_view(
        &mut self,
        font_family: &'static str,
        font_size: f32,
        line_height: f32,
        inset_x: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let text_font = font(font_family);
        let font_id = window.text_system().resolve_font(&text_font);
        let advance = window
            .text_system()
            .advance(font_id, px(font_size), '0')
            .map(|size| f32::from(size.width))
            .unwrap_or(font_size * 0.6)
            .max(1.0);
        // Leave room for the scroll bar at the trailing edge.
        let usable = f32::from(window.bounds().size.width) - inset_x * 2.0 - 8.0;
        let columns = ((usable / advance).floor() as usize).max(8);
        let needle_len = self.find_input.read(cx).text().len();
        let highlight = self
            .find_open
            .then(|| self.matches.get(self.current))
            .flatten()
            .map(|&start| start..start + needle_len);

        let Some(document) = self.long_lines.as_mut() else {
            return div().into_any_element();
        };
        document.ensure_columns(columns);
        let text = document.text.clone();
        let rows = document.rows.clone();
        let banner = format!(
            "Read-only: this document has a line of {}. Text Editor shows it wrapped by \
             character so it stays responsive; use Save As to keep a copy.",
            size_label(document.longest_line)
        );
        let text_color = mac::text();
        let match_background: Hsla = mac::accent().opacity(0.35);

        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("long-line-banner")
                    .flex_none()
                    .px(px(inset_x))
                    .py(px(6.0))
                    .bg(mac::chrome())
                    .border_b_1()
                    .border_color(mac::separator())
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(banner),
            )
            .child(rmac_ui::uniform_list_scrollbar(
                div().flex_1().min_h(px(0.0)).flex().flex_col().child(
                    uniform_list("long-line-rows", rows.len(), move |range, _window, _cx| {
                        range
                            .map(|row| {
                                let bytes = long_lines::row_range(&text, &rows, row);
                                let line: SharedString = text[bytes.clone()].to_string().into();
                                let runs = row_runs(
                                    &line,
                                    bytes.start,
                                    highlight.clone(),
                                    &text_font,
                                    text_color,
                                    match_background,
                                );
                                div()
                                    .h(px(line_height))
                                    .px(px(inset_x))
                                    .font_family(font_family)
                                    .text_size(px(font_size))
                                    .line_height(px(line_height))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(StyledText::new(line).with_runs(runs))
                            })
                            .collect()
                    })
                    .track_scroll(&document.scroll)
                    .flex_1(),
                ),
                &document.scroll,
            ))
            .into_any_element()
    }
}

/// Text runs for one row, highlighting the part of the active Find match
/// that falls inside it.
fn row_runs(
    line: &str,
    row_start: usize,
    highlight: Option<std::ops::Range<usize>>,
    text_font: &gpui::Font,
    color: Hsla,
    match_background: Hsla,
) -> Vec<TextRun> {
    let run = |len: usize, background: Option<Hsla>| TextRun {
        len,
        font: text_font.clone(),
        color,
        background_color: background,
        underline: None,
        strikethrough: None,
    };
    let row_end = row_start + line.len();
    match highlight {
        Some(found) if found.start < row_end && found.end > row_start => {
            let start = found.start.max(row_start) - row_start;
            let end = found.end.min(row_end) - row_start;
            [
                (start, None),
                (end - start, Some(match_background)),
                (line.len() - end, None),
            ]
            .into_iter()
            .filter(|(len, _)| *len > 0)
            .map(|(len, background)| run(len, background))
            .collect()
        }
        _ if line.is_empty() => Vec::new(),
        _ => vec![run(line.len(), None)],
    }
}
