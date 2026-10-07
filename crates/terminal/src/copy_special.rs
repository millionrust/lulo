//! Edit ▸ Copy Special ▸ Copy Without Background Colour (TRM-MENU-001): a
//! styled copy of the current selection, carrying RTF and HTML alongside
//! plain text so a paste into TextEdit, Mail or Notes keeps the
//! selection's real foreground ANSI colours without each cell's
//! background fill.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;
use vte::ansi::{Color, NamedColor};

use crate::profiles::Profile;
use crate::ui_state::Selection;

/// One run of selected text that shares a colour and style, in paste order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StyledRun {
    pub(crate) text: String,
    pub(crate) fg: (u8, u8, u8),
    pub(crate) bg: (u8, u8, u8),
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct CellStyle {
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    bold: bool,
    italic: bool,
    underline: bool,
}

/// `selection`'s cells as styled runs, resolving ANSI/indexed colours
/// through `profile` — the same palette math `controller/renderer/grid.rs`
/// uses to paint the screen, but parameterised rather than keyed off the
/// window's live thread-local profile, so a copy can use a different one
/// (or the same one, for "Terminal's Settings (Default)") without touching
/// what is on screen.
pub(crate) fn styled_runs<T>(
    selection: &Selection,
    term: &Term<T>,
    rows: usize,
    cols: usize,
    profile: &Profile,
) -> Vec<StyledRun> {
    let (start, end) = ordered(selection);
    let grid = term.grid();
    let history = grid.total_lines().saturating_sub(grid.screen_lines()) as i32;
    let last_col = cols.saturating_sub(1);

    let mut runs: Vec<StyledRun> = Vec::new();
    let mut push = |text: &str, style: CellStyle| {
        if let Some(last) = runs.last_mut() {
            if last.fg == style.fg
                && last.bg == style.bg
                && last.bold == style.bold
                && last.italic == style.italic
                && last.underline == style.underline
            {
                last.text.push_str(text);
                return;
            }
        }
        runs.push(StyledRun {
            text: text.to_owned(),
            fg: style.fg,
            bg: style.bg,
            bold: style.bold,
            italic: style.italic,
            underline: style.underline,
        });
    };
    let newline_style = CellStyle {
        fg: split(profile.fg),
        bg: split(profile.bg),
        bold: false,
        italic: false,
        underline: false,
    };

    let mut first_line = true;
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
        if !first_line {
            push("\n", newline_style);
        }
        first_line = false;
        for col in first_col..=final_col.min(last_col) {
            let cell = &row[Column(col)];
            let character = if cell.c == '\0' { ' ' } else { cell.c };
            let style = CellStyle {
                fg: resolve(cell.fg, profile),
                bg: resolve(cell.bg, profile),
                bold: cell.flags.intersects(Flags::BOLD | Flags::DIM_BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            };
            push(&character.to_string(), style);
        }
    }
    runs
}

/// Top-to-bottom, left-to-right order, mirroring `Selection::ordered`
/// (private to `ui_state`).
fn ordered(selection: &Selection) -> ((i32, usize), (i32, usize)) {
    if (selection.anchor.0, selection.anchor.1) <= (selection.head.0, selection.head.1) {
        (selection.anchor, selection.head)
    } else {
        (selection.head, selection.anchor)
    }
}

fn resolve(color: Color, profile: &Profile) -> (u8, u8, u8) {
    match color {
        Color::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        Color::Named(named) => named_color(named, profile),
        Color::Indexed(index) => indexed_color(index, profile),
    }
}

fn named_color(color: NamedColor, profile: &Profile) -> (u8, u8, u8) {
    use NamedColor::*;
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

fn indexed_color(index: u8, profile: &Profile) -> (u8, u8, u8) {
    match index {
        0..=15 => split(profile.ansi[index as usize]),
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

/// An HTML fragment for `runs`: one `<span>` per run with its resolved
/// foreground colour and weight/slant/underline inline, wrapped in a
/// `<pre>` on the page's own background so alignment survives the paste.
/// Every per-cell *background* is deliberately left out — that is the
/// whole difference Copy Without Background Colour (TRM-MENU-001) offers
/// over a plain styled copy: `grep --color` output pastes as coloured
/// text on the page's background rather than blocks of highlight colour.
/// Every consumer of `text/html` (Mail, Notes, a browser's rich editor)
/// renders inline styles, so this needs no stylesheet.
pub(crate) fn to_html_without_background(runs: &[StyledRun], background: u32) -> String {
    let mut html = format!(
        "<pre style=\"background-color:{};margin:0;font-family:monospace;\">",
        hex(split(background))
    );
    for run in runs {
        let mut style = format!("color:{}", hex(run.fg));
        if run.bold {
            style.push_str(";font-weight:bold");
        }
        if run.italic {
            style.push_str(";font-style:italic");
        }
        if run.underline {
            style.push_str(";text-decoration:underline");
        }
        html.push_str("<span style=\"");
        html.push_str(&style);
        html.push_str("\">");
        html.push_str(&escape_html(&run.text));
        html.push_str("</span>");
    }
    html.push_str("</pre>");
    html
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\n' => escaped.push_str("<br>"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// An RTF document for `runs`: a de-duplicated colour table and one
/// `\cfN` run each, carrying foreground colour and weight/slant/underline.
/// RTF's background highlight only names a fixed 16-colour palette, so an
/// arbitrary background is left to the HTML flavour alongside this one —
/// every consumer offered both picks whichever it understands best.
pub(crate) fn to_rtf(runs: &[StyledRun]) -> String {
    let mut palette: Vec<(u8, u8, u8)> = Vec::new();
    let mut index_of = |colour: (u8, u8, u8)| -> usize {
        if let Some(index) = palette.iter().position(|existing| *existing == colour) {
            return index + 1;
        }
        palette.push(colour);
        palette.len()
    };

    let mut body = String::new();
    for run in runs {
        let index = index_of(run.fg);
        body.push_str(&format!("\\cf{index} "));
        if run.bold {
            body.push_str("\\b ");
        }
        if run.italic {
            body.push_str("\\i ");
        }
        if run.underline {
            body.push_str("\\ul ");
        }
        body.push_str(&escape_rtf(&run.text));
        if run.bold {
            body.push_str("\\b0 ");
        }
        if run.italic {
            body.push_str("\\i0 ");
        }
        if run.underline {
            body.push_str("\\ulnone ");
        }
    }

    let mut colortbl = String::from("{\\colortbl;");
    for (r, g, b) in &palette {
        colortbl.push_str(&format!("\\red{r}\\green{g}\\blue{b};"));
    }
    colortbl.push('}');

    format!(
        "{{\\rtf1\\ansi\\deff0{{\\fonttbl{{\\f0\\fmodern Menlo;}}}}{colortbl}\\f0\\fs24 {body}}}"
    )
}

fn escape_rtf(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '{' => escaped.push_str("\\{"),
            '}' => escaped.push_str("\\}"),
            '\n' => escaped.push_str("\\line "),
            other if (other as u32) > 127 => {
                escaped.push_str(&format!("\\u{}?", other as u32));
            }
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            name: "Test",
            bg: 0x000000,
            fg: 0xffffff,
            cursor: 0xffffff,
            selection: 0x3333ff,
            ansi: [
                0x000000, 0x990000, 0x00a600, 0x999900, 0x0000b2, 0xb200b2, 0x00a6b2, 0xbfbfbf,
                0x666666, 0xe50000, 0x00d900, 0xe5e500, 0x0000ff, 0xe500e5, 0x00e5e5, 0xe5e5e5,
            ],
        }
    }

    fn run(text: &str, fg: u32) -> StyledRun {
        StyledRun {
            text: text.to_owned(),
            fg: split(fg),
            bg: (0, 0, 0),
            bold: false,
            italic: false,
            underline: false,
        }
    }

    #[test]
    fn named_colors_resolve_through_the_given_profile_not_the_active_one() {
        let profile = profile();
        assert_eq!(named_color(NamedColor::Red, &profile), split(0x990000));
        assert_eq!(
            named_color(NamedColor::Foreground, &profile),
            split(0xffffff)
        );
    }

    #[test]
    fn html_wraps_each_run_in_its_own_coloured_span() {
        let html = to_html_without_background(&[run("hi", 0xff0000)], 0x000000);
        assert!(html.contains("color:#ff0000"));
        assert!(html.contains(">hi</span>"));
    }

    #[test]
    fn html_leaves_out_each_runs_own_background_colour() {
        // TRM-MENU-001: this is the entire point of "Without Background
        // Colour" — the selection's highlighted background never reaches
        // the pasted HTML, only its foreground colour.
        let mut coloured = run("hi", 0xff0000);
        coloured.bg = (255, 255, 0);
        let html = to_html_without_background(&[coloured], 0x000000);
        assert!(!html.contains("255,255,0"));
        assert!(!html.contains("#ffff00"));
    }

    #[test]
    fn html_escapes_markup_characters_in_copied_text() {
        let html = to_html_without_background(&[run("<a> & b", 0xffffff)], 0x000000);
        assert!(html.contains("&lt;a&gt; &amp; b"));
    }

    #[test]
    fn rtf_deduplicates_repeated_colours_into_one_table_entry() {
        let rtf = to_rtf(&[run("a", 0xff0000), run("b", 0xff0000), run("c", 0x00ff00)]);
        // Two distinct colours used (red, green): exactly two `;` after the
        // colortbl's own leading one, so three in total.
        assert_eq!(rtf.matches("\\red255\\green0\\blue0;").count(), 1);
        assert_eq!(rtf.matches("\\cf1 ").count(), 2);
        assert_eq!(rtf.matches("\\cf2 ").count(), 1);
    }

    #[test]
    fn rtf_escapes_backslashes_and_braces() {
        let rtf = to_rtf(&[run("a{b}\\c", 0xffffff)]);
        assert!(rtf.contains("a\\{b\\}\\\\c"));
    }

    /// End-to-end proof for TRM-MENU-001: a real `Term` fed an SGR red
    /// escape, walked by `styled_runs` the same way Copy Without
    /// Background Colour does, resolves through the given profile — not
    /// some hard-coded colour.
    #[test]
    fn styled_runs_resolves_sgr_colours_through_the_given_profile() {
        use vte::ansi::Processor;

        let size = crate::emulator::TermSize { cols: 10, lines: 3 };
        let mut term = Term::new(
            crate::emulator::terminal_config(10),
            &size,
            crate::session::EventProxy::default(),
        );
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b[31mhi\x1b[0m");

        let selection = Selection {
            anchor: (0, 0),
            head: (0, 1),
        };
        let profile = profile();
        let runs = styled_runs(&selection, &term, 3, 10, &profile);

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "hi");
        assert_eq!(runs[0].fg, split(profile.ansi[1]));
    }
}
