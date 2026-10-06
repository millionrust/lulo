//! RTF, TextEdit's default rich-text format, read into and written from a
//! [`Document`].
//!
//! The reader is a bounded, inert, pure-Rust parser for the subset TextEdit
//! and other editors write: paragraphs, bold/italic/underline/strikethrough,
//! font family and size, text and highlight colours, alignment, line
//! spacing, bullet/numbered lists, tabs, line and page breaks, and the
//! `\'hh`/`\uN` escapes. Destinations that never reach the visible document
//! (font, colour, style and list tables, `\*` groups, pictures, objects,
//! field instructions) are skipped whole; nothing embedded is executed.
//!
//! The writer produces RTF TextEdit opens with the same formatting: one
//! `\pard` per paragraph with its ruler, character attributes as control
//! words, and lists in TextEdit's own `\ls`/`\listtext` form.
//!
//! Beyond that subset both sides know: outline (`\outl`), kerning
//! (`\expnd`/`\expndtw`/`\kerning`), superscript and subscript levels
//! (`\super`/`\sub`/`\nosupersub`), baseline offsets (`\up`/`\dn`), links
//! (`HYPERLINK` fields), nested lists with any NSTextList marker (the list
//! table's `\levelmarker`s and `\ilvl`), hyphenation (`\hyphauto`) and the
//! document properties (`\info`). Ligatures and Traditional Form have no
//! standard RTF control word, so they are written as `\luloligature` and
//! `\lulotraditional`, which other readers ignore as RTF requires.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use super::model::{
    clamp_size, Alignment, CharStyle, Document, DocumentAttributes, DocumentProperties, Ligatures,
    ListKind, Paragraph, ParagraphStyle, Rgb, StyledRun, DEFAULT_RICH_SIZE, MAX_LIST_LEVEL,
};

/// The family TextEdit's documents default to; read back as the document's
/// default face (`None`) so a round trip is exact.
const DEFAULT_FAMILY: &str = "Helvetica";

/// Destinations whose content is never part of the visible document,
/// whether or not they are also marked `\*` (many writers omit the `\*` on
/// `fonttbl`/`colortbl`/`info`, so they need to be named explicitly).
const SKIP_DESTINATIONS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "expandedcolortbl",
    "stylesheet",
    "info",
    "generator",
    "pict",
    "nonshppict",
    "shppict",
    "object",
    "header",
    "headerf",
    "headerl",
    "headerr",
    "footer",
    "footerf",
    "footerl",
    "footerr",
    "footnote",
    "xmlns",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "listtable",
    "listoverridetable",
    "rsidtbl",
    "bkmkstart",
    "bkmkend",
    "atnid",
    "atnauthor",
    "atndate",
    "annotation",
    "revtbl",
    "pgdsctbl",
    "datastore",
];

/// Windows-1252, the encoding `\ansicpg1252` names; Latin-1 elsewhere.
fn cp1252_to_char(byte: u8) -> char {
    match byte {
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        other => other as char,
    }
}

/// The colour table (`\cfN`/`\cbN` index it; entry 0 is "automatic").
fn parse_color_table(bytes: &[u8]) -> Vec<Option<Rgb>> {
    const NEEDLE: &[u8] = b"\\colortbl";
    let Some(needle_at) = bytes
        .windows(NEEDLE.len())
        .position(|window| window == NEEDLE)
    else {
        return Vec::new();
    };
    let mut pos = needle_at + NEEDLE.len();
    let mut depth = 1_i32;
    let mut colors = Vec::new();
    let (mut red, mut green, mut blue) = (None, None, None);
    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'{' => {
                depth += 1;
                pos += 1;
            }
            b'}' => {
                depth -= 1;
                pos += 1;
            }
            b';' => {
                colors.push(match (red, green, blue) {
                    (Some(r), Some(g), Some(b)) => Some(Rgb::new(r, g, b)),
                    _ => None,
                });
                (red, green, blue) = (None, None, None);
                pos += 1;
            }
            b'\\' => {
                pos += 1;
                let word_start = pos;
                while bytes.get(pos).is_some_and(u8::is_ascii_alphabetic) {
                    pos += 1;
                }
                let word = std::str::from_utf8(&bytes[word_start..pos]).unwrap_or("");
                let param_start = pos;
                while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
                    pos += 1;
                }
                let value = std::str::from_utf8(&bytes[param_start..pos])
                    .ok()
                    .and_then(|digits| digits.parse::<u32>().ok())
                    .map(|value| value.min(255) as u8);
                match word {
                    "red" => red = value,
                    "green" => green = value,
                    "blue" => blue = value,
                    _ => {}
                }
                if bytes.get(pos) == Some(&b' ') {
                    pos += 1;
                }
            }
            _ => pos += 1,
        }
    }
    colors
}

/// The font table: `\fN` → family name.
fn parse_font_table(bytes: &[u8]) -> HashMap<i32, String> {
    const NEEDLE: &[u8] = b"\\fonttbl";
    let mut fonts = HashMap::new();
    let Some(needle_at) = bytes
        .windows(NEEDLE.len())
        .position(|window| window == NEEDLE)
    else {
        return fonts;
    };
    let mut pos = needle_at + NEEDLE.len();
    let mut depth = 1_i32;
    // Depth of a `{\*…}` (or other ignorable) group being skipped.
    let mut skip_from: Option<i32> = None;
    let mut current: Option<i32> = None;
    let mut name = String::new();
    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'{' => {
                depth += 1;
                pos += 1;
                if skip_from.is_none() && bytes.get(pos..pos + 2) == Some(b"\\*") {
                    skip_from = Some(depth);
                }
            }
            b'}' => {
                if skip_from == Some(depth) {
                    skip_from = None;
                }
                depth -= 1;
                pos += 1;
            }
            b';' if skip_from.is_none() => {
                if let Some(id) = current.take() {
                    let family = name.trim();
                    if !family.is_empty() {
                        fonts.insert(id, family.to_owned());
                    }
                }
                name.clear();
                pos += 1;
            }
            b'\\' => {
                pos += 1;
                let word_start = pos;
                while bytes.get(pos).is_some_and(u8::is_ascii_alphabetic) {
                    pos += 1;
                }
                let word = std::str::from_utf8(&bytes[word_start..pos]).unwrap_or("");
                let negative = bytes.get(pos) == Some(&b'-');
                if negative {
                    pos += 1;
                }
                let param_start = pos;
                while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
                    pos += 1;
                }
                let value = std::str::from_utf8(&bytes[param_start..pos])
                    .ok()
                    .and_then(|digits| digits.parse::<i32>().ok());
                if word.is_empty() && param_start == pos {
                    // A control symbol such as `\*`: skip its one byte.
                    pos += 1;
                } else if word == "f" && skip_from.is_none() {
                    current = value;
                    name.clear();
                }
                if bytes.get(pos) == Some(&b' ') {
                    pos += 1;
                }
            }
            b'\r' | b'\n' => pos += 1,
            byte => {
                if skip_from.is_none() && current.is_some() {
                    name.push(cp1252_to_char(byte));
                }
                pos += 1;
            }
        }
    }
    fonts
}

/// One RTF token, for the table pre-scans.
#[derive(Debug, PartialEq)]
enum Token<'a> {
    Open,
    Close,
    Word(&'a str, Option<i32>),
    /// A control symbol (`\{`, `\*`, `\~` …).
    Symbol(u8),
    /// `\'hh`.
    Hex(u8),
    Text(&'a [u8]),
}

/// A minimal RTF tokenizer over `bytes`, starting at `pos`.
struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn next_token(&mut self) -> Option<Token<'a>> {
        let bytes = self.bytes;
        loop {
            let byte = *bytes.get(self.pos)?;
            match byte {
                b'{' => {
                    self.pos += 1;
                    return Some(Token::Open);
                }
                b'}' => {
                    self.pos += 1;
                    return Some(Token::Close);
                }
                b'\r' | b'\n' => self.pos += 1,
                b'\\' => {
                    self.pos += 1;
                    let first = *bytes.get(self.pos)?;
                    if !first.is_ascii_alphabetic() {
                        self.pos += 1;
                        if first == b'\'' {
                            let hex = bytes
                                .get(self.pos..self.pos + 2)
                                .and_then(|hex| std::str::from_utf8(hex).ok())
                                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                            if let Some(value) = hex {
                                self.pos += 2;
                                return Some(Token::Hex(value));
                            }
                            continue;
                        }
                        return Some(Token::Symbol(first));
                    }
                    let start = self.pos;
                    while bytes.get(self.pos).is_some_and(u8::is_ascii_alphabetic) {
                        self.pos += 1;
                    }
                    let word = std::str::from_utf8(&bytes[start..self.pos]).unwrap_or("");
                    let negative = bytes.get(self.pos) == Some(&b'-');
                    let digits = self.pos + usize::from(negative);
                    let mut end = digits;
                    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                        end += 1;
                    }
                    let param = (end > digits)
                        .then(|| std::str::from_utf8(&bytes[digits..end]).ok())
                        .flatten()
                        .and_then(|digits| digits.parse::<i32>().ok())
                        .map(|value| if negative { -value } else { value });
                    if end > digits {
                        self.pos = end;
                    }
                    if bytes.get(self.pos) == Some(&b' ') {
                        self.pos += 1;
                    }
                    return Some(Token::Word(word, param));
                }
                _ => {
                    let start = self.pos;
                    while bytes
                        .get(self.pos)
                        .is_some_and(|byte| !matches!(byte, b'{' | b'}' | b'\\' | b'\r' | b'\n'))
                    {
                        self.pos += 1;
                    }
                    return Some(Token::Text(&bytes[start..self.pos]));
                }
            }
        }
    }
}

/// A lexer positioned just after the first `needle` control word, inside
/// its group, or `None` when the document has no such group.
fn lexer_after(bytes: &[u8], needle: &[u8]) -> Option<Lexer<'_>> {
    let mut from = 0;
    while let Some(found) = bytes[from..]
        .windows(needle.len())
        .position(|window| window == needle)
    {
        let end = from + found + needle.len();
        // A whole control word: not a prefix of a longer one.
        if !bytes.get(end).is_some_and(u8::is_ascii_alphabetic) {
            return Some(Lexer { bytes, pos: end });
        }
        from = end;
    }
    None
}

/// The list table: each `\ls` override's markers, one per level.
fn parse_list_table(bytes: &[u8]) -> HashMap<i32, Vec<Option<ListKind>>> {
    let mut lists: HashMap<i32, Vec<Option<ListKind>>> = HashMap::new();
    if let Some(mut lexer) = lexer_after(bytes, b"\\listtable") {
        let mut depth = 1_i32;
        let mut levels: Vec<Option<ListKind>> = Vec::new();
        let mut marker: Option<(i32, String)> = None;
        while depth > 0 {
            let Some(token) = lexer.next_token() else {
                break;
            };
            match token {
                Token::Open => depth += 1,
                Token::Close => {
                    if let Some((_, text)) = marker.take_if(|(at, _)| *at == depth) {
                        let name = text
                            .split_once('{')
                            .and_then(|(_, rest)| rest.split_once('}'))
                            .map_or("", |(name, _)| name);
                        if let Some(level) = levels.last_mut() {
                            *level = ListKind::from_format_name(name.trim());
                        }
                    }
                    depth -= 1;
                }
                Token::Word("list", _) => levels.clear(),
                Token::Word("listlevel", _) => levels.push(None),
                Token::Word("levelmarker", _) => marker = Some((depth, String::new())),
                Token::Word("listid", Some(id)) => {
                    lists.insert(id, levels.clone());
                }
                Token::Symbol(symbol @ (b'{' | b'}')) => {
                    if let Some((_, text)) = marker.as_mut() {
                        text.push(char::from(symbol));
                    }
                }
                Token::Text(text) => {
                    if let Some((_, marker)) = marker.as_mut() {
                        marker.push_str(&String::from_utf8_lossy(text));
                    }
                }
                _ => {}
            }
        }
    }
    let mut overrides = HashMap::new();
    if let Some(mut lexer) = lexer_after(bytes, b"\\listoverridetable") {
        let mut depth = 1_i32;
        let mut list_id = None;
        while depth > 0 {
            match lexer.next_token() {
                None => break,
                Some(Token::Open) => depth += 1,
                Some(Token::Close) => depth -= 1,
                Some(Token::Word("listid", id)) => list_id = id,
                Some(Token::Word("ls", Some(ls))) => {
                    if let Some(levels) = list_id.and_then(|id| lists.get(&id)) {
                        overrides.insert(ls, levels.clone());
                    }
                }
                Some(_) => {}
            }
        }
    }
    overrides
}

/// The `\info` group's document properties.
fn parse_info(bytes: &[u8]) -> DocumentProperties {
    let mut properties = DocumentProperties::default();
    let Some(mut lexer) = lexer_after(bytes, b"\\info") else {
        return properties;
    };
    let mut depth = 1_i32;
    // The property being read, the depth of its group, and its text.
    let mut field: Option<(&str, i32, String)> = None;
    let mut uc = 1_u32;
    let mut skip = 0_u32;
    while depth > 0 {
        let Some(token) = lexer.next_token() else {
            break;
        };
        match token {
            Token::Open => depth += 1,
            Token::Close => {
                if let Some((name, _, text)) = field.take_if(|(_, at, _)| *at == depth) {
                    let slot = match name {
                        "title" => &mut properties.title,
                        "subject" => &mut properties.subject,
                        "author" => &mut properties.author,
                        "company" => &mut properties.organisation,
                        "copyright" => &mut properties.copyright,
                        "keywords" => &mut properties.keywords,
                        _ => &mut properties.comment,
                    };
                    *slot = text.trim().to_owned();
                }
                depth -= 1;
            }
            Token::Word(
                name @ ("title" | "subject" | "author" | "company" | "copyright" | "keywords"
                | "doccomm"),
                _,
            ) if field.is_none() => field = Some((name, depth, String::new())),
            Token::Word("uc", value) => uc = value.map_or(1, |value| value.max(0) as u32),
            Token::Word("u", Some(code)) => {
                let code = if code < 0 { code + 0x1_0000 } else { code };
                if let (Some((_, _, text)), Some(character)) = (
                    field.as_mut(),
                    u32::try_from(code).ok().and_then(char::from_u32),
                ) {
                    text.push(character);
                }
                skip = uc;
            }
            Token::Hex(byte) => {
                if skip > 0 {
                    skip -= 1;
                } else if let Some((_, _, text)) = field.as_mut() {
                    text.push(cp1252_to_char(byte));
                }
            }
            Token::Symbol(symbol @ (b'{' | b'}' | b'\\')) => {
                if let Some((_, _, text)) = field.as_mut() {
                    text.push(char::from(symbol));
                }
            }
            Token::Text(bytes) => {
                let mut bytes = bytes;
                while skip > 0 && !bytes.is_empty() {
                    bytes = &bytes[1..];
                    skip -= 1;
                }
                if let Some((_, _, text)) = field.as_mut() {
                    text.extend(bytes.iter().map(|&byte| cp1252_to_char(byte)));
                }
            }
            _ => {}
        }
    }
    properties
}

/// The address a field instruction links to: `HYPERLINK "url"`.
fn hyperlink_target(instruction: &str) -> Option<Arc<str>> {
    let rest = instruction.trim_start();
    let keyword = rest.get(..9)?;
    if !keyword.eq_ignore_ascii_case("HYPERLINK") {
        return None;
    }
    let rest = rest[9..].trim();
    let target = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or(""),
        None => rest.split_whitespace().next().unwrap_or(""),
    };
    (!target.is_empty()).then(|| Arc::from(target))
}

#[derive(Clone, Debug, PartialEq)]
struct RunState {
    font: Option<i32>,
    size: f32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    color: usize,
    highlight: usize,
    outline: bool,
    /// `\expndtw` (or `\expnd` × 5): extra space, in twips.
    expand_twips: i32,
    /// `\kerning0` turns pair kerning off.
    kerning_off: bool,
    ligatures: Ligatures,
    superscript: i8,
    /// `\up` / `\dn`, in half-points (up is positive).
    baseline_half_points: i32,
    traditional: bool,
    link: Option<Arc<str>>,
}

impl RunState {
    fn new(default_font: Option<i32>) -> Self {
        Self {
            font: default_font,
            size: DEFAULT_RICH_SIZE,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            color: 0,
            highlight: 0,
            outline: false,
            expand_twips: 0,
            kerning_off: false,
            ligatures: Ligatures::Default,
            superscript: 0,
            baseline_half_points: 0,
            traditional: false,
            link: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ParagraphState {
    alignment: Alignment,
    list_override: i32,
    list_level: u8,
    line_spacing_twips: i32,
    line_spacing_multiple: bool,
}

impl Default for ParagraphState {
    fn default() -> Self {
        Self {
            alignment: Alignment::Left,
            list_override: 0,
            list_level: 0,
            line_spacing_twips: 0,
            line_spacing_multiple: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
    Text,
    /// A list marker (`\listtext`/`\pntext`): kept apart to learn the
    /// list's kind, never part of the text.
    ListText,
    /// A field's instruction (`\fldinst`): read for a link's address.
    FieldInstruction,
    Skip,
}

#[derive(Clone)]
struct GroupState {
    run: RunState,
    paragraph: ParagraphState,
    destination: Destination,
    uc: u32,
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    colors: Vec<Option<Rgb>>,
    fonts: HashMap<i32, Arc<str>>,
    lists: HashMap<i32, Vec<Option<ListKind>>>,
    default_font: Option<i32>,
    run: RunState,
    paragraph: ParagraphState,
    destination: Destination,
    uc: u32,
    pending_u_skip: u32,
    pending_high_surrogate: Option<u32>,
    stack: Vec<GroupState>,
    text: String,
    runs: Vec<StyledRun>,
    list_text: String,
    field_instruction: String,
    hyphenation: bool,
    paragraphs: Vec<Paragraph>,
    finished: bool,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let fonts = parse_font_table(bytes)
            .into_iter()
            .map(|(id, name)| (id, Arc::<str>::from(name)))
            .collect();
        Self {
            bytes,
            pos: 0,
            colors: parse_color_table(bytes),
            fonts,
            lists: parse_list_table(bytes),
            default_font: None,
            run: RunState::new(None),
            paragraph: ParagraphState::default(),
            destination: Destination::Text,
            uc: 1,
            pending_u_skip: 0,
            pending_high_surrogate: None,
            stack: Vec::new(),
            text: String::new(),
            runs: Vec::new(),
            list_text: String::new(),
            field_instruction: String::new(),
            hyphenation: false,
            paragraphs: Vec::new(),
            finished: false,
        }
    }

    fn char_style(&self) -> CharStyle {
        let family = self
            .run
            .font
            .and_then(|id| self.fonts.get(&id))
            .filter(|name| !name.eq_ignore_ascii_case(DEFAULT_FAMILY))
            .cloned();
        let color = |index: usize| self.colors.get(index).copied().flatten();
        let kern = if self.run.expand_twips != 0 {
            Some(self.run.expand_twips as f32 / 20.0)
        } else if self.run.kerning_off {
            Some(0.0)
        } else {
            None
        };
        CharStyle {
            family,
            size: self.run.size,
            bold: self.run.bold,
            italic: self.run.italic,
            underline: self.run.underline,
            strikethrough: self.run.strikethrough,
            color: if self.run.color == 0 {
                None
            } else {
                color(self.run.color)
            },
            highlight: if self.run.highlight == 0 {
                None
            } else {
                color(self.run.highlight)
            },
            outline: self.run.outline,
            kern,
            ligatures: self.run.ligatures,
            superscript: self.run.superscript,
            baseline_offset: self.run.baseline_half_points as f32 / 2.0,
            traditional: self.run.traditional,
            link: self.run.link.clone(),
        }
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.destination {
            Destination::Skip => {}
            Destination::ListText => self.list_text.push_str(text),
            Destination::FieldInstruction => {
                if self.field_instruction.len() < 4096 {
                    self.field_instruction.push_str(text);
                }
            }
            Destination::Text => {
                let style = self.char_style();
                self.text.push_str(text);
                match self.runs.last_mut() {
                    Some(last) if last.style == style => last.len += text.len(),
                    _ => self.runs.push(StyledRun {
                        len: text.len(),
                        style,
                    }),
                }
            }
        }
    }

    /// Emit decoded text, dropping the `\ucN` fallback characters still owed
    /// after a `\uN` escape.
    fn emit(&mut self, text: &str) {
        if self.pending_u_skip == 0 {
            self.push_text(text);
            return;
        }
        let mut split_at = text.len();
        for (consumed, (index, _)) in text.char_indices().enumerate() {
            if consumed as u32 == self.pending_u_skip {
                split_at = index;
                break;
            }
        }
        let consumed = text[..split_at].chars().count() as u32;
        self.pending_u_skip -= consumed.min(self.pending_u_skip);
        self.push_text(&text[split_at..]);
    }

    fn finish_paragraph(&mut self) {
        let level = self.paragraph.list_level.min(MAX_LIST_LEVEL);
        let list = (self.paragraph.list_override > 0).then(|| {
            // The list table's marker for this level, else a guess from
            // the marker text written before the item.
            self.lists
                .get(&self.paragraph.list_override)
                .and_then(|levels| levels.get(usize::from(level)).copied().flatten())
                .unwrap_or_else(|| {
                    if self.list_text.chars().any(|c| c.is_ascii_digit()) {
                        ListKind::Numbered
                    } else {
                        ListKind::Bullet
                    }
                })
        });
        let line_spacing =
            if self.paragraph.line_spacing_multiple && self.paragraph.line_spacing_twips > 0 {
                (self.paragraph.line_spacing_twips as f32 / 240.0).clamp(0.5, 4.0)
            } else {
                1.0
            };
        let style = ParagraphStyle {
            alignment: self.paragraph.alignment,
            list,
            list_level: if list.is_some() { level } else { 0 },
            line_spacing,
        };
        let mut runs = std::mem::take(&mut self.runs);
        if runs.is_empty() {
            runs.push(StyledRun {
                len: 0,
                style: self.char_style(),
            });
        }
        let text = std::mem::take(&mut self.text);
        self.paragraphs.push(Paragraph::new(text, runs, style));
        self.list_text.clear();
    }

    fn run(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'{' => {
                    self.stack.push(GroupState {
                        run: self.run.clone(),
                        paragraph: self.paragraph,
                        destination: self.destination,
                        uc: self.uc,
                    });
                    self.pos += 1;
                }
                b'}' => {
                    if self.stack.len() == 1 {
                        // The document group closes: its last paragraph ends
                        // with the properties in effect inside it.
                        self.finish_paragraph();
                        self.finished = true;
                        return;
                    }
                    if let Some(state) = self.stack.pop() {
                        self.run = state.run;
                        self.paragraph = state.paragraph;
                        self.destination = state.destination;
                        self.uc = state.uc;
                    }
                    self.pos += 1;
                }
                b'\\' => {
                    self.pos += 1;
                    self.control();
                }
                b'\r' | b'\n' => self.pos += 1,
                _ => {
                    let start = self.pos;
                    while self.pos < self.bytes.len()
                        && !matches!(self.bytes[self.pos], b'{' | b'}' | b'\\' | b'\r' | b'\n')
                    {
                        self.pos += 1;
                    }
                    let text: String = self.bytes[start..self.pos]
                        .iter()
                        .map(|&byte| cp1252_to_char(byte))
                        .collect();
                    self.pending_high_surrogate = None;
                    self.emit(&text);
                }
            }
        }
    }

    fn control(&mut self) {
        let Some(&first) = self.bytes.get(self.pos) else {
            return;
        };
        if !first.is_ascii_alphabetic() {
            self.pos += 1;
            self.symbol(first);
            return;
        }
        let word_start = self.pos;
        while self
            .bytes
            .get(self.pos)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            self.pos += 1;
        }
        let word = std::str::from_utf8(&self.bytes[word_start..self.pos])
            .unwrap_or("")
            .to_owned();
        let negative = self.bytes.get(self.pos) == Some(&b'-');
        let digits_start = self.pos + usize::from(negative);
        let mut digits_end = digits_start;
        while self.bytes.get(digits_end).is_some_and(u8::is_ascii_digit) {
            digits_end += 1;
        }
        let param = (digits_end > digits_start)
            .then(|| std::str::from_utf8(&self.bytes[digits_start..digits_end]).ok())
            .flatten()
            .and_then(|digits| digits.parse::<i32>().ok())
            .map(|value| if negative { -value } else { value });
        if digits_end > digits_start || !negative {
            self.pos = digits_end;
        }
        if self.bytes.get(self.pos) == Some(&b' ') {
            self.pos += 1;
        }
        if word != "u" {
            self.pending_high_surrogate = None;
        }
        self.word(&word, param);
    }

    fn word(&mut self, word: &str, param: Option<i32>) {
        let on = param != Some(0);
        match word {
            "par" if self.destination == Destination::Text => self.finish_paragraph(),
            "line" => self.emit("\u{2028}"),
            "page" => self.emit("\u{000C}"),
            "tab" => self.emit("\t"),
            "pard" => self.paragraph = ParagraphState::default(),
            "ql" => self.paragraph.alignment = Alignment::Left,
            "qc" => self.paragraph.alignment = Alignment::Center,
            "qr" => self.paragraph.alignment = Alignment::Right,
            "qj" => self.paragraph.alignment = Alignment::Justified,
            "ls" => self.paragraph.list_override = param.unwrap_or(0),
            "ilvl" => {
                self.paragraph.list_level =
                    param.map_or(0, |level| level.clamp(0, i32::from(MAX_LIST_LEVEL)) as u8)
            }
            "hyphauto" => self.hyphenation = on,
            "outl" => self.run.outline = on,
            "expnd" => self.run.expand_twips = param.unwrap_or(0).saturating_mul(5),
            "expndtw" => self.run.expand_twips = param.unwrap_or(0),
            "kerning" => self.run.kerning_off = param == Some(0),
            "luloligature" => {
                self.run.ligatures = match param {
                    Some(0) => Ligatures::None,
                    Some(2) => Ligatures::All,
                    _ => Ligatures::Default,
                }
            }
            "lulotraditional" => self.run.traditional = on,
            "super" => self.run.superscript = param.unwrap_or(1).clamp(0, 8) as i8,
            "sub" => self.run.superscript = -(param.unwrap_or(1).clamp(0, 8) as i8),
            "nosupersub" => self.run.superscript = 0,
            "up" => self.run.baseline_half_points = param.unwrap_or(6).clamp(-2000, 2000),
            "dn" => self.run.baseline_half_points = -param.unwrap_or(6).clamp(-2000, 2000),
            "field" => self.field_instruction.clear(),
            "fldinst" => {
                // `{\*\fldinst …}` arrives marked ignorable; read it unless
                // the whole field sits in a skipped destination.
                let parent = self.stack.last().map(|group| group.destination);
                if parent != Some(Destination::Skip) {
                    self.destination = Destination::FieldInstruction;
                    self.field_instruction.clear();
                }
            }
            "fldrslt" => {
                if self.destination == Destination::Text {
                    if let Some(target) = hyperlink_target(&self.field_instruction) {
                        self.run.link = Some(target);
                    }
                }
            }
            "sl" => self.paragraph.line_spacing_twips = param.unwrap_or(0),
            "slmult" => self.paragraph.line_spacing_multiple = on,
            "listtext" | "pntext" => self.destination = Destination::ListText,
            "plain" => self.run = RunState::new(self.default_font),
            "deff" => {
                self.default_font = param;
                self.run.font = param;
            }
            "f" => self.run.font = param,
            "b" => self.run.bold = on,
            "i" => self.run.italic = on,
            "ul" | "uld" | "uldash" | "uldb" | "ulth" | "ulw" | "ulwave" => self.run.underline = on,
            "ulnone" => self.run.underline = false,
            "strike" | "striked" => self.run.strikethrough = on,
            "fs" => {
                if let Some(half_points) = param.filter(|value| *value > 0) {
                    self.run.size = clamp_size(half_points as f32 / 2.0);
                }
            }
            "cf" => self.run.color = param.and_then(|v| usize::try_from(v).ok()).unwrap_or(0),
            "cb" | "highlight" | "chcbpat" => {
                self.run.highlight = param.and_then(|v| usize::try_from(v).ok()).unwrap_or(0)
            }
            "uc" => self.uc = param.filter(|value| *value >= 0).map_or(1, |v| v as u32),
            "u" => {
                let code = param.map(|code| if code < 0 { code + 0x1_0000 } else { code });
                if let Some(code) = code.and_then(|code| u32::try_from(code).ok()) {
                    let character = if (0xD800..0xDC00).contains(&code) {
                        self.pending_high_surrogate = Some(code);
                        None
                    } else if (0xDC00..0xE000).contains(&code) {
                        self.pending_high_surrogate.take().and_then(|high| {
                            char::from_u32(0x1_0000 + ((high - 0xD800) << 10) + (code - 0xDC00))
                        })
                    } else {
                        self.pending_high_surrogate = None;
                        char::from_u32(code)
                    };
                    if let Some(character) = character {
                        let mut buffer = [0_u8; 4];
                        self.push_text(character.encode_utf8(&mut buffer));
                    }
                }
                self.pending_u_skip = self.uc;
            }
            "emdash" => self.emit("\u{2014}"),
            "endash" => self.emit("\u{2013}"),
            "lquote" => self.emit("\u{2018}"),
            "rquote" => self.emit("\u{2019}"),
            "ldblquote" => self.emit("\u{201C}"),
            "rdblquote" => self.emit("\u{201D}"),
            "bullet" => self.emit("\u{2022}"),
            _ if SKIP_DESTINATIONS.contains(&word) => self.destination = Destination::Skip,
            _ => {}
        }
    }

    fn symbol(&mut self, symbol: u8) {
        match symbol {
            b'\'' => {
                let byte = self
                    .bytes
                    .get(self.pos..self.pos + 2)
                    .and_then(|hex| std::str::from_utf8(hex).ok())
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                if let Some(byte) = byte {
                    self.pos += 2;
                    let mut buffer = [0_u8; 4];
                    let text = cp1252_to_char(byte).encode_utf8(&mut buffer);
                    self.emit(text);
                }
            }
            // A backslash before a line break is a paragraph mark (TextEdit
            // ends every paragraph this way).
            b'\n' | b'\r' if self.destination == Destination::Text => self.finish_paragraph(),
            b'~' => self.emit("\u{00A0}"),
            b'_' => self.emit("\u{2011}"),
            b'\\' => self.emit("\\"),
            b'{' => self.emit("{"),
            b'}' => self.emit("}"),
            b'*' => self.destination = Destination::Skip,
            _ => {}
        }
    }
}

/// Parse RTF bytes into a document, or `None` if `bytes` does not start
/// with an RTF header.
pub fn parse(bytes: &[u8]) -> Option<Document> {
    let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace())?;
    if !bytes[start..].starts_with(b"{\\rtf") {
        return None;
    }
    let mut parser = Parser::new(&bytes[start..]);
    parser.run();
    if !parser.finished {
        parser.finish_paragraph();
    }
    let attributes = DocumentAttributes {
        hyphenation: parser.hyphenation,
        properties: parse_info(&bytes[start..]),
    };
    Some(Document::from_paragraphs(parser.paragraphs).with_attributes(attributes))
}

/// Character state as written, to emit only what changes between runs.
#[derive(Clone, Copy, PartialEq)]
struct Written {
    font: usize,
    half_points: i32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    color: usize,
    highlight: usize,
    outline: bool,
    /// `None` is the font's kerning; otherwise extra space in twips (0
    /// turns pair kerning off).
    kern_twips: Option<i32>,
    ligatures: Ligatures,
    superscript: i8,
    baseline_half_points: i32,
    traditional: bool,
}

impl Written {
    fn controls_from(&self, previous: Option<Written>) -> String {
        let mut controls = String::new();
        let differs = |pick: fn(&Written) -> i64| previous.map(|p| pick(&p)) != Some(pick(self));
        if differs(|w| w.font as i64) {
            let _ = write!(controls, "\\f{}", self.font);
        }
        if differs(|w| i64::from(w.half_points)) {
            let _ = write!(controls, "\\fs{}", self.half_points);
        }
        if differs(|w| i64::from(w.bold)) {
            controls.push_str(if self.bold { "\\b" } else { "\\b0" });
        }
        if differs(|w| i64::from(w.italic)) {
            controls.push_str(if self.italic { "\\i" } else { "\\i0" });
        }
        if differs(|w| i64::from(w.underline)) {
            controls.push_str(if self.underline { "\\ul" } else { "\\ulnone" });
        }
        if differs(|w| i64::from(w.strikethrough)) {
            controls.push_str(if self.strikethrough {
                "\\strike"
            } else {
                "\\strike0"
            });
        }
        if differs(|w| w.color as i64) {
            let _ = write!(controls, "\\cf{}", self.color);
        }
        if differs(|w| w.highlight as i64) {
            let _ = write!(controls, "\\cb{}", self.highlight);
        }
        if differs(|w| i64::from(w.outline)) {
            controls.push_str(if self.outline { "\\outl" } else { "\\outl0" });
        }
        // Only a document that kerns says anything about kerning, so a
        // plain document reads exactly as before.
        let kern_key = |w: &Written| w.kern_twips.map_or(i64::MIN, i64::from);
        if previous.map(|p| kern_key(&p)) != Some(kern_key(self))
            && (previous.is_some() || self.kern_twips.is_some())
        {
            match self.kern_twips {
                None => controls.push_str("\\expnd0\\expndtw0\\kerning1"),
                Some(twips) => {
                    let _ = write!(
                        controls,
                        "\\expnd{}\\expndtw{twips}\\kerning0",
                        (f64::from(twips) / 5.0).round() as i32
                    );
                }
            }
        }
        if differs(|w| w.ligatures as i64) {
            let value = match self.ligatures {
                Ligatures::None => 0,
                Ligatures::Default => 1,
                Ligatures::All => 2,
            };
            if previous.is_some() || value != 1 {
                let _ = write!(controls, "\\luloligature{value}");
            }
        }
        if differs(|w| i64::from(w.superscript)) && (previous.is_some() || self.superscript != 0) {
            match self.superscript {
                0 => controls.push_str("\\nosupersub"),
                level if level > 0 => {
                    let _ = write!(controls, "\\super{level}");
                }
                level => {
                    let _ = write!(controls, "\\sub{}", -i32::from(level));
                }
            }
        }
        if differs(|w| i64::from(w.baseline_half_points))
            && (previous.is_some() || self.baseline_half_points != 0)
        {
            if self.baseline_half_points >= 0 {
                let _ = write!(controls, "\\up{}", self.baseline_half_points);
            } else {
                let _ = write!(controls, "\\dn{}", -self.baseline_half_points);
            }
        }
        if differs(|w| i64::from(w.traditional)) && (previous.is_some() || self.traditional) {
            controls.push_str(if self.traditional {
                "\\lulotraditional"
            } else {
                "\\lulotraditional0"
            });
        }
        controls
    }
}

/// The list table: one list per marker in use, with every level showing
/// that marker, as TextEdit writes a list's levels.
fn write_list_table(out: &mut String, kinds: &[ListKind]) {
    out.push_str("{\\*\\listtable");
    for (index, kind) in kinds.iter().enumerate() {
        let id = index + 1;
        let _ = write!(out, "{{\\list\\listtemplateid{id}\\listhybrid");
        for level in 0..=MAX_LIST_LEVEL {
            let nfc = match kind {
                ListKind::Numbered => 0,
                ListKind::UpperRoman => 1,
                ListKind::LowerRoman => 2,
                ListKind::UpperAlpha => 3,
                ListKind::LowerAlpha => 4,
                _ => 23,
            };
            let indent = 720 * (u32::from(level) + 1);
            let template = id * 100 + usize::from(level) + 1;
            let _ = write!(
                out,
                "{{\\listlevel\\levelnfc{nfc}\\levelnfcn{nfc}\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat1\\levelspace360\\levelindent0{{\\*\\levelmarker \\{{{}\\}}{}}}",
                kind.format_name(),
                if kind.is_ordered() { "." } else { "" }
            );
            if kind.is_ordered() {
                let _ = write!(
                    out,
                    "{{\\leveltext\\leveltemplateid{template}\\'02\\'{level:02x}.;}}{{\\levelnumbers\\'01;}}"
                );
            } else {
                let _ = write!(out, "{{\\leveltext\\leveltemplateid{template}\\'01");
                push_unicode(out, kind.marker(1).chars().next().unwrap_or('\u{2022}'));
                out.push_str(";}{\\levelnumbers;}");
            }
            let _ = write!(out, "\\fi-360\\li{indent}\\lin{indent} }}");
        }
        let _ = write!(out, "{{\\listname ;}}\\listid{id}}}");
    }
    out.push_str("}\n{\\*\\listoverridetable");
    for index in 0..kinds.len() {
        let id = index + 1;
        let _ = write!(
            out,
            "{{\\listoverride\\listid{id}\\listoverridecount0\\ls{id}}}"
        );
    }
    out.push_str("}\n");
}

/// The `\info` group, as Cocoa writes NSDocumentAttributes.
fn write_info(out: &mut String, properties: &DocumentProperties) {
    if properties.is_empty() {
        return;
    }
    out.push_str("{\\info");
    for (word, value) in [
        ("title", &properties.title),
        ("subject", &properties.subject),
        ("author", &properties.author),
        ("*\\company", &properties.organisation),
        ("*\\copyright", &properties.copyright),
        ("keywords", &properties.keywords),
        ("doccomm", &properties.comment),
    ] {
        if value.is_empty() {
            continue;
        }
        let _ = write!(out, "\n{{\\{word} ");
        push_escaped(out, value);
        out.push('}');
    }
    out.push_str("}\n");
}

/// A link's address inside a field instruction's quotes.
fn push_field_target(out: &mut String, target: &str) {
    for character in target.chars() {
        match character {
            '"' => out.push_str("%22"),
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            c if c.is_ascii_control() => {}
            c if c.is_ascii() => out.push(c),
            c => push_unicode(out, c),
        }
    }
}

/// The document as TextEdit-compatible RTF.
pub fn write(document: &Document) -> Vec<u8> {
    let mut fonts: Vec<Arc<str>> = vec![Arc::from(DEFAULT_FAMILY)];
    let mut colors: Vec<Rgb> = Vec::new();
    let mut list_kinds: Vec<ListKind> = Vec::new();
    for paragraph in document.paragraphs() {
        if let Some(kind) = paragraph.style().list {
            if !list_kinds.contains(&kind) {
                list_kinds.push(kind);
            }
        }
        for run in paragraph.runs() {
            if let Some(family) = &run.style.family {
                if !fonts.iter().any(|known| known == family) {
                    fonts.push(family.clone());
                }
            }
            for color in [run.style.color, run.style.highlight].into_iter().flatten() {
                if !colors.contains(&color) {
                    colors.push(color);
                }
            }
        }
    }

    let mut out = String::with_capacity(document.len() + 512);
    out.push_str("{\\rtf1\\ansi\\ansicpg1252\\cocoartf2822\n");
    out.push_str("\\cocoatextscaling0\\cocoaplatform0{\\fonttbl");
    for (index, family) in fonts.iter().enumerate() {
        let _ = write!(out, "\\f{index}\\fnil\\fcharset0 ");
        push_escaped_plain(&mut out, family);
        out.push(';');
    }
    out.push_str("}\n{\\colortbl;\\red255\\green255\\blue255;");
    for color in &colors {
        let _ = write!(out, "\\red{}\\green{}\\blue{};", color.r, color.g, color.b);
    }
    out.push_str("}\n{\\*\\expandedcolortbl;;");
    for _ in &colors {
        out.push(';');
    }
    out.push_str("}\n");
    if !list_kinds.is_empty() {
        write_list_table(&mut out, &list_kinds);
    }
    let attributes = document.attributes();
    write_info(&mut out, &attributes.properties);
    out.push_str(
        "\\paperw11900\\paperh16840\\margl1440\\margr1440\\vieww11520\\viewh8400\\viewkind0\n",
    );
    if attributes.hyphenation {
        out.push_str("\\hyphauto1\\hyphfactor90\n");
    }

    let font_index = |family: &Option<Arc<str>>| {
        family
            .as_ref()
            .and_then(|family| fonts.iter().position(|known| known == family))
            .unwrap_or(0)
    };
    let color_index = |color: Option<Rgb>| {
        color
            .and_then(|color| colors.iter().position(|known| *known == color))
            .map_or(0, |index| index + 2)
    };

    // The run state the reader holds when the next run starts.
    let mut state: Option<Written> = None;
    let numbers = document.list_numbers();
    let last = document.paragraph_count() - 1;
    for (index, paragraph) in document.paragraphs().iter().enumerate() {
        let style = paragraph.style();
        out.push_str("\\pard");
        if style.list.is_some() {
            let level = u32::from(style.list_level);
            let _ = write!(
                out,
                "\\tx{}\\tx{}\\li{}\\fi-720",
                220 + 720 * level,
                720 + 720 * level,
                720 * (level + 1)
            );
        }
        out.push_str("\\pardirnatural\\partightenfactor0");
        out.push_str(match style.alignment {
            Alignment::Left => "",
            Alignment::Center => "\\qc",
            Alignment::Right => "\\qr",
            Alignment::Justified => "\\qj",
        });
        if (style.line_spacing - 1.0).abs() > f32::EPSILON {
            let _ = write!(
                out,
                "\\sl{}\\slmult1",
                (style.line_spacing * 240.0).round() as i32
            );
        }
        if let Some(kind) = style.list {
            let ls = list_kinds
                .iter()
                .position(|known| *known == kind)
                .unwrap_or(0)
                + 1;
            let _ = write!(out, "\\ls{ls}\\ilvl{}", style.list_level);
        }
        out.push('\n');
        let mut marker_pending = style.list;
        for (range, run) in paragraph.styled_ranges() {
            let wanted = Written {
                font: font_index(&run.family),
                half_points: (run.size * 2.0).round() as i32,
                bold: run.bold,
                italic: run.italic,
                underline: run.underline,
                strikethrough: run.strikethrough,
                color: color_index(run.color),
                highlight: color_index(run.highlight),
                outline: run.outline,
                kern_twips: run.kern.map(|kern| (kern * 20.0).round() as i32),
                ligatures: run.ligatures,
                superscript: run.superscript,
                baseline_half_points: (run.baseline_offset * 2.0).round() as i32,
                traditional: run.traditional,
            };
            if let Some(link) = &run.link {
                // A link is a field; what is set inside its result group
                // ends with the group, so the outer state stays as it was.
                if let Some(kind) = marker_pending.take() {
                    let controls = wanted.controls_from(state);
                    state = Some(wanted);
                    if !controls.is_empty() {
                        out.push_str(&controls);
                        out.push(' ');
                    }
                    push_list_text(&mut out, kind, numbers[index]);
                }
                out.push_str("{\\field{\\*\\fldinst{HYPERLINK \"");
                push_field_target(&mut out, link);
                out.push_str("\"}}{\\fldrslt ");
                let controls = wanted.controls_from(state);
                if !controls.is_empty() {
                    out.push_str(&controls);
                    out.push(' ');
                }
                push_escaped(&mut out, &paragraph.text()[range]);
                out.push_str("}}");
                continue;
            }
            let controls = wanted.controls_from(state);
            state = Some(wanted);
            if !controls.is_empty() {
                out.push_str(&controls);
                out.push(' ');
            }
            if let Some(kind) = marker_pending.take() {
                push_list_text(&mut out, kind, numbers[index]);
            }
            push_escaped(&mut out, &paragraph.text()[range]);
        }
        if index < last {
            out.push_str("\\\n");
        }
    }
    out.push('}');
    out.into_bytes()
}

/// TextEdit's `{\listtext\t<marker>\t}` before a list item's text.
fn push_list_text(out: &mut String, kind: ListKind, number: u32) {
    out.push_str("{\\listtext\t");
    for character in kind.marker(number).chars() {
        if character.is_ascii() {
            out.push(character);
        } else {
            push_unicode(out, character);
        }
    }
    out.push_str("\t}");
}

fn push_escaped_plain(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '\\' | '{' | '}' | ';' => {}
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => push_unicode(out, c),
        }
    }
}

/// `\uc0` then one signed `\uN` per UTF-16 unit, so a surrogate pair
/// stays adjacent for the reader to join.
fn push_unicode(out: &mut String, character: char) {
    let mut units = [0_u16; 2];
    out.push_str("\\uc0");
    for unit in character.encode_utf16(&mut units) {
        let value = i32::from(*unit);
        let signed = if value > 0x7FFF {
            value - 0x1_0000
        } else {
            value
        };
        let _ = write!(out, "\\u{signed} ");
    }
}

fn push_escaped(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            '\u{2028}' => out.push_str("\\line "),
            '\u{000C}' => out.push_str("\\page "),
            '\n' | '\r' => out.push_str("\\line "),
            c if c.is_ascii_control() => {}
            c if c.is_ascii() => out.push(c),
            c => push_unicode(out, c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    fn plain(text: &str) -> Document {
        Document::from_plain_text(text, &CharStyle::default())
    }

    #[test]
    fn rejects_data_without_the_rtf_header() {
        assert!(parse(b"this is not rtf at all").is_none());
        assert!(parse(b"").is_none());
    }

    #[test]
    fn reads_plain_paragraphs_and_textedit_line_ends() {
        let document = parse(b"{\\rtf1\\ansi\\deff0 Hello world\\par Second\\\nThird}").unwrap();
        assert_eq!(document.text(), "Hello world\nSecond\nThird");
    }

    #[test]
    fn reads_character_styles_sizes_and_colours() {
        let rtf = br"{\rtf1\ansi{\colortbl;\red255\green0\blue0;\red0\green0\blue255;}\fs36\cf1 Big red\cf2\cb1  blue \b bold\b0  \i it\i0  \ul u\ulnone  \strike s\strike0}";
        let document = parse(rtf).unwrap();
        assert_eq!(document.text(), "Big red blue bold it u s");
        let red = document.style_of_char_at(0);
        assert_eq!(red.color, Some(Rgb::new(255, 0, 0)));
        assert_eq!(red.size, 18.0);
        let blue = document.style_of_char_at(8);
        assert_eq!(blue.color, Some(Rgb::new(0, 0, 255)));
        assert_eq!(blue.highlight, Some(Rgb::new(255, 0, 0)));
        assert!(document.style_of_char_at(13).bold);
        assert!(document.style_of_char_at(18).italic);
        assert!(document.style_of_char_at(21).underline);
        assert!(document.style_of_char_at(23).strikethrough);
    }

    #[test]
    fn reads_alignment_spacing_and_textedit_lists() {
        let rtf = b"{\\rtf1\\ansi\\pard\\qc centred\\par\\pard\\qj\\sl360\\slmult1 justified\\par\\pard\\tx220\\tx720\\li720\\fi-720\\ls1\\ilvl0 {\\listtext\t\\uc0\\u8226 \t}one\\\n{\\listtext\t\\uc0\\u8226 \t}two\\\n\\pard\\ls2\\ilvl0 {\\listtext\t1.\t}first}";
        let document = parse(rtf).unwrap();
        assert_eq!(document.text(), "centred\njustified\none\ntwo\nfirst");
        assert_eq!(document.paragraph(0).style().alignment, Alignment::Center);
        assert_eq!(
            document.paragraph(1).style().alignment,
            Alignment::Justified
        );
        assert_eq!(document.paragraph(1).style().line_spacing, 1.5);
        assert_eq!(document.paragraph(2).style().list, Some(ListKind::Bullet));
        assert_eq!(document.paragraph(3).style().list, Some(ListKind::Bullet));
        assert_eq!(document.paragraph(4).style().list, Some(ListKind::Numbered));
    }

    #[test]
    fn reads_unicode_hex_escapes_and_surrogate_pairs() {
        let rtf = "{\\rtf1\\ansi \\uc1\\u8364? caf\\'e9 \\uc0\\u-10179 \\u-8704 }";
        let document = parse(rtf.as_bytes()).unwrap();
        assert_eq!(document.text(), "\u{20AC} caf\u{e9} \u{1F600}");
    }

    #[test]
    fn skips_tables_and_unknown_destinations_but_keeps_field_results() {
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\colortbl;\red255\green0\blue0;}{\*\generator Riched20}{\info{\author Someone}}Before {\field{\*\fldinst HYPERLINK ignored }{\fldrslt visible}} after}";
        let document = parse(rtf).unwrap();
        assert_eq!(document.text(), "Before visible after");
    }

    #[test]
    fn reads_font_families_from_the_font_table() {
        let rtf = br"{\rtf1\ansi\deff0{\fonttbl\f0\fswiss\fcharset0 Helvetica;\f1\fmodern\fcharset0 Menlo-Regular;}\f0 a\f1 b}";
        let document = parse(rtf).unwrap();
        assert_eq!(document.style_of_char_at(0).family, None);
        assert_eq!(
            document.style_of_char_at(1).family.as_deref(),
            Some("Menlo-Regular")
        );
    }

    #[test]
    fn a_styled_document_round_trips_exactly() {
        let mut document = plain("Plain bold italic\nCentred {braces} \\ caf\u{e9} \u{1F600}\n\tTabbed\u{2028}line\n\nitem one\nitem two\nstep");
        document.update_char_style(6..10, |style| style.bold = true);
        document.update_char_style(11..17, |style| {
            style.italic = true;
            style.underline = true;
            style.size = 18.0;
        });
        document.update_char_style(18..25, |style| {
            style.color = Some(Rgb::new(200, 30, 40));
            style.highlight = Some(Rgb::new(255, 240, 120));
            style.strikethrough = true;
            style.family = Some(Arc::from("Menlo"));
        });
        document.update_paragraph_style(18..18, |style| style.alignment = Alignment::Center);
        let tabbed = document.paragraph_start(2);
        document.update_paragraph_style(tabbed..tabbed, |style| {
            style.alignment = Alignment::Justified;
            style.line_spacing = 2.0;
        });
        let items = document.paragraph_start(4);
        let last = document.len();
        document.update_paragraph_style(items..items + 10, |style| {
            style.list = Some(ListKind::Bullet)
        });
        document.update_paragraph_style(last..last, |style| {
            style.list = Some(ListKind::Numbered);
            style.alignment = Alignment::Right;
        });

        let bytes = write(&document);
        assert!(bytes.is_ascii());
        let read = parse(&bytes).expect("written RTF parses");
        assert_eq!(read.text(), document.text());
        for index in 0..document.paragraph_count() {
            assert_eq!(
                read.paragraph(index),
                document.paragraph(index),
                "paragraph {index}"
            );
        }
        assert_eq!(read, document);
    }

    #[test]
    fn an_empty_document_round_trips() {
        let document = plain("");
        assert_eq!(parse(&write(&document)).unwrap(), document);
    }

    #[test]
    fn written_lists_use_textedit_markers() {
        let mut document = plain("a\nb");
        document.update_paragraph_style(0..3, |style| style.list = Some(ListKind::Numbered));
        let text = String::from_utf8(write(&document)).unwrap();
        assert!(text.contains("\\ls1\\ilvl0"));
        assert!(text.contains("{\\listtext\t1.\t}"));
        assert!(text.contains("{\\listtext\t2.\t}"));
    }

    fn assert_round_trips(document: &Document) -> Document {
        let bytes = write(document);
        assert!(bytes.is_ascii());
        let read = parse(&bytes).expect("written RTF parses");
        for index in 0..document.paragraph_count() {
            assert_eq!(
                read.paragraph(index),
                document.paragraph(index),
                "paragraph {index}"
            );
        }
        assert_eq!(read.attributes(), document.attributes());
        assert_eq!(&read, document);
        read
    }

    #[test]
    fn font_menu_attributes_round_trip() {
        let mut document =
            plain("outline kern none tight loose lig none lig all sup sub up down trad");
        let set = |document: &mut Document, range: Range<usize>, change: fn(&mut CharStyle)| {
            document.update_char_style(range, change)
        };
        set(&mut document, 0..7, |s| s.outline = true);
        set(&mut document, 8..17, |s| s.kern = Some(0.0));
        set(&mut document, 18..23, |s| s.kern = Some(-1.0));
        set(&mut document, 24..29, |s| s.kern = Some(2.0));
        set(&mut document, 30..38, |s| s.ligatures = Ligatures::None);
        set(&mut document, 39..46, |s| s.ligatures = Ligatures::All);
        set(&mut document, 47..50, |s| s.superscript = 1);
        set(&mut document, 51..54, |s| s.superscript = -2);
        set(&mut document, 55..57, |s| s.baseline_offset = 3.0);
        set(&mut document, 58..62, |s| s.baseline_offset = -1.5);
        set(&mut document, 63..67, |s| s.traditional = true);
        let read = assert_round_trips(&document);
        assert!(read.style_of_char_at(0).outline);
        assert_eq!(read.style_of_char_at(8).kern, Some(0.0));
        assert_eq!(read.style_of_char_at(24).kern, Some(2.0));
        assert_eq!(read.style_of_char_at(47).superscript, 1);
        assert_eq!(read.style_of_char_at(58).baseline_offset, -1.5);
        assert_eq!(read.style_of_char_at(7).kern, None);
    }

    #[test]
    fn reads_textedit_kerning_and_baseline_controls() {
        let rtf = br"{\rtf1\ansi \expnd0\expndtw0\kerning0 none\expnd-4\expndtw-20 tight\expnd0\expndtw0\kerning1 \super sup\nosupersub\up6 up\up0 \sub sub}";
        let document = parse(rtf).unwrap();
        assert_eq!(document.text(), "nonetightsupupsub");
        assert_eq!(document.style_of_char_at(0).kern, Some(0.0));
        assert_eq!(document.style_of_char_at(4).kern, Some(-1.0));
        assert_eq!(document.style_of_char_at(9).kern, None);
        assert_eq!(document.style_of_char_at(9).superscript, 1);
        assert_eq!(document.style_of_char_at(12).baseline_offset, 3.0);
        assert_eq!(document.style_of_char_at(12).superscript, 0);
        assert_eq!(document.style_of_char_at(14).superscript, -1);
    }

    #[test]
    fn nested_lists_keep_their_markers_and_levels() {
        let mut document = plain("one\ninner\ninner two\ntwo\nroman\nletter");
        document.update_paragraph_style(0..30, |style| style.list = Some(ListKind::Numbered));
        document.update_paragraph_style(4..19, |style| {
            style.list = Some(ListKind::Circle);
            style.list_level = 1;
        });
        let roman = document.paragraph_start(4);
        document.update_paragraph_style(roman..roman, |style| {
            style.list = Some(ListKind::UpperRoman)
        });
        let letter = document.paragraph_start(5);
        document.update_paragraph_style(letter..letter, |style| {
            style.list = Some(ListKind::LowerAlpha);
            style.list_level = 2;
        });
        let read = assert_round_trips(&document);
        assert_eq!(read.paragraph(1).style().list, Some(ListKind::Circle));
        assert_eq!(read.paragraph(1).style().list_level, 1);
        assert_eq!(read.paragraph(5).style().list_level, 2);
        let text = String::from_utf8(write(&document)).unwrap();
        assert!(text.contains("\\levelmarker \\{circle\\}"));
        assert!(text.contains("\\levelmarker \\{upper-roman\\}."));
        assert!(text.contains("\\ilvl1"));
    }

    #[test]
    fn reads_a_textedit_list_table() {
        let rtf = b"{\\rtf1\\ansi{\\*\\listtable{\\list\\listtemplateid1\\listhybrid{\\listlevel\\levelnfc23{\\*\\levelmarker \\{square\\}}{\\leveltext\\'01\\uc0\\u9642 ;}{\\levelnumbers;}\\fi-360\\li720\\lin720 }{\\listlevel\\levelnfc4{\\*\\levelmarker \\{lower-alpha\\}.}{\\leveltext\\'02\\'01.;}{\\levelnumbers\\'01;}\\fi-360\\li1440\\lin1440 }{\\listname ;}\\listid7}}\n{\\*\\listoverridetable{\\listoverride\\listid7\\listoverridecount0\\ls1}}\n\\pard\\ls1\\ilvl0 {\\listtext\t\\uc0\\u9642 \t}top\\\n\\ls1\\ilvl1 {\\listtext\ta.\t}under}";
        let document = parse(rtf).unwrap();
        assert_eq!(document.text(), "top\nunder");
        assert_eq!(document.paragraph(0).style().list, Some(ListKind::Square));
        assert_eq!(
            document.paragraph(1).style().list,
            Some(ListKind::LowerAlpha)
        );
        assert_eq!(document.paragraph(1).style().list_level, 1);
    }

    #[test]
    fn links_round_trip_as_hyperlink_fields() {
        let mut document = plain("see the site now");
        document.update_char_style(4..12, |style| {
            style.link = Some(Arc::from("https://example.com/a?b=\"c\""))
        });
        document.update_char_style(8..12, |style| style.bold = true);
        let bytes = write(&document);
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(
            text.contains("{\\field{\\*\\fldinst{HYPERLINK \"https://example.com/a?b=%22c%22\"}}")
        );
        let read = parse(&bytes).unwrap();
        assert_eq!(read.text(), "see the site now");
        assert_eq!(
            read.style_of_char_at(4).link.as_deref(),
            Some("https://example.com/a?b=%22c%22")
        );
        assert!(read.style_of_char_at(9).bold);
        assert_eq!(read.style_of_char_at(13).link, None);
        assert!(!read.style_of_char_at(13).bold);
    }

    #[test]
    fn hyphenation_and_properties_round_trip() {
        let document = plain("text").with_attributes(DocumentAttributes {
            hyphenation: true,
            properties: DocumentProperties {
                author: "Jo {Q} Smith".into(),
                organisation: "Lulo".into(),
                copyright: "\u{a9} 2026".into(),
                title: "Caf\u{e9} notes".into(),
                subject: "Tests".into(),
                keywords: "one, two".into(),
                comment: "back\\slash".into(),
            },
        });
        let read = assert_round_trips(&document);
        assert!(read.attributes().hyphenation);
        assert_eq!(read.attributes().properties.title, "Caf\u{e9} notes");
        assert_eq!(read.text(), "text");
    }

    #[test]
    fn plain_documents_write_no_new_controls() {
        let text = String::from_utf8(write(&plain("hello"))).unwrap();
        for control in [
            "\\expnd",
            "\\luloligature",
            "\\super",
            "\\up",
            "\\lulotraditional",
            "\\hyphauto",
            "\\info",
            "\\outl",
        ] {
            assert!(!text.contains(control), "{control}");
        }
    }
}
