//! Row layout for documents whose lines are too long for the editor.
//!
//! The editable text view (gpui-component's `InputState`) lays out every
//! soft-wrapped row of each buffer line that touches the viewport, on every
//! frame. A log or minified file with one multi-megabyte line therefore
//! shapes the whole line each frame: journey 5's 24 MB single-line fixture
//! peaked at 1.8 GB on the reference laptop. Documents whose longest line
//! exceeds [`LONG_LINE_LIMIT_BYTES`] open in a read-only view instead, which
//! wraps by character count and shapes only the rows on screen.

use std::ops::Range;

/// Longest line the editable view accepts. At the default 90 columns this
/// is about 700 wrapped rows laid out per frame for the line under the
/// caret, which stays interactive on the reference laptop.
pub(crate) const LONG_LINE_LIMIT_BYTES: usize = 64 * 1024;

/// Whether a document with this longest line must open read-only.
pub(crate) fn exceeds_editor_limit(longest_line_bytes: usize) -> bool {
    longest_line_bytes > LONG_LINE_LIMIT_BYTES
}

/// Byte length of the longest `\n`-separated line, without the newline.
pub(crate) fn longest_line_bytes(text: &str) -> usize {
    text.split('\n').map(str::len).max().unwrap_or(0)
}

/// Byte offset at which each display row starts. Rows end at every `\n`
/// and after `columns` characters, so a row never needs more than one
/// screen width of layout. An empty document has one empty row.
///
/// Offsets are `u32`: documents are capped at 64 MiB
/// (`document::MAX_DOCUMENT_BYTES`), so every offset fits and the index
/// costs 4 bytes per row.
pub(crate) fn wrap_rows(text: &str, columns: usize) -> Vec<u32> {
    let columns = columns.max(1);
    let mut rows = Vec::with_capacity(text.len() / columns + 1);
    rows.push(0);
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.is_ascii() {
            let mut offset = columns;
            while offset < body.len() {
                rows.push((line_start + offset) as u32);
                offset += columns;
            }
        } else {
            for (count, (offset, _)) in body.char_indices().enumerate() {
                if count > 0 && count % columns == 0 {
                    rows.push((line_start + offset) as u32);
                }
            }
        }
        line_start += line.len();
        if line.ends_with('\n') {
            rows.push(line_start as u32);
        }
    }
    rows
}

/// Byte range of display row `row`, without its trailing newline.
pub(crate) fn row_range(text: &str, rows: &[u32], row: usize) -> Range<usize> {
    let start = rows.get(row).map_or(text.len(), |&start| start as usize);
    let end = rows.get(row + 1).map_or(text.len(), |&end| end as usize);
    let end = if end > start && text.as_bytes()[end - 1] == b'\n' {
        end - 1
    } else {
        end
    };
    start..end
}

/// The display row containing byte `offset`.
pub(crate) fn row_for_offset(rows: &[u32], offset: usize) -> usize {
    rows.partition_point(|&start| start as usize <= offset)
        .saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(text: &str, columns: usize) -> Vec<&str> {
        let rows = wrap_rows(text, columns);
        (0..rows.len())
            .map(|row| &text[row_range(text, &rows, row)])
            .collect()
    }

    #[test]
    fn rows_break_at_newlines_and_at_the_column_limit() {
        assert_eq!(
            rows_of("abcdefg\nhi\n\nj", 3),
            ["abc", "def", "g", "hi", "", "j"]
        );
        assert_eq!(rows_of("abc\n", 3), ["abc", ""]);
        assert_eq!(rows_of("", 3), [""]);
    }

    #[test]
    fn rows_count_characters_not_bytes() {
        assert_eq!(rows_of("éééé🦀x", 2), ["éé", "éé", "🦀x"]);
    }

    #[test]
    fn offsets_map_back_to_their_row() {
        let text = "abcdef\ngh";
        let rows = wrap_rows(text, 4);
        assert_eq!(rows, [0, 4, 7]);
        assert_eq!(row_for_offset(&rows, 0), 0);
        assert_eq!(row_for_offset(&rows, 5), 1);
        assert_eq!(row_for_offset(&rows, 6), 1);
        assert_eq!(row_for_offset(&rows, 8), 2);
    }

    #[test]
    fn only_documents_with_very_long_lines_leave_the_editor() {
        assert_eq!(longest_line_bytes("ab\nabcd\n"), 4);
        assert!(!exceeds_editor_limit(LONG_LINE_LIMIT_BYTES));
        assert!(exceeds_editor_limit(LONG_LINE_LIMIT_BYTES + 1));
    }

    /// Journey 5's large fixture: a 24 MiB document that is one line. Its
    /// row index must stay a small fraction of the document.
    #[test]
    fn a_24_mib_single_line_indexes_in_a_few_megabytes() {
        let text = "0123456789abcdef".repeat(24 * 1024 * 1024 / 16);
        let rows = wrap_rows(&text, 90);
        assert_eq!(rows.len(), text.len().div_ceil(90));
        let index_bytes = rows.capacity() * std::mem::size_of::<u32>();
        assert!(index_bytes <= text.len() / 16, "{index_bytes} bytes");
        assert!(exceeds_editor_limit(longest_line_bytes(&text)));
    }
}
