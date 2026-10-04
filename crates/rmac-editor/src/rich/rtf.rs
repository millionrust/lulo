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

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use super::model::{
    clamp_size, Alignment, CharStyle, Document, ListKind, Paragraph, ParagraphStyle, Rgb,
    StyledRun, DEFAULT_RICH_SIZE,
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
    "fldinst",
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct RunState {
    font: Option<i32>,
    size: f32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    color: usize,
    highlight: usize,
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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ParagraphState {
    alignment: Alignment,
    list_override: i32,
    line_spacing_twips: i32,
    line_spacing_multiple: bool,
}

impl Default for ParagraphState {
    fn default() -> Self {
        Self {
            alignment: Alignment::Left,
            list_override: 0,
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
    Skip,
}

#[derive(Clone, Copy)]
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
        }
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.destination {
            Destination::Skip => {}
            Destination::ListText => self.list_text.push_str(text),
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
        let remainder = text[split_at..].to_owned();
        self.push_text(&remainder);
    }

    fn finish_paragraph(&mut self) {
        let list = (self.paragraph.list_override > 0).then(|| {
            if self.list_text.chars().any(|c| c.is_ascii_digit()) {
                ListKind::Numbered
            } else {
                ListKind::Bullet
            }
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
                        run: self.run,
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
                    let text = cp1252_to_char(byte).encode_utf8(&mut buffer).to_owned();
                    self.emit(&text);
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
    Some(Document::from_paragraphs(parser.paragraphs))
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
        controls
    }
}

/// The document as TextEdit-compatible RTF.
pub fn write(document: &Document) -> Vec<u8> {
    let mut fonts: Vec<Arc<str>> = vec![Arc::from(DEFAULT_FAMILY)];
    let mut colors: Vec<Rgb> = Vec::new();
    let mut lists = (false, false);
    for paragraph in document.paragraphs() {
        match paragraph.style().list {
            Some(ListKind::Bullet) => lists.0 = true,
            Some(ListKind::Numbered) => lists.1 = true,
            None => {}
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
    if lists.0 || lists.1 {
        out.push_str("{\\*\\listtable");
        out.push_str(
            "{\\list\\listtemplateid1\\listhybrid{\\listlevel\\levelnfc23\\levelnfcn23\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat1\\levelspace360\\levelindent0{\\*\\levelmarker \\{disc\\}}{\\leveltext\\leveltemplateid1\\'01\\uc0\\u8226 ;}{\\levelnumbers;}\\fi-360\\li720\\lin720 }{\\listname ;}\\listid1}",
        );
        out.push_str(
            "{\\list\\listtemplateid2\\listhybrid{\\listlevel\\levelnfc0\\levelnfcn0\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat1\\levelspace360\\levelindent0{\\*\\levelmarker \\{decimal\\}.}{\\leveltext\\leveltemplateid101\\'02\\'00.;}{\\levelnumbers\\'01;}\\fi-360\\li720\\lin720 }{\\listname ;}\\listid2}}\n",
        );
        out.push_str("{\\*\\listoverridetable{\\listoverride\\listid1\\listoverridecount0\\ls1}{\\listoverride\\listid2\\listoverridecount0\\ls2}}\n");
    }
    out.push_str(
        "\\paperw11900\\paperh16840\\margl1440\\margr1440\\vieww11520\\viewh8400\\viewkind0\n",
    );

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
            out.push_str("\\tx220\\tx720\\li720\\fi-720");
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
        match style.list {
            Some(ListKind::Bullet) => out.push_str("\\ls1\\ilvl0"),
            Some(ListKind::Numbered) => out.push_str("\\ls2\\ilvl0"),
            None => {}
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
            };
            let controls = wanted.controls_from(state);
            state = Some(wanted);
            if !controls.is_empty() {
                out.push_str(&controls);
                out.push(' ');
            }
            if let Some(kind) = marker_pending.take() {
                out.push_str("{\\listtext\t");
                match kind {
                    ListKind::Bullet => out.push_str("\\uc0\\u8226 "),
                    ListKind::Numbered => {
                        let _ = write!(out, "{}.", numbers[index]);
                    }
                }
                out.push_str("\t}");
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
        assert!(text.contains("\\ls2\\ilvl0"));
        assert!(text.contains("{\\listtext\t1.\t}"));
        assert!(text.contains("{\\listtext\t2.\t}"));
    }
}
