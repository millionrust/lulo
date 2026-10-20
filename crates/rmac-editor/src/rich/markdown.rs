//! Notes Markdown: how Notes stores an attributed note body as text.
//!
//! Each paragraph is one line, so a plain note's stored text is exactly
//! the text a person typed and searching, snippets and export keep working
//! on it. Structure Markdown already has is written as Markdown:
//!
//! - `# `, `## `, `### ` start a Title, Heading or Subheading paragraph, and
//!   a fenced code block holds Monostyled paragraphs;
//! - `- `, `* `, `1. ` and `- [ ] ` / `- [x] ` start dashed, bulleted,
//!   numbered and checklist items, two spaces per nesting level, and `> `
//!   a block quote;
//! - `**bold**`, `_italic_`, `~~strikethrough~~` and `` `code` ``, with
//!   Notes' own `++underline++`, `==highlight==`, `^superscript^` and
//!   `::subscript::`, and `[links](address)`.
//!
//! What Markdown has no syntax for travels as inline HTML a Markdown reader
//! keeps: `<span style="font-family:…; font-size:…pt; color:#…;
//! background-color:#…; direction:rtl">`. Paragraph alignment, writing
//! direction and indentation are trailing ` :center:`, ` :right:`,
//! ` :justify:`, ` :rtl:`, ` :ltr:` and ` :indent-N:` tokens (centre and
//! right are what earlier Notes versions already wrote).
//!
//! Writing checks every line by reading it back: a line whose text would
//! read back differently (a `*` or `#` someone typed) is written again with
//! every attribute as a span and every Markdown character escaped, so any
//! document survives the trip. Reading accepts every earlier Notes body:
//! anything it does not recognise stays literal text.

use std::sync::Arc;

use super::model::{
    Alignment, CharStyle, Document, ListKind, Paragraph, ParagraphRole, ParagraphStyle, Rgb,
    StyledRun, WritingDirection,
};

/// Format ▸ Font ▸ Highlight's colour, what `==text==` means.
pub const NOTES_HIGHLIGHT: Rgb = Rgb::from_u32(0xF5DB6B);

/// Each paragraph role's typeface, size and weight.
pub type Typography<'a> = &'a dyn Fn(ParagraphRole) -> CharStyle;

/// Characters an escaped line writes with a backslash before them.
const ESCAPED: &str = "\\*_~+=^:`[]<>!";

/// Read a stored note body.
pub fn parse(markdown: &str, typography: Typography<'_>) -> Document {
    let normalized = super::model::normalize_newlines(markdown);
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut paragraphs = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.starts_with("```") {
            // A fenced block: Monostyled paragraphs up to the closing fence.
            let base = typography(ParagraphRole::Monostyled);
            let style = ParagraphStyle {
                role: ParagraphRole::Monostyled,
                ..ParagraphStyle::default()
            };
            index += 1;
            while index < lines.len() && lines[index].trim_end() != "```" {
                paragraphs.push(Paragraph::plain(lines[index], &base, style));
                index += 1;
            }
            // Past the closing fence (or the end of an unclosed one).
            index += 1;
            continue;
        }
        paragraphs.push(parse_line(line, typography));
        index += 1;
    }
    if paragraphs.is_empty() {
        paragraphs.push(Paragraph::plain(
            "",
            &typography(ParagraphRole::Body),
            ParagraphStyle::default(),
        ));
    }
    Document::from_paragraphs(paragraphs)
}

/// Write a note body.
pub fn write(document: &Document, typography: Typography<'_>) -> String {
    let numbers = document.list_numbers();
    let paragraphs = document.paragraphs();
    let mut out = String::with_capacity(document.len() + 16);
    let mut index = 0;
    let mut first = true;
    let mut push_line = |out: &mut String, line: &str| {
        if !std::mem::take(&mut first) {
            out.push('\n');
        }
        out.push_str(line);
    };
    while index < paragraphs.len() {
        if fenceable(&paragraphs[index], typography) {
            push_line(&mut out, "```");
            while index < paragraphs.len() && fenceable(&paragraphs[index], typography) {
                push_line(&mut out, paragraphs[index].text());
                index += 1;
            }
            push_line(&mut out, "```");
            continue;
        }
        let paragraph = &paragraphs[index];
        let line = write_line(paragraph, numbers[index], typography, false);
        let line = if same_encoding(&parse_line(&line, typography), paragraph) {
            line
        } else {
            write_line(paragraph, numbers[index], typography, true)
        };
        push_line(&mut out, &line);
        index += 1;
    }
    out
}

/// The text a body reads as, without any Markdown: for snippets and
/// search previews.
pub fn plain_text(markdown: &str) -> String {
    parse(markdown, &|_| CharStyle::default()).text()
}

// ---- Reading ---------------------------------------------------------------

fn parse_line(line: &str, typography: Typography<'_>) -> Paragraph {
    let mut style = ParagraphStyle::default();
    let mut rest = line;

    // Trailing paragraph tokens, in any order.
    'tokens: loop {
        for (token, apply) in TOKENS {
            if let Some(stripped) = rest.strip_suffix(token) {
                if let Some(stripped) = stripped.strip_suffix(' ') {
                    apply(&mut style);
                    rest = stripped;
                    continue 'tokens;
                }
            }
        }
        if let Some((stripped, level)) = indent_token(rest) {
            style.list_level = level;
            rest = stripped;
            continue;
        }
        break;
    }

    if let Some(stripped) = rest.strip_prefix("> ") {
        style.quote = true;
        rest = stripped;
    } else if rest == ">" {
        style.quote = true;
        rest = "";
    }

    let spaces = rest.len() - rest.trim_start_matches(' ').len();
    let after_spaces = &rest[spaces..];
    if let Some((list, checked, text)) = list_marker(after_spaces) {
        style.list = Some(list);
        style.checked = checked;
        style.list_level = (spaces / 2).min(usize::from(super::model::MAX_LIST_LEVEL)) as u8;
        rest = text;
    }

    for (marker, role) in [
        ("# ", ParagraphRole::Title),
        ("## ", ParagraphRole::Heading),
        ("### ", ParagraphRole::Subheading),
    ] {
        if let Some(stripped) = rest.strip_prefix(marker) {
            style.role = role;
            rest = stripped;
            break;
        }
    }

    let base = typography(style.role);
    let mono = typography(ParagraphRole::Monostyled).family;
    let mut segments: Vec<(String, CharStyle)> = Vec::new();
    parse_inline(rest, &base, mono.as_ref(), &mut segments);
    let mut text = String::with_capacity(rest.len());
    let mut runs = Vec::with_capacity(segments.len());
    for (piece, piece_style) in segments {
        text.push_str(&piece);
        runs.push(StyledRun {
            len: piece.len(),
            style: piece_style,
        });
    }
    if runs.is_empty() {
        runs.push(StyledRun {
            len: 0,
            style: base,
        });
    }
    Paragraph::new(text, runs, style)
}

type TokenApply = fn(&mut ParagraphStyle);

const TOKENS: [(&str, TokenApply); 6] = [
    (":center:", |style| style.alignment = Alignment::Center),
    (":right:", |style| style.alignment = Alignment::Right),
    (":justify:", |style| style.alignment = Alignment::Justified),
    (":rtl:", |style| style.direction = WritingDirection::RightToLeft),
    (":ltr:", |style| style.direction = WritingDirection::LeftToRight),
    (":mono:", |style| style.role = ParagraphRole::Monostyled),
];

/// A trailing ` :indent-N:`.
fn indent_token(line: &str) -> Option<(&str, u8)> {
    let without = line.strip_suffix(':')?;
    let at = without.rfind(" :indent-")?;
    let level: u8 = without[at + " :indent-".len()..].parse().ok()?;
    (1..=super::model::MAX_LIST_LEVEL)
        .contains(&level)
        .then_some((&line[..at], level))
}

/// A list item's marker at the start of `line`: its kind, tick and text.
fn list_marker(line: &str) -> Option<(ListKind, bool, &str)> {
    for (marker, checked) in [("- [ ] ", false), ("- [x] ", true), ("- [X] ", true)] {
        if let Some(text) = line.strip_prefix(marker) {
            return Some((ListKind::Checkbox, checked, text));
        }
    }
    if let Some(text) = line.strip_prefix("- ") {
        return Some((ListKind::Hyphen, false, text));
    }
    if let Some(text) = line.strip_prefix("* ") {
        return Some((ListKind::Bullet, false, text));
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        if let Some(text) = line[digits..].strip_prefix(". ") {
            return Some((ListKind::Numbered, false, text));
        }
    }
    None
}

/// Inline delimiters: (opening and closing text, whether it may not touch a
/// letter or digit outside it, what it does).
type InlineApply = fn(&mut CharStyle);

const DELIMITERS: [(&str, bool, InlineApply); 9] = [
    ("**", false, |style| style.bold = true),
    ("__", true, |style| style.bold = true),
    ("~~", false, |style| style.strikethrough = true),
    ("++", true, |style| style.underline = true),
    ("==", true, |style| style.highlight = Some(NOTES_HIGHLIGHT)),
    ("::", true, |style| style.superscript = -1),
    ("_", true, |style| style.italic = true),
    ("*", false, |style| style.italic = true),
    ("^", true, |style| style.superscript = 1),
];

fn push_text(segments: &mut Vec<(String, CharStyle)>, text: &str, style: &CharStyle) {
    if text.is_empty() {
        return;
    }
    match segments.last_mut() {
        Some((last, last_style)) if last_style == style => last.push_str(text),
        _ => segments.push((text.to_owned(), style.clone())),
    }
}

fn is_escapable(character: char) -> bool {
    character.is_ascii_punctuation()
}

fn parse_inline(
    text: &str,
    style: &CharStyle,
    mono: Option<&Arc<str>>,
    segments: &mut Vec<(String, CharStyle)>,
) {
    let mut literal_start = 0;
    let mut at = 0;
    let flush = |segments: &mut Vec<(String, CharStyle)>, from: usize, to: usize| {
        push_text(segments, &text[from..to], style);
    };
    while at < text.len() {
        let rest = &text[at..];
        let character = rest.chars().next().unwrap_or(' ');
        // A backslash escape: the next punctuation character is literal.
        if character == '\\' {
            if let Some(next) = rest[1..].chars().next().filter(|c| is_escapable(*c)) {
                flush(segments, literal_start, at);
                push_text(segments, &next.to_string(), style);
                at += 1 + next.len_utf8();
                literal_start = at;
                continue;
            }
        }
        // A code span: literal text in the monospaced face.
        if character == '`' {
            let ticks = rest.bytes().take_while(|byte| *byte == b'`').count();
            if let Some((content, end)) = code_span(text, at, ticks) {
                flush(segments, literal_start, at);
                let mut code = style.clone();
                code.family = mono.cloned();
                push_text(segments, content, &code);
                at = end;
                literal_start = at;
                continue;
            }
            at += ticks;
            continue;
        }
        // A span with a style attribute.
        if rest.starts_with("<span style=\"") {
            if let Some((css, inner, end)) = html_span(text, at) {
                flush(segments, literal_start, at);
                let mut inner_style = style.clone();
                apply_css(&mut inner_style, css);
                parse_inline(inner, &inner_style, mono, segments);
                at = end;
                literal_start = at;
                continue;
            }
        }
        // A link (never an image: `![` stays literal).
        if character == '[' && !text[..at].ends_with('!') {
            if let Some((label, address, end)) = link(text, at) {
                flush(segments, literal_start, at);
                let mut linked = style.clone();
                linked.link = Some(Arc::from(decode_address(address)));
                parse_inline(label, &linked, mono, segments);
                at = end;
                literal_start = at;
                continue;
            }
        }
        let mut matched = false;
        for (delimiter, word_bound, apply) in DELIMITERS {
            if !rest.starts_with(delimiter) {
                continue;
            }
            if let Some(close) = closing(text, at, delimiter, word_bound) {
                flush(segments, literal_start, at);
                let mut inner = style.clone();
                apply(&mut inner);
                parse_inline(&text[at + delimiter.len()..close], &inner, mono, segments);
                at = close + delimiter.len();
                literal_start = at;
                matched = true;
            }
            // The longest delimiter that starts here decides, matched or
            // not ("**" is never read as two "*").
            break;
        }
        if !matched {
            at += character.len_utf8();
        }
    }
    flush(segments, literal_start, text.len());
}

/// Where the delimiter opening at `open` closes, if it opens at all.
fn closing(text: &str, open: usize, delimiter: &str, word_bound: bool) -> Option<usize> {
    let after = open + delimiter.len();
    let next = text[after..].chars().next()?;
    if next.is_whitespace() {
        return None;
    }
    if word_bound
        && text[..open]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
    {
        return None;
    }
    let single = delimiter.len() == 1;
    let marker = delimiter.chars().next()?;
    let mut at = after;
    while at < text.len() {
        let rest = &text[at..];
        let character = rest.chars().next()?;
        if character == '\\' && rest[1..].chars().next().is_some_and(is_escapable) {
            at += 1 + rest[1..].chars().next().map_or(0, char::len_utf8);
            continue;
        }
        if character == '`' {
            // Skip a code span inside.
            let ticks = rest.bytes().take_while(|byte| *byte == b'`').count();
            match code_span(text, at, ticks) {
                Some((_, end)) => at = end,
                None => at += ticks,
            }
            continue;
        }
        if rest.starts_with(delimiter) && at > after {
            // A single-character delimiter does not close on half of a
            // doubled one ("*a **b** c*").
            let doubled = single && rest[1..].starts_with(marker);
            let before = text[..at].chars().next_back();
            let follow = rest[delimiter.len()..].chars().next();
            let flanks = !(before.is_some_and(char::is_whitespace)
                || word_bound && follow.is_some_and(char::is_alphanumeric));
            if !doubled && flanks {
                return Some(at);
            }
            if doubled {
                at += 2;
                continue;
            }
        }
        at += character.len_utf8();
    }
    None
}

/// A code span of `ticks` backticks at `open`: its content and end.
fn code_span(text: &str, open: usize, ticks: usize) -> Option<(&str, usize)> {
    let start = open + ticks;
    let mut at = start;
    while at < text.len() {
        let rest = &text[at..];
        if rest.starts_with('`') {
            let run = rest.bytes().take_while(|byte| *byte == b'`').count();
            if run == ticks {
                let mut content = &text[start..at];
                if content.len() >= 2
                    && content.starts_with(' ')
                    && content.ends_with(' ')
                    && !content.trim().is_empty()
                {
                    content = &content[1..content.len() - 1];
                }
                if content.is_empty() {
                    return None;
                }
                return Some((content, at + run));
            }
            at += run;
            continue;
        }
        at += rest.chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// `<span style="css">inner</span>` at `open`: css, inner and end.
fn html_span(text: &str, open: usize) -> Option<(&str, &str, usize)> {
    const OPEN: &str = "<span style=\"";
    const CLOSE: &str = "</span>";
    let css_start = open + OPEN.len();
    let css_end = css_start + text[css_start..].find("\">")?;
    let inner_start = css_end + 2;
    let mut depth = 1;
    let mut at = inner_start;
    while at < text.len() {
        let rest = &text[at..];
        if rest.starts_with('\\') && rest[1..].chars().next().is_some_and(is_escapable) {
            at += 1 + rest[1..].chars().next().map_or(0, char::len_utf8);
            continue;
        }
        if rest.starts_with(OPEN) {
            depth += 1;
            at += OPEN.len();
            continue;
        }
        if rest.starts_with(CLOSE) {
            depth -= 1;
            if depth == 0 {
                return Some((&text[css_start..css_end], &text[inner_start..at], at + CLOSE.len()));
            }
            at += CLOSE.len();
            continue;
        }
        at += rest.chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// `[label](address)` at `open`: label, address and end.
fn link(text: &str, open: usize) -> Option<(&str, &str, usize)> {
    let mut depth = 0;
    let mut at = open;
    let mut label_end = None;
    while at < text.len() {
        let rest = &text[at..];
        let character = rest.chars().next()?;
        if character == '\\' && rest[1..].chars().next().is_some_and(is_escapable) {
            at += 1 + rest[1..].chars().next().map_or(0, char::len_utf8);
            continue;
        }
        match character {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    label_end = Some(at);
                    break;
                }
            }
            _ => {}
        }
        at += character.len_utf8();
    }
    let label_end = label_end?;
    let address_start = label_end + 2;
    if !text[label_end + 1..].starts_with('(') {
        return None;
    }
    let address_end = address_start + text[address_start..].find(')')?;
    let address = &text[address_start..address_end];
    if address.is_empty() || address.contains(char::is_whitespace) || label_end == open + 1 {
        return None;
    }
    Some((&text[open + 1..label_end], address, address_end + 1))
}

/// A written address back to the one it stands for: the three characters
/// writing encodes.
fn decode_address(address: &str) -> String {
    address
        .replace("%28", "(")
        .replace("%29", ")")
        .replace("%20", " ")
}

fn parse_hex(value: &str) -> Option<Rgb> {
    let hex = value.trim().strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
        .map(Rgb::from_u32)
}

fn apply_css(style: &mut CharStyle, css: &str) {
    for declaration in css.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match property.trim() {
            "font-family" => {
                let family = value.trim_matches(|c| c == '\'' || c == '"');
                style.family = (!family.is_empty()).then(|| Arc::from(family));
            }
            "font-size" => {
                if let Ok(size) = value.trim_end_matches("pt").trim().parse::<f32>() {
                    style.size = super::model::clamp_size(size);
                }
            }
            "font-weight" => style.bold = value == "bold" || value == "700",
            "font-style" => style.italic = value == "italic",
            "text-decoration" => {
                style.underline = value.contains("underline");
                style.strikethrough = value.contains("line-through");
            }
            "color" => style.color = parse_hex(value),
            "background-color" | "background" => style.highlight = parse_hex(value),
            "vertical-align" => {
                style.superscript = match value {
                    "super" => 1,
                    "sub" => -1,
                    _ => 0,
                }
            }
            "direction" => {
                style.embedding = match value {
                    "rtl" => WritingDirection::RightToLeft,
                    "ltr" => WritingDirection::LeftToRight,
                    _ => WritingDirection::Default,
                }
            }
            _ => {}
        }
    }
}

// ---- Writing ---------------------------------------------------------------

/// The character attributes Notes Markdown keeps.
#[derive(Clone, Debug, PartialEq)]
struct Kept {
    family: Option<Arc<str>>,
    size: i32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    color: Option<Rgb>,
    highlight: Option<Rgb>,
    superscript: i8,
    link: Option<Arc<str>>,
    embedding: WritingDirection,
}

fn kept(style: &CharStyle) -> Kept {
    Kept {
        family: style.family.clone(),
        size: (style.size * 10.0).round() as i32,
        bold: style.bold,
        italic: style.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
        color: style.color,
        highlight: style.highlight,
        superscript: style.superscript.signum(),
        link: style.link.clone(),
        embedding: style.embedding,
    }
}

fn kept_list(list: Option<ListKind>) -> Option<ListKind> {
    list.map(|kind| match kind {
        ListKind::Checkbox | ListKind::Hyphen | ListKind::Numbered => kind,
        _ => ListKind::Bullet,
    })
}

/// Whether `read` (a written line read back) is `original` as far as Notes
/// Markdown keeps anything.
fn same_encoding(read: &Paragraph, original: &Paragraph) -> bool {
    let (a, b) = (read.style(), original.style());
    let paragraph_same = a.role == b.role
        && kept_list(a.list) == kept_list(b.list)
        && a.list_level == b.list_level
        && (a.list != Some(ListKind::Checkbox) || a.checked == b.checked)
        && a.quote == b.quote
        && a.alignment == b.alignment
        && a.direction == b.direction;
    if !paragraph_same || read.text() != original.text() {
        return false;
    }
    // Character by character; white space shows no weight, slant, face
    // or baseline, so those may differ on it.
    let chars = |paragraph: &Paragraph| -> Vec<Kept> {
        let mut kept_chars = Vec::with_capacity(paragraph.len());
        for (range, style) in paragraph.styled_ranges() {
            let style = kept(style);
            for character in paragraph.text()[range].chars() {
                let mut char_style = style.clone();
                if character.is_whitespace() {
                    char_style.bold = false;
                    char_style.italic = false;
                    char_style.superscript = 0;
                    char_style.family = None;
                }
                kept_chars.push(char_style);
            }
        }
        kept_chars
    };
    chars(read) == chars(original)
}

/// Whether a paragraph goes in a fenced code block.
fn fenceable(paragraph: &Paragraph, typography: Typography<'_>) -> bool {
    let style = paragraph.style();
    if style.role != ParagraphRole::Monostyled
        || style.list.is_some()
        || style.quote
        || style.list_level != 0
        || style.alignment != Alignment::Left
        || style.direction != WritingDirection::Default
        || paragraph.text().trim_end() == "```"
        || paragraph.text().starts_with("```")
    {
        return false;
    }
    let base = kept(&typography(ParagraphRole::Monostyled));
    paragraph
        .styled_ranges()
        .all(|(range, style)| range.is_empty() || kept(style) == base)
}

fn write_line(
    paragraph: &Paragraph,
    number: u32,
    typography: Typography<'_>,
    escape: bool,
) -> String {
    let style = paragraph.style();
    let mut line = String::with_capacity(paragraph.len() + 16);
    if style.quote {
        line.push_str("> ");
    }
    if let Some(list) = style.list {
        for _ in 0..style.list_level {
            line.push_str("  ");
        }
        match kept_list(Some(list)) {
            Some(ListKind::Checkbox) => {
                line.push_str(if style.checked { "- [x] " } else { "- [ ] " })
            }
            Some(ListKind::Hyphen) => line.push_str("- "),
            Some(ListKind::Numbered) => {
                line.push_str(&number.max(1).to_string());
                line.push_str(". ");
            }
            _ => line.push_str("* "),
        }
    }
    match style.role {
        ParagraphRole::Title => line.push_str("# "),
        ParagraphRole::Heading => line.push_str("## "),
        ParagraphRole::Subheading => line.push_str("### "),
        ParagraphRole::Body | ParagraphRole::Monostyled => {}
    }
    let base = typography(style.role);
    let mono = typography(ParagraphRole::Monostyled).family;
    let body_start = line.len();
    for (range, run) in paragraph.styled_ranges() {
        if range.is_empty() {
            continue;
        }
        write_run(
            &mut line,
            &paragraph.text()[range],
            run,
            &base,
            mono.as_ref(),
            escape,
        );
    }
    if escape {
        escape_line_start(&mut line, body_start);
    }
    let mut tokens: Vec<String> = Vec::new();
    match style.alignment {
        Alignment::Left => {}
        Alignment::Center => tokens.push(":center:".into()),
        Alignment::Right => tokens.push(":right:".into()),
        Alignment::Justified => tokens.push(":justify:".into()),
    }
    match style.direction {
        WritingDirection::Default => {}
        WritingDirection::LeftToRight => tokens.push(":ltr:".into()),
        WritingDirection::RightToLeft => tokens.push(":rtl:".into()),
    }
    if style.role == ParagraphRole::Monostyled {
        tokens.push(":mono:".into());
    }
    if style.list.is_none() && style.list_level > 0 {
        tokens.push(format!(":indent-{}:", style.list_level));
    }
    for token in tokens {
        line.push(' ');
        line.push_str(&token);
    }
    line
}

/// In an escaped line, a body that would read as a heading, list or quote
/// marker starts with a backslash instead.
fn escape_line_start(line: &mut String, body_start: usize) {
    let body = &line[body_start..];
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let needs = body.starts_with('#')
        || body.starts_with("- ")
        || body.starts_with(' ')
        || (digits > 0 && body[digits..].starts_with(". "));
    if !needs {
        return;
    }
    if digits > 0 {
        line.insert(body_start + digits, '\\');
    } else if body.starts_with(' ') {
        // Leading spaces before a list marker would read as nesting.
        let spaces = body.len() - body.trim_start_matches(' ').len();
        let after = &body[spaces..];
        if after.starts_with("- ") || after.starts_with("* ") {
            line.insert(body_start + spaces, '\\');
        }
    } else {
        line.insert(body_start, '\\');
    }
}

fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for character in text.chars() {
        if ESCAPED.contains(character) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

fn hex(color: Rgb) -> String {
    format!("#{:06x}", color.to_u32())
}

fn write_run(
    out: &mut String,
    text: &str,
    run: &CharStyle,
    base: &CharStyle,
    mono: Option<&Arc<str>>,
    escape: bool,
) {
    let mut css: Vec<String> = Vec::new();
    let as_code = !escape
        && run.family.is_some()
        && run.family.as_ref() == mono
        && base.family.as_ref() != mono
        && !text.contains('`');
    // Markers go round the run's words; white space at either end stays
    // outside them, where Markdown expects it.
    let core_start = text.len() - text.trim_start().len();
    let core_end = text.trim_end().len().max(core_start);
    let (lead, core, trail) = if escape || as_code {
        ("", text, "")
    } else {
        (&text[..core_start], &text[core_start..core_end], &text[core_end..])
    };
    let mut inner = if as_code {
        format!("`{text}`")
    } else if escape {
        escape_text(text)
    } else {
        core.to_owned()
    };
    let marked = !core.is_empty();
    if !as_code && run.family != base.family {
        match &run.family {
            Some(family) => css.push(format!("font-family:{family}")),
            None => css.push("font-family:".into()),
        }
    }
    if (run.size - base.size).abs() > 0.05 {
        css.push(format!("font-size:{}pt", trim_float(run.size)));
    }
    let wrap = |inner: &mut String, open: &str, close: &str| {
        inner.insert_str(0, open);
        inner.push_str(close);
    };
    let superscript = run.superscript.signum();
    if escape {
        if superscript > 0 {
            css.push("vertical-align:super".into());
        } else if superscript < 0 {
            css.push("vertical-align:sub".into());
        }
        if let Some(color) = run.highlight {
            css.push(format!("background-color:{}", hex(color)));
        }
        let mut decoration = Vec::new();
        if run.underline {
            decoration.push("underline");
        }
        if run.strikethrough {
            decoration.push("line-through");
        }
        if !decoration.is_empty() {
            css.push(format!("text-decoration:{}", decoration.join(" ")));
        }
        if run.italic {
            css.push("font-style:italic".into());
        }
        if run.bold != base.bold {
            css.push(format!(
                "font-weight:{}",
                if run.bold { "bold" } else { "normal" }
            ));
        }
    } else if marked {
        if superscript > 0 {
            wrap(&mut inner, "^", "^");
        } else if superscript < 0 {
            wrap(&mut inner, "::", "::");
        }
        match run.highlight {
            Some(color) if color == NOTES_HIGHLIGHT => wrap(&mut inner, "==", "=="),
            Some(color) => css.push(format!("background-color:{}", hex(color))),
            None => {}
        }
        if run.underline {
            wrap(&mut inner, "++", "++");
        }
        if run.strikethrough {
            wrap(&mut inner, "~~", "~~");
        }
        if run.italic {
            wrap(&mut inner, "_", "_");
        }
        if run.bold && !base.bold {
            wrap(&mut inner, "**", "**");
        } else if !run.bold && base.bold {
            css.push("font-weight:normal".into());
        }
    }
    if !escape && !as_code {
        inner = format!("{lead}{inner}{trail}");
    }
    if let Some(color) = run.color {
        css.push(format!("color:{}", hex(color)));
    }
    match run.embedding {
        WritingDirection::Default => {}
        WritingDirection::LeftToRight => css.push("direction:ltr".into()),
        WritingDirection::RightToLeft => css.push("direction:rtl".into()),
    }
    if let Some(address) = &run.link {
        let address = address
            .replace('(', "%28")
            .replace(')', "%29")
            .replace(' ', "%20");
        inner = format!("[{inner}]({address})");
    }
    if !css.is_empty() {
        inner = format!("<span style=\"{}\">{inner}</span>", css.join(";"));
    }
    out.push_str(&inner);
}

fn trim_float(value: f32) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typography(role: ParagraphRole) -> CharStyle {
        match role {
            ParagraphRole::Title => CharStyle {
                bold: true,
                ..CharStyle::with_size(20.0)
            },
            ParagraphRole::Heading => CharStyle {
                bold: true,
                ..CharStyle::with_size(17.0)
            },
            ParagraphRole::Subheading => CharStyle {
                bold: true,
                ..CharStyle::with_size(15.0)
            },
            ParagraphRole::Body => CharStyle::with_size(13.0),
            ParagraphRole::Monostyled => CharStyle {
                family: Some(Arc::from("Menlo")),
                ..CharStyle::with_size(12.0)
            },
        }
    }

    fn read(markdown: &str) -> Document {
        parse(markdown, &typography)
    }

    fn written(document: &Document) -> String {
        write(document, &typography)
    }

    /// Reading then writing gives the same text back.
    fn stable(markdown: &str) {
        assert_eq!(written(&read(markdown)), markdown, "{markdown:?}");
    }

    #[test]
    fn plain_notes_are_stored_as_their_own_text() {
        for body in ["", "Buy milk", "one\ntwo\n\nfour", "a\n", "10:30 at 192.168.1.1:5555"] {
            stable(body);
            assert_eq!(read(body).text(), body);
        }
    }

    #[test]
    fn earlier_notes_markdown_reads_as_styles() {
        let document = read("# Title\n## Heading\n### Sub\nBody **bold** _it_ ~~gone~~");
        assert_eq!(document.text(), "Title\nHeading\nSub\nBody bold it gone");
        assert_eq!(document.paragraph(0).style().role, ParagraphRole::Title);
        assert_eq!(document.paragraph(1).style().role, ParagraphRole::Heading);
        assert_eq!(document.paragraph(2).style().role, ParagraphRole::Subheading);
        assert!(document.paragraph(0).style_of_char_at(0).bold);
        assert_eq!(document.paragraph(0).style_of_char_at(0).size, 20.0);
        let body = document.paragraph(3);
        assert!(body.style_of_char_at(5).bold);
        assert!(body.style_of_char_at(10).italic);
        assert!(body.style_of_char_at(13).strikethrough);
        assert!(!body.style_of_char_at(0).bold);
    }

    #[test]
    fn notes_own_markers_read_as_underline_highlight_and_baseline() {
        let document = read("++u++ ==h== x^2^ H::2::O");
        let paragraph = document.paragraph(0);
        assert_eq!(paragraph.text(), "u h x^2^ H::2::O");
        assert!(paragraph.style_of_char_at(0).underline);
        assert_eq!(paragraph.style_of_char_at(2).highlight, Some(NOTES_HIGHLIGHT));
        // Inside a word, `^` and `::` are just text (a version, a path).
        assert_eq!(paragraph.style_of_char_at(5).superscript, 0);
        let separated = read("x ^2^ and ::n::");
        assert_eq!(separated.text(), "x 2 and n");
        assert_eq!(separated.paragraph(0).style_of_char_at(2).superscript, 1);
        assert_eq!(separated.paragraph(0).style_of_char_at(8).superscript, -1);
    }

    #[test]
    fn code_paths_and_maths_stay_literal() {
        for body in [
            "std::io::Read",
            "snake_case_name",
            "a == b",
            "c++ and g++",
            "2 * 3 * 4",
            "price $5 [draft]",
            "![image](a.png)",
        ] {
            assert_eq!(read(body).text(), body, "{body:?}");
            stable(body);
        }
    }

    #[test]
    fn lists_checklists_and_quotes_round_trip() {
        stable("- [ ] Buy milk\n- [x] Eggs\n- dash\n* bullet\n1. one\n2. two\n  - nested\n> quoted");
        let document = read("- [x] Eggs\n  * deep");
        let eggs = document.paragraph(0).style();
        assert_eq!(eggs.list, Some(ListKind::Checkbox));
        assert!(eggs.checked);
        assert_eq!(document.paragraph(0).text(), "Eggs");
        let deep = document.paragraph(1).style();
        assert_eq!((deep.list, deep.list_level), (Some(ListKind::Bullet), 1));
    }

    #[test]
    fn numbered_items_are_renumbered_on_write() {
        assert_eq!(written(&read("1. a\n1. b\n1. c")), "1. a\n2. b\n3. c");
    }

    #[test]
    fn monostyled_paragraphs_are_a_fenced_block() {
        let markdown = "Before\n```\nfn main() {\n    let x = 1;\n}\n```\nAfter";
        let document = read(markdown);
        assert_eq!(document.text(), "Before\nfn main() {\n    let x = 1;\n}\nAfter");
        assert_eq!(document.paragraph(2).style().role, ParagraphRole::Monostyled);
        assert_eq!(
            document.paragraph(2).style_of_char_at(0).family.as_deref(),
            Some("Menlo")
        );
        stable(markdown);
    }

    #[test]
    fn alignment_direction_and_indent_tokens_round_trip() {
        stable("Centred :center:\nRight :right:\nBoth :justify:\n\u{5e9}\u{5dc}\u{5d5}\u{5dd} :rtl:\nIndented :indent-2:");
        let document = read("Old centre :center:");
        assert_eq!(document.paragraph(0).style().alignment, Alignment::Center);
        assert_eq!(document.text(), "Old centre");
    }

    #[test]
    fn attributes_without_markdown_syntax_are_spans() {
        let mut document = Document::from_paragraphs(vec![Paragraph::plain(
            "red big",
            &typography(ParagraphRole::Body),
            ParagraphStyle::default(),
        )]);
        document.update_char_style(0..3, |style| style.color = Some(Rgb::from_u32(0xff0000)));
        document.update_char_style(4..7, |style| {
            style.size = 24.0;
            style.family = Some(Arc::from("Georgia"));
        });
        let markdown = written(&document);
        assert_eq!(
            markdown,
            "<span style=\"color:#ff0000\">red</span> <span style=\"font-family:Georgia;font-size:24pt\">big</span>"
        );
        let back = read(&markdown);
        assert_eq!(back.text(), "red big");
        assert_eq!(back.paragraph(0).style_of_char_at(0).color, Some(Rgb::from_u32(0xff0000)));
        assert_eq!(back.paragraph(0).style_of_char_at(4).size, 24.0);
        stable(&markdown);
    }

    #[test]
    fn typed_markdown_characters_are_escaped_and_survive() {
        for text in [
            "# not a heading",
            "- not a list",
            "1. not numbered",
            "**not bold**",
            "x :center:",
            "a `tick",
            "<span style=\"color:#000000\">literal</span>",
            "back\\slash \\* star",
        ] {
            let document = Document::from_paragraphs(vec![Paragraph::plain(
                text,
                &typography(ParagraphRole::Body),
                ParagraphStyle::default(),
            )]);
            let markdown = written(&document);
            let back = read(&markdown);
            assert_eq!(back.text(), text, "{markdown:?}");
            assert_eq!(back.paragraph(0).style(), ParagraphStyle::default());
            assert!(!back.paragraph(0).style_of_char_at(0).bold);
        }
    }

    #[test]
    fn styled_runs_round_trip_through_markers() {
        let body = typography(ParagraphRole::Body);
        let mut document = Document::from_paragraphs(vec![Paragraph::plain(
            "bold both link plain",
            &body,
            ParagraphStyle::default(),
        )]);
        document.update_char_style(0..9, |style| style.bold = true);
        document.update_char_style(5..9, |style| style.italic = true);
        document.update_char_style(10..14, |style| {
            style.link = Some(Arc::from("https://example.com/a(b)"));
        });
        let markdown = written(&document);
        assert_eq!(
            markdown,
            "**bold** _**both**_ [link](https://example.com/a%28b%29) plain"
        );
        let back = read(&markdown);
        assert_eq!(back.text(), "bold both link plain");
        assert!(back.paragraph(0).style_of_char_at(0).bold);
        assert!(back.paragraph(0).style_of_char_at(6).italic);
        assert!(back.paragraph(0).style_of_char_at(6).bold);
        assert_eq!(
            back.paragraph(0).style_of_char_at(10).link.as_deref(),
            Some("https://example.com/a(b)")
        );
    }

    #[test]
    fn a_title_with_a_plain_word_keeps_it_plain() {
        let mut document = read("# Big title");
        document.update_char_style(4..9, |style| style.bold = false);
        let markdown = written(&document);
        let back = read(&markdown);
        assert!(back.paragraph(0).style_of_char_at(0).bold);
        assert!(!back.paragraph(0).style_of_char_at(4).bold);
        assert_eq!(back.text(), "Big title");
    }

    #[test]
    fn selection_direction_survives() {
        let mut document = read("abc");
        document.update_char_style(0..3, |style| {
            style.embedding = WritingDirection::RightToLeft;
        });
        let back = read(&written(&document));
        assert_eq!(
            back.paragraph(0).style_of_char_at(0).embedding,
            WritingDirection::RightToLeft
        );
    }

    #[test]
    fn plain_text_drops_the_markup() {
        assert_eq!(plain_text("# Shopping\n- [ ] **Milk**"), "Shopping\nMilk");
    }
}
