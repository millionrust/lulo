//! Find in scrollback: case-insensitive matching over grid cells, stepping
//! between matches, and the scroll position that brings a match into view.
//!
//! Everything here is pure so it can be tested without a terminal. Matches
//! are found within one grid row; text that soft-wraps onto the next row is
//! not joined (a known limitation shared with the highlight).

/// One grid cell as the matcher sees it: its column, how many columns it
/// covers (2 for a wide character) and its character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellText {
    pub(crate) column: usize,
    pub(crate) width: usize,
    pub(crate) character: char,
}

/// A match on one grid line. `line` uses the grid's coordinates: 0 is the
/// top of the screen and negative lines are scrollback. `end` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FindMatch {
    pub(crate) line: i32,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Where Find stands after the last ⌘G / ⇧⌘G: which match is current and how
/// many there were. `current` is 1-based; a total of 0 means "Not Found".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FindStatus {
    pub(crate) current: usize,
    pub(crate) total: usize,
}

impl FindStatus {
    pub(crate) fn label(&self) -> String {
        if self.total == 0 {
            "Not Found".to_string()
        } else {
            format!("{} of {}", self.current, self.total)
        }
    }
}

/// Fold a character for case-insensitive comparison. Characters whose
/// lowercase form is more than one character (rare) compare as themselves so
/// one cell always stays one character.
fn fold(character: char) -> char {
    let mut lower = character.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

/// The query as folded characters, ready for [`match_columns`].
pub(crate) fn fold_query(query: &str) -> Vec<char> {
    query.chars().map(fold).collect()
}

/// Column ranges (`start..end`) where `needle` occurs in the row, left to
/// right and without overlaps.
pub(crate) fn match_columns(cells: &[CellText], needle: &[char]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    if needle.is_empty() || cells.len() < needle.len() {
        return spans;
    }
    let mut index = 0;
    while index + needle.len() <= cells.len() {
        let hit = cells[index..index + needle.len()]
            .iter()
            .zip(needle)
            .all(|(cell, wanted)| fold(cell.character) == *wanted);
        if hit {
            let last = &cells[index + needle.len() - 1];
            spans.push((cells[index].column, last.column + last.width.max(1)));
            index += needle.len();
        } else {
            index += 1;
        }
    }
    spans
}

/// Mark the columns of a `cols`-wide row covered by any span.
pub(crate) fn covered_columns(spans: &[(usize, usize)], cols: usize) -> Vec<bool> {
    let mut covered = vec![false; cols];
    for &(start, end) in spans {
        for cell in covered.iter_mut().take(end.min(cols)).skip(start) {
            *cell = true;
        }
    }
    covered
}

/// The match to show after a step. `matches` must be sorted top to bottom.
///
/// Next moves down (later output) and Previous moves up, both wrapping. With
/// no current match, or one that has scrolled away, both start from the match
/// nearest the bottom, since recent output is what people usually look for.
pub(crate) fn step(
    matches: &[FindMatch],
    current: Option<FindMatch>,
    forward: bool,
) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let last = matches.len() - 1;
    let Some(current) = current else {
        return Some(last);
    };
    Some(match matches.binary_search(&current) {
        Ok(index) if forward => (index + 1) % matches.len(),
        Ok(index) => index.checked_sub(1).unwrap_or(last),
        // The current match is gone (the output moved or changed): continue
        // from where it was.
        Err(insert) if forward => {
            if insert > last {
                0
            } else {
                insert
            }
        }
        Err(insert) => insert.checked_sub(1).unwrap_or(last),
    })
}

/// The display offset (lines scrolled back from the bottom) that shows
/// `line`. A line already on screen keeps the current offset; otherwise the
/// line is brought to the middle of the screen, within the scrollback.
pub(crate) fn display_offset_for(
    line: i32,
    rows: usize,
    history_size: usize,
    current_offset: usize,
) -> usize {
    let rows = rows.max(1) as i64;
    let viewport_row = i64::from(line) + current_offset as i64;
    if (0..rows).contains(&viewport_row) {
        return current_offset;
    }
    let centred = rows / 2 - i64::from(line);
    centred.clamp(0, history_size as i64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str) -> Vec<CellText> {
        text.chars()
            .enumerate()
            .map(|(column, character)| CellText {
                column,
                width: 1,
                character,
            })
            .collect()
    }

    fn found(line: i32, start: usize, end: usize) -> FindMatch {
        FindMatch { line, start, end }
    }

    #[test]
    fn matching_ignores_case_and_never_overlaps() {
        let needle = fold_query("aA");
        assert_eq!(
            match_columns(&row("AAAa-aa"), &needle),
            vec![(0, 2), (2, 4), (5, 7)]
        );
        assert_eq!(
            match_columns(&row("Error: error"), &fold_query("ERROR")),
            vec![(0, 5), (7, 12)]
        );
        assert!(match_columns(&row("short"), &fold_query("longer query")).is_empty());
        assert!(match_columns(&row("anything"), &[]).is_empty());
    }

    #[test]
    fn columns_follow_cells_not_bytes() {
        // "é" is two bytes, and the wide "日" covers two columns; the spacer
        // column after it is not a cell of its own.
        let cells = vec![
            CellText {
                column: 0,
                width: 1,
                character: 'é',
            },
            CellText {
                column: 1,
                width: 1,
                character: 'x',
            },
            CellText {
                column: 2,
                width: 2,
                character: '日',
            },
            CellText {
                column: 4,
                width: 1,
                character: 'y',
            },
        ];
        assert_eq!(match_columns(&cells, &fold_query("x日")), vec![(1, 4)]);
        assert_eq!(match_columns(&cells, &fold_query("É")), vec![(0, 1)]);
        assert_eq!(match_columns(&cells, &fold_query("日y")), vec![(2, 5)]);
    }

    #[test]
    fn covered_columns_clip_to_the_row() {
        assert_eq!(
            covered_columns(&[(1, 3), (4, 9)], 6),
            vec![false, true, true, false, true, true]
        );
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let matches = [found(-5, 0, 3), found(-1, 2, 5), found(3, 0, 3)];
        assert_eq!(step(&matches, None, true), Some(2));
        assert_eq!(step(&matches, None, false), Some(2));
        assert_eq!(step(&matches, Some(matches[2]), true), Some(0));
        assert_eq!(step(&matches, Some(matches[0]), false), Some(2));
        assert_eq!(step(&matches, Some(matches[1]), true), Some(2));
        assert_eq!(step(&matches, Some(matches[1]), false), Some(0));
        assert_eq!(step(&[], None, true), None);
    }

    #[test]
    fn a_vanished_match_continues_from_its_place() {
        let matches = [found(-5, 0, 3), found(-1, 2, 5), found(3, 0, 3)];
        let gone = found(-3, 0, 3);
        assert_eq!(step(&matches, Some(gone), true), Some(1));
        assert_eq!(step(&matches, Some(gone), false), Some(0));
        let below_all = found(10, 0, 1);
        assert_eq!(step(&matches, Some(below_all), true), Some(0));
        let above_all = found(-9, 0, 1);
        assert_eq!(step(&matches, Some(above_all), false), Some(2));
    }

    #[test]
    fn scrolling_keeps_visible_lines_and_centres_the_rest() {
        // On screen at the bottom: nothing moves.
        assert_eq!(display_offset_for(3, 24, 100, 0), 0);
        // Already visible while scrolled back.
        assert_eq!(display_offset_for(-10, 24, 100, 20), 20);
        // In scrollback: brought to the middle row.
        assert_eq!(display_offset_for(-40, 24, 100, 0), 52);
        // Never past the oldest line or below the bottom.
        assert_eq!(display_offset_for(-100, 24, 100, 0), 100);
        assert_eq!(display_offset_for(20, 24, 100, 80), 0);
    }

    #[test]
    fn status_reads_like_a_find_bar() {
        assert_eq!(
            FindStatus {
                current: 3,
                total: 12
            }
            .label(),
            "3 of 12"
        );
        assert_eq!(FindStatus::default().label(), "Not Found");
    }
}
