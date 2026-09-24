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
                .left(px(PAD_X + column as f32 * self.cell_w))
                .top(px(PAD_TOP + row as f32 * self.line_h))
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

    /// Behind another window Terminal draws the cursor as an outline in the
    /// cursor colour instead of a filled block.
    pub(super) fn render_inactive_cursor(&self) -> Option<Div> {
        if self.window_active || !self.tabs[self.active].accepts_input() {
            return None;
        }
        {
            let term = self.tabs[self.active].term.lock().ok()?;
            if term.grid().display_offset() != 0 || !term.mode().contains(TermMode::SHOW_CURSOR) {
                return None;
            }
        }
        let (row, column) = self.active_cursor_viewport_cell()?;
        Some(
            div()
                .absolute()
                .left(px(PAD_X + column as f32 * self.cell_w))
                .top(px(PAD_TOP + row as f32 * self.line_h))
                .w(px(self.cell_w))
                .h(px(self.line_h))
                .border_1()
                .border_color(hsla(active().cursor)),
        )
    }

    pub(super) fn render_rows(&self, query: &str) -> Vec<gpui::AnyElement> {
        let Ok(term) = self.tabs[self.active].term.lock() else {
            return Vec::new();
        };
        let grid = term.grid();
        let offset = grid.display_offset() as i32;
        let cursor = grid.cursor.point;
        // Full-screen programs hide the cursor (DECTCEM) while they draw.
        let show_cursor = offset == 0
            && self.tabs[self.active].accepts_input()
            && term.mode().contains(TermMode::SHOW_CURSOR);
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
                if show_cursor
                    && self.window_active
                    && line_index == cursor_line
                    && column == cursor_column
                {
                    // Terminal fills the block cursor with the profile's
                    // cursor colour (Basic dark: #9C9D9D, measured) and
                    // draws the character under it in the background colour.
                    background = hsla(active().cursor);
                    foreground = hsla(active().bg);
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

fn span(text: &str, style: Style) -> gpui::AnyElement {
    let mut element = div()
        .text_color(style.fg)
        .bg(style.bg)
        .child(text.to_string());
    if style.bold {
        element = element.font_weight(FontWeight::BOLD);
    }
    if style.italic {
        element = element.italic();
    }
    if style.underline {
        element = element.underline();
    }
    if style.strike {
        element = element.line_through();
    }
    element.into_any_element()
}

/// Map a terminal color to RGB.
fn conv(color: Color) -> Hsla {
    let (red, green, blue) = match color {
        Color::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        Color::Named(named) => named_color(named),
        Color::Indexed(index) => indexed_color(index),
    };
    gpui::rgb(((red as u32) << 16) | ((green as u32) << 8) | (blue as u32)).into()
}

fn named_color(color: NamedColor) -> (u8, u8, u8) {
    use NamedColor::*;
    let profile = active();
    let ansi = |index: usize| split(profile.ansi[index]);
    match color {
        Background => split(profile.bg),
        Foreground => split(profile.fg),
        Cursor => split(profile.cursor),
        Black => ansi(0),
        Red => ansi(1),
        Green => ansi(2),
        Yellow => ansi(3),
        Blue => ansi(4),
        Magenta => ansi(5),
        Cyan => ansi(6),
        White => ansi(7),
        BrightBlack => ansi(8),
        BrightRed => ansi(9),
        BrightGreen => ansi(10),
        BrightYellow => ansi(11),
        BrightBlue => ansi(12),
        BrightMagenta => ansi(13),
        BrightCyan => ansi(14),
        BrightWhite => ansi(15),
        _ => split(profile.fg),
    }
}

fn indexed_color(index: u8) -> (u8, u8, u8) {
    match index {
        0..=15 => split(active().ansi[index as usize]),
        16..=231 => {
            let index = index - 16;
            let component = |value: u8| -> u8 {
                if value == 0 {
                    0
                } else {
                    55 + 40 * value
                }
            };
            (
                component(index / 36),
                component((index % 36) / 6),
                component(index % 6),
            )
        }
        _ => {
            let value = 8 + (index - 232) * 10;
            (value, value, value)
        }
    }
}

fn split(hex: u32) -> (u8, u8, u8) {
    (
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}
