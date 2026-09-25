//! Read-only RTF support: parse an `.rtf` file into styled text runs using
//! macOS's own `NSAttributedString` RTF reader, so the editor can render a
//! faithful formatted preview (bold / italic / underline / color).
//!
//! GPUI has no editable rich-text widget, so this is a *viewer* — editing still
//! happens on the plain-text body. Per-run font size isn't carried by GPUI's
//! `TextRun`, so size variation is not reflected; weight/style/underline/color
//! are.

/// One contiguous run of same-styled text from an RTF document.
#[derive(Clone, Debug, PartialEq)]
pub struct RtfRun {
    pub text: String,
    /// The run's font family (e.g. "Helvetica"), so bold/italic faces render —
    /// GPUI won't synthesize a missing weight, so we use the document's own font.
    pub family: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Option<(u8, u8, u8)>,
    /// Point size (`\fsN` is in half-points), when the document sets one.
    /// Like `family`, GPUI's `TextRun` carries no per-run size, so the
    /// preview uses one dominant size for the whole document rather than
    /// varying it run by run (see `view/render/rtf.rs`).
    pub size: Option<f32>,
}

/// Parse RTF bytes into styled runs. Returns `None` if the data isn't valid
/// RTF. On macOS this asks AppKit's own reader; everywhere else (and in this
/// module's own tests) it uses [`portable::parse_rtf`], a pure-Rust reader
/// for RTF's common subset — see that module for exactly what it supports.
#[cfg(target_os = "macos")]
pub fn parse_rtf(bytes: &[u8]) -> Option<Vec<RtfRun>> {
    imp::parse_rtf(bytes)
}

#[cfg(not(target_os = "macos"))]
pub fn parse_rtf(bytes: &[u8]) -> Option<Vec<RtfRun>> {
    portable::parse_rtf(bytes)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::RtfRun;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSAttributedStringAppKitDocumentFormats, NSColor, NSColorSpace, NSFont,
        NSFontAttributeName, NSFontDescriptorSymbolicTraits, NSForegroundColorAttributeName,
        NSUnderlineStyleAttributeName,
    };
    use objc2_foundation::{NSAttributedString, NSData, NSNumber, NSRange, NSString};

    pub fn parse_rtf(bytes: &[u8]) -> Option<Vec<RtfRun>> {
        let data = NSData::with_bytes(bytes);
        let attr: Retained<NSAttributedString> = unsafe {
            NSAttributedString::initWithRTF_documentAttributes(
                NSAttributedString::alloc(),
                &data,
                None,
            )
        }?;

        let ns = attr.string();
        let total = attr.length();
        if total == 0 {
            return Some(Vec::new());
        }

        // Accessing these AppKit extern statics is unsafe; bind once.
        let (font_name, underline_name, color_name) = unsafe {
            (
                NSFontAttributeName,
                NSUnderlineStyleAttributeName,
                NSForegroundColorAttributeName,
            )
        };

        let mut runs: Vec<RtfRun> = Vec::new();
        let mut i: usize = 0;
        while i < total {
            let limit = NSRange {
                location: i,
                length: total - i,
            };
            let mut eff = NSRange {
                location: 0,
                length: 0,
            };

            // Split on the *whole* attribute dictionary so a run is constant in
            // font, color, and underline together — otherwise a color change
            // inside one font would be missed.
            let _attrs =
                unsafe { attr.attributesAtIndex_longestEffectiveRange_inRange(i, &mut eff, limit) };
            let run_range = if eff.length == 0 {
                NSRange {
                    location: i,
                    length: total - i,
                }
            } else {
                eff
            };

            let font_obj = attr_at(&attr, font_name, i, limit);

            let text = ns.substringWithRange(run_range).to_string();

            let (mut bold, mut italic, mut family) = (false, false, None);
            if let Some(obj) = font_obj {
                if let Ok(font) = obj.downcast::<NSFont>() {
                    let traits = font.fontDescriptor().symbolicTraits();
                    bold = traits.contains(NSFontDescriptorSymbolicTraits::TraitBold);
                    italic = traits.contains(NSFontDescriptorSymbolicTraits::TraitItalic);
                    family = font.familyName().map(|n| n.to_string());
                }
            }

            let underline = attr_at(&attr, underline_name, i, limit)
                .and_then(|o| o.downcast::<NSNumber>().ok())
                .map(|n| n.integerValue() != 0)
                .unwrap_or(false);

            let color = attr_at(&attr, color_name, i, limit)
                .and_then(|o| o.downcast::<NSColor>().ok())
                .and_then(|c| color_rgb(&c));

            runs.push(RtfRun {
                text,
                family,
                bold,
                italic,
                underline,
                color,
                size: None,
            });
            i += run_range.length.max(1);
        }
        Some(runs)
    }

    /// Fetch a single attribute object at `i`, ignoring its effective range.
    fn attr_at(
        attr: &NSAttributedString,
        name: &NSString,
        i: usize,
        limit: NSRange,
    ) -> Option<Retained<AnyObject>> {
        let mut scratch = NSRange {
            location: 0,
            length: 0,
        };
        unsafe {
            attr.attribute_atIndex_longestEffectiveRange_inRange(name, i, &mut scratch, limit)
        }
    }

    fn color_rgb(c: &NSColor) -> Option<(u8, u8, u8)> {
        let space = NSColorSpace::sRGBColorSpace();
        let rgb = c.colorUsingColorSpace(&space)?;
        let to_u8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some((
            to_u8(rgb.redComponent()),
            to_u8(rgb.greenComponent()),
            to_u8(rgb.blueComponent()),
        ))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn parses_bold_italic_runs() {
        // Minimal RTF: plain "Hi " then bold "bold" then plain " and " then italic "it".
        let rtf = br"{\rtf1\ansi {Hi \b bold\b0  and \i it\i0 }}";
        let runs = parse_rtf(rtf).expect("valid rtf");
        let joined: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert!(joined.contains("bold"), "text preserved: {joined:?}");
        assert!(
            runs.iter().any(|r| r.bold && r.text.contains("bold")),
            "{runs:?}"
        );
        assert!(
            runs.iter().any(|r| r.italic && r.text.contains("it")),
            "{runs:?}"
        );
    }

    #[test]
    fn rejects_non_rtf() {
        assert!(parse_rtf(b"this is not rtf at all").is_none());
    }
}

/// A portable (pure-Rust) RTF reader for the subset TE-03 asks for: plain
/// text, paragraphs, bold/italic/underline, font size, colour, and the
/// `\'hh`/`\uN` escapes. It has no platform dependency, so it also parses
/// RTF on Linux, where AppKit's own reader ([`imp`]) is unavailable.
///
/// What it does *not* attempt: font family (a run's `family` is always
/// `None`, matching the base UI font, exactly as `imp` falls back when a
/// run's font is missing), alignment, lists, tables, embedded pictures and
/// objects, fields, and bookmarks/annotations. An unrecognized destination
/// group (anything opened with `\*`, or one of [`SKIP_DESTINATIONS`]) is
/// skipped whole rather than spilled into the document as literal text —
/// that is what keeps a font or color table, a field's instruction code, or
/// an embedded picture's binary data out of the parsed text.
// On macOS, `parse_rtf` above calls `imp` and never reaches this module, so
// its own items look unused to this platform's build; they are the real
// `parse_rtf` everywhere else. `#[cfg(test)]` still exercises them here too
// (see this module's own tests), which is exactly why the platform split is
// no wider than the two `parse_rtf` bodies.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub mod portable {
    use super::RtfRun;

    /// Destinations whose content is never part of the visible document,
    /// whether or not they are also marked `\*` (many writers omit the `\*`
    /// on `fonttbl`/`colortbl`/`info`, so they need to be named explicitly).
    const SKIP_DESTINATIONS: &[&str] = &[
        "fonttbl",
        "colortbl",
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
    ];

    /// Maps a raw byte the parser met outside any `\'hh`/`\uN` escape (plain
    /// RTF text is 7-bit ASCII, so seeing one at all is already a writer
    /// that bent the rules) or an `\'hh` escape's byte value to a Unicode
    /// scalar, treating the byte as Windows-1252 — the encoding RTF's own
    /// `\ansicpg1252`, by far the common case, names. Bytes below 0x80 and
    /// 0xA0 and above match Latin-1 (and so plain ASCII) exactly; only
    /// 0x80..=0x9F differ.
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
            // 0x81, 0x8D, 0x8F, 0x90, 0x9D are undefined in Windows-1252;
            // falling back to Latin-1 identity keeps this total.
            other => other as char,
        }
    }

    /// `bytes` decoded a byte at a time as Windows-1252/Latin-1. Used for a
    /// plain-text run (7-bit ASCII in a well-formed document) so a
    /// non-conforming writer's stray high byte cannot make the whole parse
    /// fail or panic.
    fn decode_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|&byte| cp1252_to_char(byte)).collect()
    }

    /// A finished color table entry, `;`-terminated, before it is added to
    /// `colors`: `None` until a `\redN`/`\greenN`/`\blueN` sets it.
    #[derive(Clone, Copy, Default)]
    struct ColorEntry {
        red: Option<u8>,
        green: Option<u8>,
        blue: Option<u8>,
    }

    impl ColorEntry {
        fn finish(self) -> Option<(u8, u8, u8)> {
            match (self.red, self.green, self.blue) {
                (Some(r), Some(g), Some(b)) => Some((r, g, b)),
                // No component at all is the table's implicit "auto" entry
                // (conventionally index 0); a partial one is malformed —
                // both are safest read as "no explicit color".
                _ => None,
            }
        }
    }

    /// The document's color table (`\cfN` indexes into it, 0-based), read as
    /// its own pass so the main parser can simply skip `colortbl` like any
    /// other non-visible destination.
    fn parse_color_table(bytes: &[u8]) -> Vec<Option<(u8, u8, u8)>> {
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
        let mut current = ColorEntry::default();
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
                    colors.push(current.finish());
                    current = ColorEntry::default();
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
                        .and_then(|digits| digits.parse::<u16>().ok())
                        .map(|value| value.min(255) as u8);
                    match word {
                        "red" => current.red = value,
                        "green" => current.green = value,
                        "blue" => current.blue = value,
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

    #[derive(Clone, Copy, Default, PartialEq)]
    struct RunStyle {
        bold: bool,
        italic: bool,
        underline: bool,
        color: Option<(u8, u8, u8)>,
        size: Option<f32>,
    }

    struct Parser<'a> {
        bytes: &'a [u8],
        pos: usize,
        colors: Vec<Option<(u8, u8, u8)>>,
        runs: Vec<RtfRun>,
        style: RunStyle,
        /// True inside a destination whose content never reaches the
        /// document (a font/color/style table, `\*` group, field code, …).
        skip: bool,
        /// `\ucN`: how many characters after a `\uN` escape are the
        /// non-Unicode fallback, and so are not part of the document.
        uc: u32,
        /// Fallback characters still owed after the last `\uN` escape.
        pending_u_skip: u32,
        /// One saved `(style, skip, uc)` per open group, restored at `}`.
        stack: Vec<(RunStyle, bool, u32)>,
    }

    impl<'a> Parser<'a> {
        fn new(bytes: &'a [u8], colors: Vec<Option<(u8, u8, u8)>>) -> Self {
            Self {
                bytes,
                pos: 0,
                colors,
                runs: Vec::new(),
                style: RunStyle::default(),
                skip: false,
                uc: 1,
                pending_u_skip: 0,
                stack: Vec::new(),
            }
        }

        fn push_text(&mut self, text: &str) {
            if text.is_empty() {
                return;
            }
            let continues = self.runs.last().is_some_and(|run| {
                run.bold == self.style.bold
                    && run.italic == self.style.italic
                    && run.underline == self.style.underline
                    && run.color == self.style.color
                    && run.size == self.style.size
            });
            if continues {
                self.runs.last_mut().expect("checked above").text.push_str(text);
            } else {
                self.runs.push(RtfRun {
                    text: text.to_owned(),
                    family: None,
                    bold: self.style.bold,
                    italic: self.style.italic,
                    underline: self.style.underline,
                    color: self.style.color,
                    size: self.style.size,
                });
            }
        }

        /// Emit already-decoded text, honoring a `\uN` escape's still-owed
        /// fallback characters and this group's skip state.
        fn emit_str(&mut self, text: &str) {
            if text.is_empty() {
                return;
            }
            if self.pending_u_skip > 0 {
                let mut consumed = 0_u32;
                let mut split_at = text.len();
                for (index, _) in text.char_indices() {
                    if consumed == self.pending_u_skip {
                        split_at = index;
                        break;
                    }
                    consumed += 1;
                }
                self.pending_u_skip -= consumed;
                let remainder = &text[split_at..];
                if self.pending_u_skip > 0 || remainder.is_empty() {
                    return;
                }
                if !self.skip {
                    self.push_text(remainder);
                }
                return;
            }
            if !self.skip {
                self.push_text(text);
            }
        }

        fn run(&mut self) {
            while self.pos < self.bytes.len() {
                match self.bytes[self.pos] {
                    b'{' => {
                        self.stack.push((self.style, self.skip, self.uc));
                        self.pos += 1;
                    }
                    b'}' => {
                        if let Some((style, skip, uc)) = self.stack.pop() {
                            self.style = style;
                            self.skip = skip;
                            self.uc = uc;
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
                        let text = decode_bytes(&self.bytes[start..self.pos]);
                        self.emit_str(&text);
                    }
                }
            }
        }

        fn control(&mut self) {
            let Some(&first) = self.bytes.get(self.pos) else {
                return;
            };
            if first.is_ascii_alphabetic() {
                let word_start = self.pos;
                while self.bytes.get(self.pos).is_some_and(u8::is_ascii_alphabetic) {
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
                self.pos = digits_end;
                if self.bytes.get(self.pos) == Some(&b' ') {
                    self.pos += 1;
                }
                self.dispatch_word(&word, param);
            } else {
                self.pos += 1;
                self.dispatch_symbol(first);
            }
        }

        fn dispatch_word(&mut self, word: &str, param: Option<i32>) {
            match word {
                "par" | "line" => self.emit_str("\n"),
                "tab" => self.emit_str("\t"),
                "b" => self.style.bold = param != Some(0),
                "i" => self.style.italic = param != Some(0),
                "ul" => self.style.underline = param != Some(0),
                "ulnone" => self.style.underline = false,
                "plain" => self.style = RunStyle::default(),
                "fs" => {
                    if let Some(half_points) = param.filter(|value| *value > 0) {
                        self.style.size = Some(half_points as f32 / 2.0);
                    }
                }
                "cf" => {
                    let index = param.filter(|value| *value >= 0).unwrap_or(-1);
                    self.style.color = usize::try_from(index)
                        .ok()
                        .and_then(|index| self.colors.get(index))
                        .copied()
                        .flatten();
                }
                "uc" => self.uc = param.filter(|value| *value >= 0).map_or(1, |v| v as u32),
                "u" => {
                    let scalar = param.map(|code| {
                        if code < 0 {
                            (code + 0x1_0000) as u32
                        } else {
                            code as u32
                        }
                    });
                    if let Some(character) = scalar.and_then(char::from_u32) {
                        if !self.skip {
                            let mut buffer = [0_u8; 4];
                            self.push_text(character.encode_utf8(&mut buffer));
                        }
                    }
                    self.pending_u_skip = self.uc;
                }
                "emdash" => self.emit_str("\u{2014}"),
                "endash" => self.emit_str("\u{2013}"),
                "lquote" => self.emit_str("\u{2018}"),
                "rquote" => self.emit_str("\u{2019}"),
                "ldblquote" => self.emit_str("\u{201C}"),
                "rdblquote" => self.emit_str("\u{201D}"),
                "bullet" => self.emit_str("\u{2022}"),
                _ if SKIP_DESTINATIONS.contains(&word) => self.skip = true,
                _ => {}
            }
        }

        fn dispatch_symbol(&mut self, symbol: u8) {
            match symbol {
                b'\'' => {
                    let hex = self.bytes.get(self.pos..self.pos + 2);
                    let byte = hex
                        .and_then(|hex| std::str::from_utf8(hex).ok())
                        .and_then(|hex| u8::from_str_radix(hex, 16).ok());
                    if let Some(byte) = byte {
                        self.pos += 2;
                        let mut buffer = [0_u8; 4];
                        let text = cp1252_to_char(byte).encode_utf8(&mut buffer);
                        let owned = text.to_owned();
                        self.emit_str(&owned);
                    }
                }
                b'~' => self.emit_str("\u{00A0}"),
                b'_' => self.emit_str("\u{2011}"),
                b'\\' => self.emit_str("\\"),
                b'{' => self.emit_str("{"),
                b'}' => self.emit_str("}"),
                b'*' => self.skip = true,
                _ => {}
            }
        }
    }

    /// Parse RTF bytes into styled runs, or `None` if `bytes` does not look
    /// like RTF at all (missing the `{\rtf` header).
    pub fn parse_rtf(bytes: &[u8]) -> Option<Vec<RtfRun>> {
        let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace())?;
        if !bytes[start..].starts_with(b"{\\rtf") {
            return None;
        }
        let colors = parse_color_table(bytes);
        let mut parser = Parser::new(&bytes[start..], colors);
        parser.run();
        Some(parser.runs)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn joined(runs: &[RtfRun]) -> String {
            runs.iter().map(|run| run.text.as_str()).collect()
        }

        #[test]
        fn rejects_data_without_the_rtf_header() {
            assert!(parse_rtf(b"this is not rtf at all").is_none());
            assert!(parse_rtf(b"").is_none());
        }

        #[test]
        fn parses_plain_paragraphs() {
            let rtf = br"{\rtf1\ansi\deff0 Hello world\par Second paragraph.}";
            let runs = parse_rtf(rtf).expect("valid rtf");
            assert_eq!(joined(&runs), "Hello world\nSecond paragraph.");
        }

        #[test]
        fn parses_bold_italic_and_underline_runs() {
            let rtf = br"{\rtf1\ansi Hi \b bold\b0  \i italic\i0  \ul under\ulnone  done}";
            let runs = parse_rtf(rtf).expect("valid rtf");
            assert_eq!(joined(&runs), "Hi bold italic under done");
            assert!(runs.iter().any(|r| r.bold && r.text.contains("bold")));
            assert!(runs.iter().any(|r| r.italic && r.text.contains("italic")));
            assert!(runs.iter().any(|r| r.underline && r.text.contains("under")));
            assert!(runs
                .iter()
                .any(|r| !r.bold && !r.italic && !r.underline && r.text.contains("Hi")));
        }

        #[test]
        fn parses_font_size_and_color_from_the_color_table() {
            let rtf = br"{\rtf1\ansi{\colortbl;\red255\green0\blue0;\red0\green0\blue255;}\fs36\cf1 Big red\cf2  blue}";
            let runs = parse_rtf(rtf).expect("valid rtf");
            let red = runs.iter().find(|r| r.text.contains("Big red")).unwrap();
            assert_eq!(red.color, Some((255, 0, 0)));
            assert_eq!(red.size, Some(18.0));
            let blue = runs.iter().find(|r| r.text.contains("blue")).unwrap();
            assert_eq!(blue.color, Some((0, 0, 255)));
        }

        #[test]
        fn parses_unicode_and_hex_escapes() {
            // The Euro sign as a decimal `\u` escape (8364 = 0x20AC), with
            // its `\uc1` fallback "?" for readers without Unicode support;
            // `\'e9` is Windows-1252 for e-acute. Built with `format!` so
            // this source file never spells the escape and a digit run
            // next to each other, which a JSON-style tool layer could read
            // as its own `\uXXXX` escape.
            let euro_codepoint = 8364;
            let rtf = format!(
                "{{\\rtf1\\ansi \\uc1\\u{euro_codepoint}? caf\\'e9}}"
            );
            let runs = parse_rtf(rtf.as_bytes()).expect("valid rtf");
            assert_eq!(joined(&runs), "\u{20AC} caf\u{e9}");
        }

        #[test]
        fn skips_font_and_color_tables_and_unknown_destinations() {
            let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\colortbl;\red255\green0\blue0;}{\*\generator Riched20}{\info{\author Someone}}Visible text only}";
            let runs = parse_rtf(rtf).expect("valid rtf");
            assert_eq!(joined(&runs), "Visible text only");
        }

        #[test]
        fn field_result_text_survives_while_its_instruction_code_does_not() {
            let rtf = br"{\rtf1\ansi Before {\field{\*\fldinst HYPERLINK ignored }{\fldrslt visible}} after}";
            let runs = parse_rtf(rtf).expect("valid rtf");
            assert_eq!(joined(&runs), "Before visible after");
        }

        #[test]
        fn a_run_of_plain_text_never_panics_on_stray_high_bytes() {
            let mut rtf = b"{\\rtf1\\ansi ".to_vec();
            rtf.extend_from_slice(&[0x93, 0x94, 0x96]); // smart quotes, en dash
            rtf.push(b'}');
            let runs = parse_rtf(&rtf).expect("valid rtf");
            assert_eq!(joined(&runs), "\u{201C}\u{201D}\u{2013}");
        }
    }
}
