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

/// Every match of `needle` anywhere in `term`'s buffer (scrollback and
/// screen), top to bottom — the same whole-history scan `find_step` already
/// did inline, shared with Edit ▸ Find ▸ Select All.
fn all_matches<T>(term: &alacritty_terminal::term::Term<T>, needle: &[char]) -> Vec<FindMatch> {
    let grid = term.grid();
    let history = grid.history_size();
    let rows = grid.screen_lines();
    let cols = grid.columns();
    let mut matches = Vec::new();
    for line in -(history as i32)..rows as i32 {
        let cells = row_cells(&grid[Line(line)], cols);
        for (start, end) in find::match_columns(&cells, needle) {
            matches.push(FindMatch { line, start, end });
        }
    }
    matches
}

impl TerminalView {
    /// Move to the next or previous Find match anywhere in the scrollback,
    /// scroll it into view and select it (⌘G, ⇧⌘G, Return, Shift-Return).
    /// The whole history is scanned once per step, never while idle.
    pub(crate) fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) {
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
        let matches = all_matches(&term, &needle);
        let history = term.grid().history_size();
        let rows = term.grid().screen_lines();
        let display_offset = term.grid().display_offset();
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
        tab.ui.selected_matches.clear();
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

    /// Edit ▸ Find ▸ Select All / Select All in Selection (TERM-23):
    /// selects every match at once (`ui.selected_matches`), instead of
    /// stepping to just one the way ⌘G/⇧⌘G does. "in Selection" narrows
    /// the whole-buffer scan to matches that fall entirely inside the
    /// current single-range selection, which this then replaces.
    pub(crate) fn find_select_all(&mut self, in_selection_only: bool, cx: &mut Context<Self>) {
        if self.modal_open() {
            return;
        }
        self.capture_active_search_query(cx);
        let query = self.tabs[self.active].ui.search_query.clone();
        if query.is_empty() {
            return;
        }
        let needle = find::fold_query(&query);
        let bounds = in_selection_only
            .then(|| self.tabs[self.active].ui.selection)
            .flatten();
        if in_selection_only && bounds.is_none() {
            return;
        }
        let tab = &mut self.tabs[self.active];
        let Ok(term) = tab.term.lock() else {
            self.operation_error = Some(SessionWriteError::State.to_string().into());
            cx.notify();
            return;
        };
        let mut matches = all_matches(&term, &needle);
        drop(term);
        if let Some(selection) = bounds {
            matches.retain(|found| {
                selection.contains(found.line, found.start)
                    && selection.contains(found.line, found.end.saturating_sub(1))
            });
        }
        tab.ui.selection = None;
        tab.ui.find_current = None;
        tab.ui.find_status = Some((
            query,
            FindStatus {
                current: matches.len().min(1),
                total: matches.len(),
            },
        ));
        tab.ui.selected_matches = matches;
        cx.notify();
    }

    /// The Find bar's "3 of 12" or "Not found", for the query on screen only.
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

    /// Settings ▸ Text ▸ Cursor style: the focused-window Underline and
    /// Vertical Bar shapes, drawn as a small overlay instead of the Block
    /// shape's inline cell-colour swap in `render_rows` (`show_cursor`
    /// already covers DECTCEM, scrollback and the running-shell checks, and
    /// `blink_visible` the blink phase; both are mirrored here).
    pub(super) fn render_active_cursor_overlay(&self) -> Option<Div> {
        if self.cursor_style == CursorStyle::Block
            || !self.window_active
            || !self.blink_visible
            || !self.tabs[self.active].accepts_input()
        {
            return None;
        }
        {
            let term = self.tabs[self.active].term.lock().ok()?;
            if term.grid().display_offset() != 0 || !term.mode().contains(TermMode::SHOW_CURSOR) {
                return None;
            }
        }
        let (row, column) = self.active_cursor_viewport_cell()?;
        let left = PAD_X + column as f32 * self.cell_w;
        let top = PAD_TOP + row as f32 * self.line_h;
        let shape = match self.cursor_style {
            CursorStyle::Block => return None,
            CursorStyle::Underline => div()
                .absolute()
                .left(px(left))
                .top(px(top + self.line_h - 2.0))
                .w(px(self.cell_w))
                .h(px(2.0)),
            CursorStyle::Bar => div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(2.0))
                .h(px(self.line_h)),
        };
        Some(shape.bg(hsla(active().cursor)))
    }

    pub(super) fn render_rows(&self, query: &str) -> Vec<gpui::AnyElement> {
        self.render_rows_at(query, None, true)
    }

    /// `forced_offset` renders at an explicit display offset instead of the
    /// shared `Term::grid().display_offset()` — View ▸ Split Pane's second
    /// viewport reads the exact same grid at its own, independent scroll
    /// position. `allow_cursor` is false for that secondary viewport: the
    /// live cursor only ever belongs to the one primary pane.
    pub(super) fn render_rows_at(
        &self,
        query: &str,
        forced_offset: Option<i32>,
        allow_cursor: bool,
    ) -> Vec<gpui::AnyElement> {
        let Ok(mut term) = self.tabs[self.active].term.lock() else {
            return Vec::new();
        };
        // View ▸ Show/Hide Alternative Screen (TERM-23): a manual peek at
        // the primary screen underneath, done by actually swapping the
        // grid the same way the program's own DECSET 1049 would — then
        // swapping straight back below before anything else observes the
        // flip (`term.mode()`, selection, …) or the PTY reader (blocked on
        // this same lock) can write to the wrong buffer. There is no
        // early return between the two swaps.
        let peeking_primary_screen =
            self.viewing_primary_while_alt_screen && term.mode().contains(TermMode::ALT_SCREEN);
        if peeking_primary_screen {
            term.swap_alt();
        }
        let grid = term.grid();
        let offset = forced_offset.unwrap_or(grid.display_offset() as i32);
        let history_size = grid.history_size();
        let cursor = grid.cursor.point;
        // Full-screen programs hide the cursor (DECTCEM) while they draw.
        let show_cursor = allow_cursor
            && offset == 0
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
            // Edit ▸ Find ▸ Select All/Select All in Selection: every
            // selected match on this row reads as selected text, same as
            // `current_match` below but for the whole set at once.
            let selected_matches_here: Vec<(usize, usize)> = ui
                .selected_matches
                .iter()
                .filter(|found| found.line == line_index)
                .map(|found| (found.start, found.end))
                .collect();
            let selected_match_columns = find::covered_columns(&selected_matches_here, self.cols);
            let mut spans = Vec::new();
            let mut run = String::new();
            let mut run_style: Option<Style> = None;

            for column in 0..self.cols {
                let cell = &row[Column(column)];
                let flags = cell.flags;
                let mut foreground = if self.display_ansi_colours {
                    conv(if self.bright_bold_text && flags.contains(Flags::BOLD) {
                        effective_bold_foreground(cell.fg, active().fg)
                    } else {
                        cell.fg
                    })
                } else {
                    hsla(active().fg)
                };
                let mut background = if self.display_ansi_colours {
                    conv(cell.bg)
                } else {
                    hsla(active().bg)
                };

                if flags.contains(Flags::DIM) {
                    foreground.a *= 0.65;
                }
                if show_cursor
                    && self.window_active
                    && self.cursor_style == CursorStyle::Block
                    && self.blink_visible
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
                if current_match.is_some_and(|found| (found.start..found.end).contains(&column))
                    || selected_match_columns.get(column).copied().unwrap_or(false)
                {
                    background = hsla(active().selection);
                } else if matched.get(column).copied().unwrap_or(false) {
                    background = hsla(FIND_HL);
                    foreground = hsla(active().bg);
                }

                let style = Style {
                    fg: foreground,
                    bg: background,
                    bold: self.use_bold_fonts && flags.intersects(Flags::BOLD | Flags::DIM_BOLD),
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

            // View ▸ Show Marks (TERM-23): a small gutter dot on any row
            // with a mark or bookmark, at the absolute line coordinate
            // marks are recorded in (`history_size` shifts alacritty's
            // signed `Line` into that always-non-negative numbering).
            let marked = self.show_marks
                && usize::try_from(i64::from(history_size as i32) + i64::from(line_index))
                    .is_ok_and(|absolute| {
                        self.tabs[self.active].has_mark_at_absolute_line(absolute)
                    });
            rows.push(
                div()
                    .relative()
                    .flex()
                    .h(px(self.line_h))
                    .children(spans)
                    .when(marked, |row| {
                        row.child(
                            div()
                                .absolute()
                                .left_0()
                                .top(px(self.line_h / 2.0 - 2.0))
                                .w(px(4.0))
                                .h(px(4.0))
                                .rounded(px(rmac_ui::mac::radius_pill()))
                                .bg(hsla(active().cursor)),
                        )
                    })
                    .into_any_element(),
            );
        }
        if peeking_primary_screen {
            term.swap_alt();
        }
        rows
    }

    /// View ▸ Split Pane (⌘D): a second, clipped viewport under the
    /// primary one, showing this same session's grid at its own scroll
    /// offset. Plain content only — no selection, IME or mouse-report
    /// routing there, unlike the primary pane, which keeps all of that
    /// unchanged.
    pub(super) fn render_split_pane(
        &self,
        query: &str,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let offset = self.tabs[self.active].ui.split_offset?;
        let rows = self.render_rows_at(query, Some(offset), false);
        Some(
            div()
                .id("terminal-split-pane")
                .role(Role::Group)
                .aria_label("Split Pane")
                .flex_1()
                .min_h(px(0.0))
                .overflow_hidden()
                .border_t_1()
                .border_color(rmac_ui::mac::separator())
                .bg(hsla(active().bg))
                .font_family(rmac_ui::MONO_FONT)
                .text_size(px(self.font_size))
                .pl(px(PAD_X))
                .pr(px(PAD_X))
                .pt(px(PAD_TOP))
                .pb(px(PAD_BOTTOM))
                .v_flex()
                .children(rows)
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                    let delta_y = match event.delta {
                        ScrollDelta::Lines(point) => point.y,
                        ScrollDelta::Pixels(point) => f32::from(point.y) / this.line_h,
                    };
                    this.scroll_split_pane(delta_y.trunc() as i32, cx);
                })),
        )
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

/// SGR 1 can use the bright half of the profile's ANSI palette while bold
/// type remains a separate preference. True RGB colours stay exact.
fn bright_variant(color: Color) -> Color {
    use NamedColor::*;
    match color {
        Color::Named(name) => Color::Named(match name {
            Black => BrightBlack,
            Red => BrightRed,
            Green => BrightGreen,
            Yellow => BrightYellow,
            Blue => BrightBlue,
            Magenta => BrightMagenta,
            Cyan => BrightCyan,
            White => BrightWhite,
            _ => name,
        }),
        Color::Indexed(index @ 0..=7) => Color::Indexed(index + 8),
        _ => color,
    }
}

/// Settings ▸ Text ▸ "Use bright colours for bold text", for one cell's bold
/// foreground: the profile's 8 ANSI colours brighten through their explicit
/// bright half (`bright_variant`); plain (default-coloured) bold text has
/// no such pair, so it lightens the profile's own text colour instead
/// (`profiles::brighten`) — the Settings window's "Bold Text" swatch shows
/// exactly this computed colour.
fn effective_bold_foreground(fg: Color, profile_fg: u32) -> Color {
    match fg {
        Color::Named(NamedColor::Foreground) => {
            let (r, g, b) = split(profiles::brighten(profile_fg));
            Color::Spec(vte::ansi::Rgb { r, g, b })
        }
        other => bright_variant(other),
    }
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

#[cfg(test)]
mod colour_preference_tests {
    use super::*;

    #[test]
    fn bold_brightens_palette_colours_without_changing_true_rgb() {
        assert_eq!(
            bright_variant(Color::Named(NamedColor::Red)),
            Color::Named(NamedColor::BrightRed),
        );
        assert_eq!(bright_variant(Color::Indexed(2)), Color::Indexed(10));
        assert_eq!(bright_variant(Color::Indexed(17)), Color::Indexed(17));
        let rgb = Color::Spec(vte::ansi::Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(bright_variant(rgb), rgb);
    }

    #[test]
    fn bold_default_coloured_text_brightens_the_profile_text_colour() {
        // Basic's measured fg, #000000: NamedColor::Foreground has no
        // bright half of its own, so it should lighten toward white
        // (profiles::brighten) rather than pass through unchanged.
        assert_eq!(
            effective_bold_foreground(Color::Named(NamedColor::Foreground), 0x000000),
            Color::Spec(vte::ansi::Rgb {
                r: 0x59,
                g: 0x59,
                b: 0x59
            }),
        );
        // An explicitly ANSI-coloured bold cell still uses the palette's
        // own bright half, unaffected by the profile's text colour.
        assert_eq!(
            effective_bold_foreground(Color::Named(NamedColor::Red), 0x000000),
            Color::Named(NamedColor::BrightRed),
        );
    }
}
