//! Terminal grid, style-run, search-highlight, cursor, and IME projection.

use super::*;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;

impl TerminalView {
    pub(super) fn render_ime_preedit(&self) -> Option<Div> {
        let composition = self.ime.as_ref()?;
        if composition.session_id != self.tabs[self.active].id || composition.buffer.text.is_empty()
        {
            return None;
        }
        let (row, column) = self.active_cursor_viewport_cell()?;
        let remaining_columns = self.cols.saturating_sub(column).max(1);
        Some(
            div()
                .absolute()
                .left(px(BODY_PAD + column as f32 * self.cell_w))
                .top(px(BODY_PAD + row as f32 * self.line_h))
                .w(px(remaining_columns as f32 * self.cell_w))
                .max_h(px(self.rows.saturating_sub(row).max(1) as f32 * self.line_h))
                .overflow_hidden()
                .bg(hsla(active().bg))
                .text_color(hsla(active().fg))
                .line_height(px(self.line_h))
                .underline()
                .child(composition.buffer.text.clone()),
        )
    }

    pub(super) fn render_rows(&self, query: &str) -> Vec<gpui::AnyElement> {
        let Ok(term) = self.tabs[self.active].term.lock() else {
            return Vec::new();
        };
        let grid = term.grid();
        let offset = grid.display_offset() as i32;
        let cursor = grid.cursor.point;
        let show_cursor = offset == 0 && self.tabs[self.active].accepts_input();
        let cursor_line = cursor.line.0;
        let cursor_column = cursor.column.0;

        let mut rows = Vec::with_capacity(self.rows);
        for viewport_row in 0..self.rows as i32 {
            let line_index = viewport_row - offset;
            let row = &grid[Line(line_index)];
            let matched = if query.is_empty() {
                Vec::new()
            } else {
                let text: String = (0..self.cols)
                    .map(|column| {
                        let character = row[Column(column)].c;
                        if character == '\0' {
                            ' '
                        } else {
                            character
                        }
                    })
                    .collect::<String>()
                    .to_lowercase();
                let mut matched = vec![false; self.cols];
                let query_length = query.chars().count().max(1);
                let mut start = 0;
                while let Some(position) = text.get(start..).and_then(|text| text.find(query)) {
                    let match_start = start + position;
                    for cell in matched
                        .iter_mut()
                        .take((match_start + query_length).min(self.cols))
                        .skip(match_start)
                    {
                        *cell = true;
                    }
                    start = match_start + query_length;
                    if start >= text.len() {
                        break;
                    }
                }
                matched
            };
            let mut spans = Vec::new();
            let mut run = String::new();
            let mut run_style: Option<Style> = None;

            for column in 0..self.cols {
                let cell = &row[Column(column)];
                let flags = cell.flags;
                let mut foreground = conv(cell.fg);
                let mut background = conv(cell.bg);

                if flags.contains(Flags::DIM) {
                    foreground.a *= 0.65;
                }
                if show_cursor && line_index == cursor_line && column == cursor_column {
                    std::mem::swap(&mut foreground, &mut background);
                }
                if let Some(selection) = &self.tabs[self.active].ui.selection {
                    if !selection.is_empty() && selection.contains(line_index, column) {
                        background = hsla(active().selection);
                    }
                }
                if matched.get(column).copied().unwrap_or(false) {
                    background = hsla(FIND_HL);
                    foreground = hsla(active().bg);
                }

                let style = Style {
                    fg: foreground,
                    bg: background,
                    bold: flags.intersects(Flags::BOLD | Flags::DIM_BOLD),
                    italic: flags.contains(Flags::ITALIC),
                    underline: flags.intersects(Flags::ALL_UNDERLINES)
                        || cell.hyperlink().is_some(),
                    strike: flags.contains(Flags::STRIKEOUT),
                };
                let character = if cell.c == '\0' { ' ' } else { cell.c };

                match run_style {
                    None => run_style = Some(style),
                    Some(previous) if previous != style => {
                        spans.push(span(&run, previous));
                        run.clear();
                        run_style = Some(style);
                    }
                    _ => {}
                }
                run.push(character);
            }
            if let Some(style) = run_style {
                if !run.is_empty() {
                    spans.push(span(&run, style));
                }
            }

            rows.push(
                div()
                    .flex()
                    .h(px(self.line_h))
                    .children(spans)
                    .into_any_element(),
            );
        }
        rows
    }
}
