//! Terminal grid, style-run, search-highlight, cursor, and IME projection.

use super::*;
use alacritty_terminal::grid::Row;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};

/// The row's characters as Find sees them: one entry per character, with the
/// column it starts in. The spacer column after a wide character is skipped
/// so a match can span it.
fn row_cells(row: &Row<Cell>, cols: usize) -> Vec<find::CellText> {
    let mut cells = Vec::with_capacity(cols);
    for column in 0..cols {
        let cell = &row[Column(column)];
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        cells.push(find::CellText {
            column,
            width: if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            },
            character: if cell.c == '\0' { ' ' } else { cell.c },
        });
    }
    cells
}

impl TerminalView {
    /// Move to the next or previous Find match anywhere in the scrollback,
    /// scroll it into view and select it (⌘G, ⇧⌘G, Return, Shift-Return).
    /// The whole history is scanned once per step, never while idle.
    pub(super) fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        self.capture_active_search_query(cx);
        let query = self.tabs[self.active].ui.search_query.clone();
        if query.is_empty() {
            return;
        }
        let needle = find::fold_query(&query);
        let tab = &mut self.tabs[self.active];
        let Ok(mut term) = tab.term.lock() else {
            self.operation_error = Some(SessionWriteError::State.to_string().into());
            cx.notify();
            return;
        };
        let grid = term.grid();
        let history = grid.history_size();
        let rows = grid.screen_lines();
        let cols = grid.columns();
        let display_offset = grid.display_offset();
        let mut matches = Vec::new();
        for line in -(history as i32)..rows as i32 {
            let cells = row_cells(&grid[Line(line)], cols);
            for (start, end) in find::match_columns(&cells, &needle) {
                matches.push(FindMatch { line, start, end });
            }
        }
        let current = tab
            .ui
            .find_status
            .as_ref()
            .filter(|(searched, _)| *searched == query)
            .and(tab.ui.find_current);
        let Some(index) = find::step(&matches, current, forward) else {
            drop(term);
            tab.ui.find_current = None;
            tab.ui.find_status = Some((query, FindStatus::default()));
            cx.notify();
            return;
        };
        let found = matches[index];
        let offset = find::display_offset_for(found.line, rows, history, display_offset);
        if offset != display_offset {
            term.scroll_display(Scroll::Bottom);
            term.scroll_display(Scroll::Delta(i32::try_from(offset).unwrap_or(i32::MAX)));
        }
        drop(term);
        tab.ui.selection = Some(Selection {
            anchor: (found.line, found.start),
            head: (found.line, found.end.saturating_sub(1)),
        });
        tab.ui.find_current = Some(found);
        tab.ui.find_status = Some((
            query,
            FindStatus {
                current: index + 1,
                total: matches.len(),
            },
        ));
        cx.notify();
    }

    /// The Find bar's "3 of 12" or "Not Found", for the query on screen only.
    pub(super) fn find_status_label(&self) -> Option<String> {
        let ui = &self.tabs[self.active].ui;
        ui.find_status
            .as_ref()
            .filter(|(searched, _)| *searched == ui.search_query)
            .map(|(_, status)| status.label())
    }

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
        let needle = find::fold_query(query);
        let ui = &self.tabs[self.active].ui;
        let current_match = ui
            .find_status
            .as_ref()
            .filter(|(searched, _)| !needle.is_empty() && *searched == ui.search_query)
            .and(ui.find_current);

        let mut rows = Vec::with_capacity(self.rows);
        for viewport_row in 0..self.rows as i32 {
            let line_index = viewport_row - offset;
            let row = &grid[Line(line_index)];
            let matched = if needle.is_empty() {
                Vec::new()
            } else {
                find::covered_columns(
                    &find::match_columns(&row_cells(row, self.cols), &needle),
                    self.cols,
                )
            };
            let current_match = current_match.filter(|found| found.line == line_index);
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
                // The match ⌘G moved to reads as selected text; the others
                // keep the yellow highlight.
                if current_match.is_some_and(|found| (found.start..found.end).contains(&column)) {
                    background = hsla(active().selection);
                } else if matched.get(column).copied().unwrap_or(false) {
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
