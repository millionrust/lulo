use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Row};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term};
use vte::ansi::Processor;

pub(super) const MIN_COLS: usize = 20;
pub(super) const MIN_ROWS: usize = 5;
const MAX_COLS: usize = 500;
const MAX_ROWS: usize = 300;
pub(super) const SCROLLBACK_LINES: usize = 10_000;
const MAX_GRID_BASE_BYTES_PER_WINDOW: usize = 512 * 1024 * 1024;
/// `Vec` can retain almost twice the requested elements after amortized growth.
const MAX_ROW_CELL_CAPACITY_FACTOR: usize = 2;
/// Alacritty retains both the primary and alternate visible screen grids.
const VISIBLE_GRID_COPIES: usize = 2;
/// Conservative allocator metadata/alignment/size-class allowance per cell row.
const ROW_ALLOCATION_ALLOWANCE_BYTES: usize = 1024;
/// Reserve for retained/rounded primary and alternate outer Row storage.
const OUTER_ROW_STORAGE_ALLOWANCE_BYTES_PER_TAB: usize = 1024 * 1024;
const MAX_COMBINING_MARKS_PER_CELL: usize = 16;

/// Grid geometry handed to the terminal model and the PTY.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TermSize {
    pub(super) cols: usize,
    pub(super) lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

pub(super) fn terminal_config(scrollback_lines: usize) -> Config {
    Config {
        scrolling_history: scrollback_lines.min(SCROLLBACK_LINES),
        ..Config::default()
    }
}

fn bounded_cell_row_bytes() -> usize {
    MAX_COLS
        .saturating_mul(MAX_ROW_CELL_CAPACITY_FACTOR)
        .saturating_mul(std::mem::size_of::<Cell>())
        .saturating_add(ROW_ALLOCATION_ALLOWANCE_BYTES)
}

fn retained_row_slots_bytes_per_tab() -> usize {
    // Shrinking history drops each Row's cell allocation, but the outer
    // primary/alternate Vecs can retain their old Row-slot capacities. Budget
    // their lifetime maximum independently from the current history limit.
    SCROLLBACK_LINES
        .saturating_add(MAX_ROWS.saturating_mul(VISIBLE_GRID_COPIES))
        .saturating_mul(MAX_ROW_CELL_CAPACITY_FACTOR)
        .saturating_mul(std::mem::size_of::<Row<Cell>>())
        .saturating_add(OUTER_ROW_STORAGE_ALLOWANCE_BYTES_PER_TAB)
}

pub(super) fn scrollback_limit_for_tab_count(tab_count: usize) -> usize {
    if tab_count == 0 {
        return 0;
    }
    let limit = MAX_GRID_BASE_BYTES_PER_WINDOW
        .checked_div(tab_count)
        .unwrap_or(0)
        .saturating_sub(retained_row_slots_bytes_per_tab())
        .checked_div(bounded_cell_row_bytes())
        .unwrap_or(0)
        .saturating_sub(MAX_ROWS.saturating_mul(VISIBLE_GRID_COPIES))
        .min(SCROLLBACK_LINES);
    debug_assert!(
        bounded_grid_base_bytes(tab_count, limit) <= MAX_GRID_BASE_BYTES_PER_WINDOW,
        "scrollback history must stay inside the base-grid budget"
    );
    limit
}

fn bounded_grid_base_bytes(tab_count: usize, history_lines: usize) -> usize {
    let cell_rows = tab_count
        .saturating_mul(
            MAX_ROWS
                .saturating_mul(VISIBLE_GRID_COPIES)
                .saturating_add(history_lines),
        )
        .saturating_mul(bounded_cell_row_bytes());
    cell_rows.saturating_add(tab_count.saturating_mul(retained_row_slots_bytes_per_tab()))
}

fn cap_cursor_combining_marks<T: EventListener>(term: &mut Term<T>) {
    let (line, mut column, input_needs_wrap) = {
        let grid = term.grid();
        (
            grid.cursor.point.line,
            grid.cursor.point.column,
            grid.cursor.input_needs_wrap,
        )
    };
    if !input_needs_wrap {
        column.0 = column.0.saturating_sub(1);
    }
    if term.grid()[line][column]
        .flags
        .contains(Flags::WIDE_CHAR_SPACER)
    {
        column.0 = column.0.saturating_sub(1);
    }

    let cell = &term.grid()[line][column];
    let Some(zerowidth) = cell
        .zerowidth()
        .filter(|marks| marks.len() > MAX_COMBINING_MARKS_PER_CELL)
    else {
        return;
    };
    let retained = zerowidth[..MAX_COMBINING_MARKS_PER_CELL].to_vec();
    let mut bounded = Cell {
        c: cell.c,
        fg: cell.fg,
        bg: cell.bg,
        flags: cell.flags,
        extra: None,
    };
    bounded.set_underline_color(cell.underline_color());
    bounded.set_hyperlink(cell.hyperlink());
    for mark in retained {
        bounded.push_zerowidth(mark);
    }
    term.grid_mut()[line][column] = bounded;
}

pub(super) fn advance_filtered_output<T: EventListener>(
    parser: &mut Processor,
    term: &mut Term<T>,
    bytes: &[u8],
) {
    let mut segment_start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        // Non-ASCII bytes can complete a zero-width scalar. ASCII `b` can
        // complete CSI REP and repeat the preceding combining scalar up to a
        // u16 count, so it is also an immediate cap boundary.
        if byte & 0x80 == 0 && *byte != b'b' {
            continue;
        }
        parser.advance(term, &bytes[segment_start..=index]);
        cap_cursor_combining_marks(term);
        segment_start = index + 1;
    }
    if segment_start < bytes.len() {
        parser.advance(term, &bytes[segment_start..]);
        cap_cursor_combining_marks(term);
    }
}

pub(super) fn grid_dimensions(
    width: f32,
    height: f32,
    cell_width: f32,
    line_height: f32,
) -> TermSize {
    let bounded = |available: f32, cell: f32, minimum: usize| {
        if !available.is_finite() || !cell.is_finite() || cell <= 0.0 {
            return minimum;
        }
        (available.max(0.0) / cell).floor() as usize
    };
    TermSize {
        cols: bounded(width - 16.0, cell_width, MIN_COLS).clamp(MIN_COLS, MAX_COLS),
        lines: bounded(height - 50.0, line_height, MIN_ROWS).clamp(MIN_ROWS, MAX_ROWS),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::session::EventProxy;
    use crate::MAX_TABS;
    use alacritty_terminal::event::{Event, EventListener};
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::cell::Flags;
    use vte::ansi::{Color, NamedColor};

    /// `vte` 0.15 terminates synchronized updates before this heap buffer fills.
    const MAX_VTE_SYNC_BUFFER_BYTES: usize = 2 * 1024 * 1024;
    const MAX_VTE_CSI_PARAMETERS: usize = 32;
    const MAX_VTE_INTERMEDIATES: usize = 2;
    const MAX_UTF8_SCALAR_BYTES: usize = 4;
    /// `alacritty_terminal` 0.25 evicts the oldest saved title at this depth.
    const MAX_TITLE_STACK_DEPTH: usize = 4096;

    #[derive(Clone, Default)]
    struct TitleEventProxy(Arc<Mutex<Vec<Option<String>>>>);

    impl EventListener for TitleEventProxy {
        fn send_event(&self, event: Event) {
            let title = match event {
                Event::Title(title) => Some(Some(title)),
                Event::ResetTitle => Some(None),
                _ => None,
            };
            if let (Some(title), Ok(mut events)) = (title, self.0.lock()) {
                events.push(title);
            }
        }
    }

    struct NoopHandler;

    impl vte::ansi::Handler for NoopHandler {}

    #[derive(Default)]
    struct ParserBoundRecorder {
        parameters: usize,
        intermediates: usize,
        ignored: bool,
        printed: String,
    }

    impl vte::Perform for ParserBoundRecorder {
        fn print(&mut self, character: char) {
            self.printed.push(character);
        }

        fn csi_dispatch(
            &mut self,
            parameters: &vte::Params,
            intermediates: &[u8],
            ignored: bool,
            _action: char,
        ) {
            self.parameters = parameters.iter().flatten().count();
            self.intermediates = intermediates.len();
            self.ignored = ignored;
        }
    }

    #[test]
    fn geometry_is_bounded_for_invalid_and_extreme_windows() {
        assert_eq!(
            grid_dimensions(0.0, 0.0, 7.8, 17.0),
            TermSize {
                cols: MIN_COLS,
                lines: MIN_ROWS,
            }
        );
        assert_eq!(
            grid_dimensions(f32::MAX, f32::MAX, 7.8, 17.0),
            TermSize {
                cols: MAX_COLS,
                lines: MAX_ROWS,
            }
        );
        assert_eq!(
            grid_dimensions(f32::NAN, f32::NAN, 7.8, 17.0),
            TermSize {
                cols: MIN_COLS,
                lines: MIN_ROWS,
            }
        );
    }

    #[test]
    fn vte_synchronized_update_buffer_stops_at_the_pinned_limit() {
        let mut parser: Processor = Processor::new();
        let mut handler = NoopHandler;
        parser.advance(&mut handler, b"\x1b[?2026h");

        let payload = vec![b'x'; MAX_VTE_SYNC_BUFFER_BYTES - 2];
        parser.advance(&mut handler, &payload);
        assert_eq!(parser.sync_bytes_count(), MAX_VTE_SYNC_BUFFER_BYTES - 2);

        parser.advance(&mut handler, b"x");
        assert_eq!(parser.sync_bytes_count(), 0);
    }

    #[test]
    fn vte_parser_arrays_and_partial_utf8_are_bounded() {
        let mut parser = vte::Parser::new();
        let mut recorder = ParserBoundRecorder::default();
        let mut parameters = b"\x1b[".to_vec();
        parameters.extend_from_slice("1;".repeat(MAX_VTE_CSI_PARAMETERS + 8).as_bytes());
        parameters.push(b'm');
        parser.advance(&mut recorder, &parameters);
        assert_eq!(recorder.parameters, MAX_VTE_CSI_PARAMETERS);
        assert!(recorder.ignored);

        recorder = ParserBoundRecorder::default();
        parser.advance(&mut recorder, b"\x1b[!!!m");
        assert_eq!(recorder.intermediates, MAX_VTE_INTERMEDIATES);
        assert!(recorder.ignored);

        recorder = ParserBoundRecorder::default();
        for byte in "😀".as_bytes().chunks(1) {
            parser.advance(&mut recorder, byte);
        }
        assert_eq!("😀".len(), MAX_UTF8_SCALAR_BYTES);
        assert_eq!(recorder.printed, "😀");
    }

    #[test]
    fn alacritty_title_stack_evicts_at_the_pinned_depth() {
        let proxy = TitleEventProxy::default();
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, proxy.clone());
        let mut parser: Processor = Processor::new();

        for title in 0..=MAX_TITLE_STACK_DEPTH {
            let sequence = format!("\x1b]0;{title}\x07\x1b[22t");
            parser.advance(&mut term, sequence.as_bytes());
        }
        if let Ok(mut events) = proxy.0.lock() {
            events.clear();
        }

        for _ in 0..=MAX_TITLE_STACK_DEPTH {
            parser.advance(&mut term, b"\x1b[23t");
        }

        let events = proxy
            .0
            .lock()
            .expect("title event lock should remain healthy");
        assert_eq!(events.len(), MAX_TITLE_STACK_DEPTH);
        assert_eq!(events.first().and_then(Option::as_deref), Some("4096"));
        assert_eq!(events.last().and_then(Option::as_deref), Some("1"));
    }

    #[test]
    fn aggregate_scrollback_budget_covers_every_supported_tab_count() {
        assert_eq!(scrollback_limit_for_tab_count(0), 0);
        assert_eq!(scrollback_limit_for_tab_count(1), SCROLLBACK_LINES);
        assert_eq!(scrollback_limit_for_tab_count(2), SCROLLBACK_LINES);
        assert!(bounded_cell_row_bytes() >= MAX_COLS * std::mem::size_of::<Cell>());
        assert!(
            retained_row_slots_bytes_per_tab()
                >= (SCROLLBACK_LINES + MAX_ROWS * VISIBLE_GRID_COPIES)
                    * std::mem::size_of::<Row<Cell>>()
        );

        let mut previous = SCROLLBACK_LINES;
        for tabs in 1..=MAX_TABS {
            let history = scrollback_limit_for_tab_count(tabs);
            assert!(history <= previous);
            assert!(
                bounded_grid_base_bytes(tabs, history) <= MAX_GRID_BASE_BYTES_PER_WINDOW,
                "{tabs} tabs with {history} history lines exceeded the base-grid budget"
            );
            previous = history;
        }

        let crowded_history = scrollback_limit_for_tab_count(MAX_TABS);
        assert!(crowded_history >= 650);
        assert!(crowded_history < SCROLLBACK_LINES);
        assert!(
            bounded_grid_base_bytes(MAX_TABS, crowded_history + 1) > MAX_GRID_BASE_BYTES_PER_WINDOW
        );
        assert_eq!(
            terminal_config(usize::MAX).scrolling_history,
            SCROLLBACK_LINES
        );
    }

    #[test]
    fn history_rebalance_preserves_zero_history_on_the_alternate_screen() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, EventProxy);
        let mut parser: Processor = Processor::new();

        parser.advance(&mut term, b"\x1b[?1049h");
        term.set_options(terminal_config(2));
        for _ in 0..20 {
            parser.advance(&mut term, b"alternate\r\n");
        }
        assert_eq!(term.grid().history_size(), 0);

        parser.advance(&mut term, b"\x1b[?1049l");
        for _ in 0..20 {
            parser.advance(&mut term, b"primary\r\n");
        }
        assert_eq!(term.grid().history_size(), 2);
    }

    fn append_combining_marks(bytes: &mut Vec<u8>, count: usize) {
        for _ in 0..count {
            bytes.extend_from_slice("\u{301}".as_bytes());
        }
    }

    #[test]
    fn combining_marks_are_bounded_per_exact_cell_without_losing_style() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, EventProxy);
        let mut parser: Processor = Processor::new();
        let mut bytes = b"\x1b[31;4ma".to_vec();
        append_combining_marks(&mut bytes, MAX_COMBINING_MARKS_PER_CELL + 20);
        bytes.push(b'b');
        append_combining_marks(&mut bytes, MAX_COMBINING_MARKS_PER_CELL + 20);

        advance_filtered_output(&mut parser, &mut term, &bytes);

        for column in [Column(0), Column(1)] {
            let cell = &term.grid()[Line(0)][column];
            assert_eq!(
                cell.zerowidth().map(<[char]>::len),
                Some(MAX_COMBINING_MARKS_PER_CELL)
            );
            assert_eq!(cell.fg, Color::Named(NamedColor::Red));
            assert!(cell.flags.contains(Flags::UNDERLINE));
        }

        // CSI REP is entirely ASCII but can repeat the preceding zero-width
        // scalar many times; its final `b` is an explicit cap boundary.
        advance_filtered_output(&mut parser, &mut term, b"\x1b[200b");
        assert_eq!(
            term.grid()[Line(0)][Column(1)]
                .zerowidth()
                .map(<[char]>::len),
            Some(MAX_COMBINING_MARKS_PER_CELL)
        );
    }

    #[test]
    fn combining_cap_handles_wide_cells_and_split_utf8() {
        let size = TermSize { cols: 20, lines: 5 };
        let mut term = Term::new(terminal_config(10), &size, EventProxy);
        let mut parser: Processor = Processor::new();
        let mut bytes = "界".as_bytes().to_vec();
        append_combining_marks(&mut bytes, MAX_COMBINING_MARKS_PER_CELL + 20);
        let split = "界".len() + 1;

        advance_filtered_output(&mut parser, &mut term, &bytes[..split]);
        advance_filtered_output(&mut parser, &mut term, &bytes[split..]);

        let wide = &term.grid()[Line(0)][Column(0)];
        assert_eq!(
            wide.zerowidth().map(<[char]>::len),
            Some(MAX_COMBINING_MARKS_PER_CELL)
        );
        assert!(term.grid()[Line(0)][Column(1)]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER));
    }
}
