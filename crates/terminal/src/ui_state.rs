//! Per-tab presentation state and terminal-grid selection projection.
//!
//! This module owns bounded, session-local UI state. The window controller
//! decides when state changes; this module defines the state contract and
//! converts selected terminal cells into clipboard text.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;

use crate::find::{FindMatch, FindStatus};

pub(super) const MAX_SEARCH_QUERY_BYTES: usize = 4096;

/// A selected cell range, in alacritty grid-line coordinates (`Line` values,
/// which are negative for scrollback). Coordinates are `(line, column)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Selection {
    pub(super) anchor: (i32, usize),
    pub(super) head: (i32, usize),
}

impl Selection {
    /// Return `(start, end)` ordered top-to-bottom, left-to-right.
    fn ordered(&self) -> ((i32, usize), (i32, usize)) {
        if (self.anchor.0, self.anchor.1) <= (self.head.0, self.head.1) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Is the cell at `(line, col)` inside the selection?
    pub(super) fn contains(&self, line: i32, col: usize) -> bool {
        let (start, end) = self.ordered();
        (line, col) >= start && (line, col) <= end
    }

    /// Extract selected cells as text. Hard line breaks become `\n`, while
    /// soft-wrapped rows are joined so wrapped commands copy as one line.
    pub(super) fn text<T>(&self, term: &Term<T>, rows: usize, cols: usize) -> String {
        let (start, end) = self.ordered();
        let grid = term.grid();
        let history = grid.total_lines().saturating_sub(grid.screen_lines()) as i32;
        let last_col = cols.saturating_sub(1);

        let mut output = String::new();
        let mut first = true;
        let mut previous_wrapped = false;
        for line in start.0..=end.0 {
            if line < -history || line >= rows as i32 {
                continue;
            }
            let (first_col, final_col) = if start.0 == end.0 {
                (start.1, end.1)
            } else if line == start.0 {
                (start.1, last_col)
            } else if line == end.0 {
                (0, end.1)
            } else {
                (0, last_col)
            };
            let row = &grid[Line(line)];
            let mut text = String::new();
            for col in first_col..=final_col.min(last_col) {
                let character = row[Column(col)].c;
                text.push(if character == '\0' { ' ' } else { character });
            }
            let wrapped =
                final_col >= last_col && row[Column(last_col)].flags.contains(Flags::WRAPLINE);

            if !first && !previous_wrapped {
                output.push('\n');
            }
            if wrapped {
                output.push_str(&text);
            } else {
                output.push_str(text.trim_end());
            }
            previous_wrapped = wrapped;
            first = false;
        }
        output
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SessionUiState {
    pub(super) selection: Option<Selection>,
    pub(super) search_open: bool,
    pub(super) search_query: String,
    /// The match ⌘G / ⇧⌘G last moved to, and the query it was found for.
    pub(super) find_current: Option<FindMatch>,
    pub(super) find_status: Option<(String, FindStatus)>,
}

pub(super) fn bounded_search_query(value: &str) -> String {
    if value.len() <= MAX_SEARCH_QUERY_BYTES {
        return value.to_string();
    }
    let mut end = MAX_SEARCH_QUERY_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_and_search_state_are_isolated_and_search_is_bounded() {
        let selection = Selection {
            anchor: (-2, 1),
            head: (3, 8),
        };
        let mut tabs = [SessionUiState::default(), SessionUiState::default()];
        tabs[0].selection = Some(selection);
        tabs[0].search_open = true;
        tabs[0].search_query = "first tab".into();

        assert_eq!(tabs[0].selection, Some(selection));
        assert_eq!(tabs[0].search_query, "first tab");
        assert_eq!(tabs[1], SessionUiState::default());

        let oversized = format!("{}😀", "a".repeat(MAX_SEARCH_QUERY_BYTES - 1));
        let bounded = bounded_search_query(&oversized);
        assert_eq!(bounded.len(), MAX_SEARCH_QUERY_BYTES - 1);
        assert!(bounded.is_char_boundary(bounded.len()));
        assert_eq!(
            bounded_search_query(&"b".repeat(MAX_SEARCH_QUERY_BYTES)),
            "b".repeat(MAX_SEARCH_QUERY_BYTES)
        );
    }
}
