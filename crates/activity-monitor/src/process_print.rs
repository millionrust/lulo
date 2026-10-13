//! File ▸ Print… (⌘P): the process list as it is on screen, handed to the
//! print portal as text. The Mac prints the visible columns of the current
//! view; so does this, one line per process, columns padded so they line up
//! in the monospaced page the portal path draws.

/// The most processes one printout carries, so a very long view cannot make
/// a runaway document.
pub(crate) const MAX_PRINTED_ROWS: usize = 2_000;

/// Columns of `rows` aligned under `titles`, left to right, two spaces
/// apart. Numeric-looking cells are right-aligned within their column.
pub(crate) fn table_text(titles: &[&str], rows: &[Vec<String>]) -> String {
    let columns = titles.len();
    let mut widths: Vec<usize> = titles.iter().map(|title| title.chars().count()).collect();
    for row in rows.iter().take(MAX_PRINTED_ROWS) {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: &[String]| {
        let mut text = String::new();
        for (index, width) in widths.iter().enumerate().take(columns) {
            let cell = cells.get(index).map_or("", String::as_str);
            let padding = width.saturating_sub(cell.chars().count());
            if index > 0 {
                text.push_str("  ");
            }
            if looks_numeric(cell) {
                text.push_str(&" ".repeat(padding));
                text.push_str(cell);
            } else {
                text.push_str(cell);
                // No trailing blanks on the last column.
                if index + 1 < columns {
                    text.push_str(&" ".repeat(padding));
                }
            }
        }
        text
    };
    let header: Vec<String> = titles.iter().map(|title| (*title).to_owned()).collect();
    let mut lines = vec![line(&header)];
    lines.extend(rows.iter().take(MAX_PRINTED_ROWS).map(|row| line(row)));
    lines.join("\n")
}

/// A cell that is a number, a percentage, a size or a duration, which reads
/// better against the right edge of its column.
fn looks_numeric(cell: &str) -> bool {
    let mut characters = cell.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_digit())
        && cell.chars().all(|c| {
            c.is_ascii_digit() || matches!(c, '.' | ',' | ':' | ' ' | '%') || c.is_alphabetic()
        })
        && cell.chars().filter(|c| c.is_alphabetic()).count() <= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|cell| (*cell).to_owned()).collect()
    }

    #[test]
    fn columns_line_up_with_numbers_on_the_right() {
        let text = table_text(
            &["Process Name", "% CPU", "PID"],
            &[
                row(&["firefox", "12.5", "4012"]),
                row(&["bash", "0.0", "77"]),
            ],
        );
        let expected = [
            "Process Name  % CPU  PID".to_owned(),
            format!("firefox{}12.5  4012", " ".repeat(8)),
            format!("bash{}0.0{}77", " ".repeat(12), " ".repeat(4)),
        ]
        .join("\n");
        assert_eq!(text, expected);
    }

    #[test]
    fn short_rows_and_missing_values_do_not_break_the_layout() {
        let text = table_text(&["Name", "Status"], &[row(&["init"]), row(&["a", "—"])]);
        assert_eq!(text, "Name  Status\ninit  \na     —");
    }

    #[test]
    fn a_runaway_view_is_cut_off() {
        let rows: Vec<Vec<String>> = (0..MAX_PRINTED_ROWS + 50)
            .map(|index| vec![index.to_string()])
            .collect();
        let text = table_text(&["PID"], &rows);
        assert_eq!(text.lines().count(), MAX_PRINTED_ROWS + 1);
    }

    #[test]
    fn sizes_and_durations_count_as_numeric() {
        assert!(looks_numeric("1.5 GB"));
        assert!(looks_numeric("0:03:12"));
        assert!(looks_numeric("12.5"));
        assert!(!looks_numeric("firefox"));
        assert!(!looks_numeric("—"));
        assert!(!looks_numeric(""));
    }
}
