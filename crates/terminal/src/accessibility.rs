//! Bounded visible-grid text, caret, and selection semantics.

use std::ops::Range;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;

pub const MAX_ACCESSIBLE_ROWS: usize = 300;
pub const MAX_ACCESSIBLE_COLUMNS: usize = 500;
/// Covers the largest supported grid even when every cell carries the
/// terminal engine's bounded combining-mark payload.
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridPoint {
    /// Alacritty grid line. Scrollback lines are negative.
    pub line: i32,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSelection {
    pub anchor: GridPoint,
    pub head: GridPoint,
}

impl GridSelection {
    fn ordered(self) -> (GridPoint, GridPoint) {
        if (self.anchor.line, self.anchor.column) <= (self.head.line, self.head.column) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    fn contains(self, line: i32, column: usize) -> bool {
        let (start, end) = self.ordered();
        (line, column) >= (start.line, start.column) && (line, column) <= (end.line, end.column)
    }

    fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalAccessibilitySnapshot {
    /// Visible viewport text. Soft-wrapped rows are joined and hard rows use
    /// `\n`. Offsets below count Unicode scalar values, not UTF-8 bytes.
    pub text: String,
    /// Insertion point in `text`, available only at the live bottom viewport.
    pub caret: Option<usize>,
    /// Visible intersection of the grid selection as one half-open range.
    pub selection: Option<Range<usize>>,
    pub row_count: usize,
    pub column_count: usize,
    pub scrolled: bool,
    pub input_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    GeometryLimit,
    TextLimit,
}

pub fn project_visible_terminal<T>(
    term: &Term<T>,
    selection: Option<GridSelection>,
    input_enabled: bool,
) -> Result<TerminalAccessibilitySnapshot, ProjectionError> {
    let grid = term.grid();
    let rows = grid.screen_lines();
    let columns = grid.columns();
    if rows == 0 || columns == 0 || rows > MAX_ACCESSIBLE_ROWS || columns > MAX_ACCESSIBLE_COLUMNS {
        return Err(ProjectionError::GeometryLimit);
    }

    let display_offset = grid.display_offset();
    let cursor = (display_offset == 0 && input_enabled).then_some(GridPoint {
        line: grid.cursor.point.line.0,
        column: grid.cursor.point.column.0.min(columns.saturating_sub(1)),
    });
    let selection = selection.filter(|selection| !selection.is_empty());
    let mut text = String::new();
    let mut scalar_offset = 0;
    let mut caret = None;
    let mut selection_start = None;
    let mut selection_end = None;

    for viewport_row in 0..rows {
        let line = i32::try_from(viewport_row).unwrap_or(i32::MAX)
            - i32::try_from(display_offset).unwrap_or(i32::MAX);
        let row = &grid[Line(line)];
        let mut last_column = (0..columns).rev().find(|column| {
            let cell = &row[Column(*column)];
            !cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                && (cell.c != ' ' && cell.c != '\0'
                    || cell.zerowidth().is_some_and(|marks| !marks.is_empty()))
        });
        if cursor.is_some_and(|cursor| cursor.line == line) {
            last_column = last_column
                .into_iter()
                .chain(cursor.and_then(|cursor| cursor.column.checked_sub(1)))
                .max();
        }

        if let Some(last_column) = last_column {
            for column in 0..=last_column {
                let cell = &row[Column(column)];
                if cursor == Some(GridPoint { line, column }) {
                    caret = Some(scalar_offset);
                }
                let selected = selection.is_some_and(|selection| selection.contains(line, column));
                if selected {
                    selection_start.get_or_insert(scalar_offset);
                }
                if !cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    push_character(
                        &mut text,
                        if cell.c == '\0' { ' ' } else { cell.c },
                        &mut scalar_offset,
                    )?;
                    if let Some(marks) = cell.zerowidth() {
                        for mark in marks {
                            push_character(&mut text, *mark, &mut scalar_offset)?;
                        }
                    }
                }
                if selected {
                    selection_end = Some(scalar_offset);
                }
            }
        }
        if caret.is_none() && cursor.is_some_and(|cursor| cursor.line == line) {
            caret = Some(scalar_offset);
        }

        let hard_break = viewport_row + 1 < rows
            && !row[Column(columns.saturating_sub(1))]
                .flags
                .contains(Flags::WRAPLINE);
        if hard_break {
            let selects_break = selection.is_some_and(|selection| {
                let (start, end) = selection.ordered();
                line >= start.line && line < end.line
            });
            if selects_break {
                selection_start.get_or_insert(scalar_offset);
            }
            push_character(&mut text, '\n', &mut scalar_offset)?;
            if selects_break {
                selection_end = Some(scalar_offset);
            }
        }
    }

    let selection = selection_start
        .zip(selection_end)
        .filter(|(start, end)| start < end)
        .map(|(start, end)| start..end);
    Ok(TerminalAccessibilitySnapshot {
        text,
        caret,
        selection,
        row_count: rows,
        column_count: columns,
        scrolled: display_offset != 0,
        input_enabled,
    })
}

fn push_character(
    text: &mut String,
    character: char,
    scalar_offset: &mut usize,
) -> Result<(), ProjectionError> {
    if text.len().saturating_add(character.len_utf8()) > MAX_ACCESSIBLE_TEXT_BYTES {
        return Err(ProjectionError::TextLimit);
    }
    text.push(character);
    *scalar_offset = scalar_offset.saturating_add(1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use vte::ansi::Processor;

    #[derive(Clone, Copy)]
    struct Size {
        rows: usize,
        columns: usize,
    }

    impl Dimensions for Size {
        fn total_lines(&self) -> usize {
            self.rows
        }

        fn screen_lines(&self) -> usize {
            self.rows
        }

        fn columns(&self) -> usize {
            self.columns
        }
    }

    fn term(rows: usize, columns: usize, bytes: &[u8]) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &Size { rows, columns }, VoidListener);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, bytes);
        term
    }

    #[test]
    fn visible_text_and_caret_use_unicode_scalar_offsets() {
        let term = term(3, 10, b"hello\r\nworld");
        let snapshot = project_visible_terminal(&term, None, true).unwrap();
        assert_eq!(snapshot.text, "hello\nworld\n");
        assert_eq!(snapshot.caret, Some(11));
        assert_eq!(snapshot.selection, None);
        assert_eq!((snapshot.row_count, snapshot.column_count), (3, 10));
        assert!(!snapshot.scrolled);
        assert!(snapshot.input_enabled);
    }

    #[test]
    fn soft_wraps_wide_cells_and_combining_marks_are_semantic_text() {
        let wrapped = term(3, 5, b"abcdef");
        assert_eq!(
            project_visible_terminal(&wrapped, None, true).unwrap().text,
            "abcdef\n"
        );

        let unicode = term(2, 10, "界e\u{301}".as_bytes());
        let snapshot = project_visible_terminal(&unicode, None, true).unwrap();
        assert_eq!(snapshot.text, "界e\u{301}\n");
        assert_eq!(snapshot.caret, Some(3));
    }

    #[test]
    fn grid_selection_projects_to_one_half_open_text_range() {
        let grid = term(3, 10, b"abc\r\ndef");
        let selection = GridSelection {
            anchor: GridPoint { line: 0, column: 1 },
            head: GridPoint { line: 1, column: 1 },
        };
        let snapshot = project_visible_terminal(&grid, Some(selection), false).unwrap();
        assert_eq!(snapshot.text, "abc\ndef\n");
        assert_eq!(snapshot.selection, Some(1..6));
        assert_eq!(&snapshot.text[1..6], "bc\nde");
        assert_eq!(snapshot.caret, None);

        let blank_end = term(3, 10, b"abc\r\n");
        let snapshot = project_visible_terminal(&blank_end, Some(selection), false).unwrap();
        assert_eq!(snapshot.text, "abc\n\n");
        assert_eq!(snapshot.selection, Some(1..4));
        assert_eq!(&snapshot.text[1..4], "bc\n");
    }

    #[test]
    fn public_projection_refuses_geometry_beyond_terminal_contract() {
        let term = term(MAX_ACCESSIBLE_ROWS + 1, 10, b"");
        assert_eq!(
            project_visible_terminal(&term, None, true),
            Err(ProjectionError::GeometryLimit)
        );
    }
}
