//! The attributed-text model behind [`super::RichTextEditor`].
//!
//! A [`Document`] is a list of paragraphs, each a string with styled runs
//! and its own paragraph style (alignment, list, line spacing), the same
//! split NSTextStorage makes between character and paragraph attributes.
//! Offsets are UTF-8 bytes into the document's text with one `\n` between
//! paragraphs. Paragraphs are shared (`Arc`) and copied on write, so an undo
//! snapshot or a save costs one pointer per paragraph, and an edit touches
//! only the paragraphs it changes — the editor relays out just those.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation as _;

/// TextEdit's default rich-text size: Helvetica 12.
pub const DEFAULT_RICH_SIZE: f32 = 12.0;
pub const MIN_FONT_SIZE: f32 = 4.0;
pub const MAX_FONT_SIZE: f32 = 288.0;

/// A colour stored in the document, as RTF's colour table holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn from_u32(value: u32) -> Self {
        Self {
            r: ((value >> 16) & 0xff) as u8,
            g: ((value >> 8) & 0xff) as u8,
            b: (value & 0xff) as u8,
        }
    }

    pub const fn to_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }
}

/// Character attributes of one run.
#[derive(Clone, Debug, PartialEq)]
pub struct CharStyle {
    /// The run's font family as the document names it; `None` is the
    /// document's default face.
    pub family: Option<Arc<str>>,
    /// Point size.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Text colour; `None` is the automatic colour (black on paper).
    pub color: Option<Rgb>,
    /// Highlight (background) colour.
    pub highlight: Option<Rgb>,
}

impl Default for CharStyle {
    fn default() -> Self {
        Self {
            family: None,
            size: DEFAULT_RICH_SIZE,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            color: None,
            highlight: None,
        }
    }
}

impl CharStyle {
    pub fn with_size(size: f32) -> Self {
        Self {
            size: clamp_size(size),
            ..Self::default()
        }
    }
}

pub fn clamp_size(size: f32) -> f32 {
    if size.is_finite() {
        size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
    } else {
        DEFAULT_RICH_SIZE
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// "•" markers.
    Bullet,
    /// "1." "2." … markers.
    Numbered,
}

/// Paragraph attributes: Format ▸ Text and Format ▸ List….
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParagraphStyle {
    pub alignment: Alignment,
    pub list: Option<ListKind>,
    /// Line-height multiple (Format ▸ Text ▸ Spacing…).
    pub line_spacing: f32,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            alignment: Alignment::Left,
            list: None,
            line_spacing: 1.0,
        }
    }
}

/// `len` UTF-8 bytes in `style`.
#[derive(Clone, Debug, PartialEq)]
pub struct StyledRun {
    pub len: usize,
    pub style: CharStyle,
}

static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

fn next_version() -> u64 {
    NEXT_VERSION.fetch_add(1, Ordering::Relaxed)
}

/// One paragraph: its text (without the paragraph separator), styled runs
/// covering it exactly, and its paragraph style.
#[derive(Clone, Debug)]
pub struct Paragraph {
    text: String,
    /// Non-empty; an empty paragraph keeps one zero-length run that holds
    /// the style typing there will use.
    runs: Vec<StyledRun>,
    style: ParagraphStyle,
    utf16_len: usize,
    /// Unique per content: changes on every edit, so layout caches can key
    /// on it.
    version: u64,
}

impl PartialEq for Paragraph {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.runs == other.runs && self.style == other.style
    }
}

impl Paragraph {
    pub fn new(text: String, runs: Vec<StyledRun>, style: ParagraphStyle) -> Self {
        let mut paragraph = Self {
            text,
            runs,
            style,
            utf16_len: 0,
            version: 0,
        };
        paragraph.normalize();
        paragraph
    }

    pub fn plain(text: &str, style: &CharStyle, paragraph_style: ParagraphStyle) -> Self {
        Self::new(
            text.to_owned(),
            vec![StyledRun {
                len: text.len(),
                style: style.clone(),
            }],
            paragraph_style,
        )
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn runs(&self) -> &[StyledRun] {
        &self.runs
    }

    pub fn style(&self) -> ParagraphStyle {
        self.style
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn utf16_len(&self) -> usize {
        self.utf16_len
    }

    /// Runs as `(range, style)`, in order.
    pub fn styled_ranges(&self) -> impl Iterator<Item = (Range<usize>, &CharStyle)> + '_ {
        let mut start = 0;
        self.runs.iter().map(move |run| {
            let range = start..start + run.len;
            start += run.len;
            (range, &run.style)
        })
    }

    /// The style typing at `local` uses: the character before it, or the
    /// first character at the paragraph's start (as NSTextView does).
    pub fn style_at(&self, local: usize) -> &CharStyle {
        let mut start = 0;
        let mut found = &self.runs[0].style;
        for run in &self.runs {
            if local > start {
                found = &run.style;
            } else {
                break;
            }
            start += run.len;
        }
        found
    }

    /// The style of the character starting at `local`.
    pub fn style_of_char_at(&self, local: usize) -> &CharStyle {
        let mut start = 0;
        for run in &self.runs {
            if local < start + run.len {
                return &run.style;
            }
            start += run.len;
        }
        &self.runs[self.runs.len() - 1].style
    }

    fn normalize(&mut self) {
        let fallback = self
            .runs
            .first()
            .map(|run| run.style.clone())
            .unwrap_or_default();
        let total: usize = self.runs.iter().map(|run| run.len).sum();
        if total != self.text.len() {
            // Defensive: never leave runs that do not cover the text.
            let style = fallback.clone();
            self.runs = vec![StyledRun {
                len: self.text.len(),
                style,
            }];
        }
        let mut merged: Vec<StyledRun> = Vec::with_capacity(self.runs.len());
        for run in self.runs.drain(..) {
            if run.len == 0 {
                continue;
            }
            match merged.last_mut() {
                Some(last) if last.style == run.style => last.len += run.len,
                _ => merged.push(run),
            }
        }
        if merged.is_empty() {
            merged.push(StyledRun {
                len: 0,
                style: fallback,
            });
        }
        self.runs = merged;
        self.utf16_len = self.text.chars().map(char::len_utf16).sum();
        self.version = next_version();
    }

    /// The runs covering `range` (local), cut to it.
    fn runs_in(&self, range: Range<usize>) -> Vec<StyledRun> {
        let mut result = Vec::new();
        for (run_range, style) in self.styled_ranges() {
            let start = run_range.start.max(range.start);
            let end = run_range.end.min(range.end);
            if start < end {
                result.push(StyledRun {
                    len: end - start,
                    style: style.clone(),
                });
            }
        }
        if result.is_empty() {
            result.push(StyledRun {
                len: 0,
                style: self.style_at(range.start).clone(),
            });
        }
        result
    }

    /// A copy of `range` (local) as its own paragraph.
    fn slice(&self, range: Range<usize>) -> Paragraph {
        Paragraph::new(
            self.text[range.clone()].to_owned(),
            self.runs_in(range),
            self.style,
        )
    }
}

fn concat(
    left: &Paragraph,
    left_range: Range<usize>,
    right: &Paragraph,
    right_range: Range<usize>,
    style: ParagraphStyle,
) -> Paragraph {
    let mut text = String::with_capacity(left_range.len() + right_range.len());
    text.push_str(&left.text[left_range.clone()]);
    text.push_str(&right.text[right_range.clone()]);
    let mut runs = left.runs_in(left_range);
    runs.extend(right.runs_in(right_range));
    Paragraph::new(text, runs, style)
}

/// An attributed document: paragraphs joined by `\n`.
#[derive(Clone, Debug)]
pub struct Document {
    paragraphs: Vec<Arc<Paragraph>>,
    /// Offset of each paragraph's first byte.
    starts: Vec<usize>,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.paragraphs.len() == other.paragraphs.len()
            && self
                .paragraphs
                .iter()
                .zip(&other.paragraphs)
                .all(|(left, right)| Arc::ptr_eq(left, right) || left == right)
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::empty(&CharStyle::default())
    }
}

impl Document {
    pub fn empty(style: &CharStyle) -> Self {
        Self::from_paragraphs(vec![Paragraph::plain("", style, ParagraphStyle::default())])
    }

    /// Plain text in one style. `\r\n` and `\r` become paragraph breaks.
    pub fn from_plain_text(text: &str, style: &CharStyle) -> Self {
        Self::from_text_with(text, style, ParagraphStyle::default())
    }

    fn from_text_with(text: &str, style: &CharStyle, paragraph_style: ParagraphStyle) -> Self {
        let normalized = normalize_newlines(text);
        Self::from_paragraphs(
            normalized
                .split('\n')
                .map(|line| Paragraph::plain(line, style, paragraph_style))
                .collect(),
        )
    }

    pub fn from_paragraphs(paragraphs: Vec<Paragraph>) -> Self {
        let mut document = Self {
            paragraphs: paragraphs.into_iter().map(Arc::new).collect(),
            starts: Vec::new(),
        };
        if document.paragraphs.is_empty() {
            document.paragraphs.push(Arc::new(Paragraph::plain(
                "",
                &CharStyle::default(),
                ParagraphStyle::default(),
            )));
        }
        document.recompute_starts();
        document
    }

    fn recompute_starts(&mut self) {
        self.starts.clear();
        let mut offset = 0;
        for paragraph in &self.paragraphs {
            self.starts.push(offset);
            offset += paragraph.len() + 1;
        }
    }

    pub fn paragraphs(&self) -> &[Arc<Paragraph>] {
        &self.paragraphs
    }

    pub fn paragraph(&self, index: usize) -> &Paragraph {
        &self.paragraphs[index]
    }

    pub fn paragraph_count(&self) -> usize {
        self.paragraphs.len()
    }

    pub fn paragraph_start(&self, index: usize) -> usize {
        self.starts[index]
    }

    pub fn paragraph_range(&self, index: usize) -> Range<usize> {
        let start = self.starts[index];
        start..start + self.paragraphs[index].len()
    }

    /// Total length in bytes, counting one `\n` between paragraphs.
    pub fn len(&self) -> usize {
        let last = self.paragraphs.len() - 1;
        self.starts[last] + self.paragraphs[last].len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn utf16_len(&self) -> usize {
        self.paragraphs.iter().map(|p| p.utf16_len()).sum::<usize>() + self.paragraphs.len() - 1
    }

    pub fn text(&self) -> String {
        let mut text = String::with_capacity(self.len());
        for (index, paragraph) in self.paragraphs.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            text.push_str(paragraph.text());
        }
        text
    }

    /// The paragraph holding `offset` and the offset inside it. An offset on
    /// a paragraph separator is the end of the paragraph before it.
    pub fn locate(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.len());
        let index = self.starts.partition_point(|start| *start <= offset) - 1;
        let local = (offset - self.starts[index]).min(self.paragraphs[index].len());
        (index, local)
    }

    /// `offset` clamped to the document and moved back to a char boundary.
    pub fn clamp_offset(&self, offset: usize) -> usize {
        let (index, mut local) = self.locate(offset);
        let text = self.paragraphs[index].text();
        while local > 0 && !text.is_char_boundary(local) {
            local -= 1;
        }
        self.starts[index] + local
    }

    pub fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_offset(range.start.min(range.end));
        let end = self.clamp_offset(range.end.max(range.start));
        start..end
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let range = self.clamp_range(range);
        let (first, first_local) = self.locate(range.start);
        let (last, last_local) = self.locate(range.end);
        if first == last {
            return self.paragraphs[first].text()[first_local..last_local].to_owned();
        }
        let mut text = String::with_capacity(range.len());
        text.push_str(&self.paragraphs[first].text()[first_local..]);
        for paragraph in &self.paragraphs[first + 1..last] {
            text.push('\n');
            text.push_str(paragraph.text());
        }
        text.push('\n');
        text.push_str(&self.paragraphs[last].text()[..last_local]);
        text
    }

    /// A styled copy of `range`.
    pub fn fragment(&self, range: Range<usize>) -> Document {
        let range = self.clamp_range(range);
        let (first, first_local) = self.locate(range.start);
        let (last, last_local) = self.locate(range.end);
        if first == last {
            return Document::from_paragraphs(vec![
                self.paragraphs[first].slice(first_local..last_local)
            ]);
        }
        let mut paragraphs = Vec::with_capacity(last - first + 1);
        let head = &self.paragraphs[first];
        paragraphs.push(head.slice(first_local..head.len()));
        for paragraph in &self.paragraphs[first + 1..last] {
            paragraphs.push((**paragraph).clone());
        }
        paragraphs.push(self.paragraphs[last].slice(0..last_local));
        Document::from_paragraphs(paragraphs)
    }

    /// The style typing at `offset` would use.
    pub fn style_at(&self, offset: usize) -> CharStyle {
        let (index, local) = self.locate(offset);
        self.paragraphs[index].style_at(local).clone()
    }

    /// The style of the character starting at `offset` (the first character
    /// of a selection), or the typing style at a paragraph's end.
    pub fn style_of_char_at(&self, offset: usize) -> CharStyle {
        let (index, local) = self.locate(offset);
        let paragraph = &self.paragraphs[index];
        if local < paragraph.len() {
            paragraph.style_of_char_at(local).clone()
        } else {
            paragraph.style_at(local).clone()
        }
    }

    pub fn paragraph_style_at(&self, offset: usize) -> ParagraphStyle {
        self.paragraphs[self.locate(offset).0].style()
    }

    /// Indices of the paragraphs `range` touches.
    pub fn paragraph_indices(&self, range: Range<usize>) -> Range<usize> {
        let range = self.clamp_range(range);
        let first = self.locate(range.start).0;
        let last = self.locate(range.end).0;
        first..last + 1
    }

    /// Replace `range` with plain `text` in `style`. Paragraphs the text
    /// creates take the paragraph style of the one it was typed into, as
    /// Return does in TextEdit. Returns the inserted range.
    pub fn replace_text(
        &mut self,
        range: Range<usize>,
        text: &str,
        style: &CharStyle,
    ) -> Range<usize> {
        let range = self.clamp_range(range);
        let paragraph_style = self.paragraph_style_at(range.start);
        let fragment = Document::from_text_with(text, style, paragraph_style);
        self.splice(range, &fragment, false)
    }

    /// Replace `range` with a styled `fragment` (paste, undo of a cut).
    /// With `keep_paragraph_styles`, whole paragraphs inside the fragment
    /// keep their own paragraph styles. Returns the inserted range.
    pub fn replace_fragment(
        &mut self,
        range: Range<usize>,
        fragment: &Document,
        keep_paragraph_styles: bool,
    ) -> Range<usize> {
        let range = self.clamp_range(range);
        self.splice(range, fragment, keep_paragraph_styles)
    }

    fn splice(
        &mut self,
        range: Range<usize>,
        fragment: &Document,
        keep_paragraph_styles: bool,
    ) -> Range<usize> {
        let (first, first_local) = self.locate(range.start);
        let (last, last_local) = self.locate(range.end);
        let head = self.paragraphs[first].clone();
        let tail = self.paragraphs[last].clone();
        let head_style = head.style();
        let pieces = &fragment.paragraphs;
        let mut replacement: Vec<Arc<Paragraph>> = Vec::with_capacity(pieces.len());
        if pieces.len() == 1 {
            let middle = &pieces[0];
            // head + fragment + tail, built in two steps so runs stay exact.
            let joined = concat(&head, 0..first_local, middle, 0..middle.len(), head_style);
            let joined_len = joined.len();
            let full = concat(
                &joined,
                0..joined_len,
                &tail,
                last_local..tail.len(),
                head_style,
            );
            replacement.push(Arc::new(full));
        } else {
            let opening = &pieces[0];
            replacement.push(Arc::new(concat(
                &head,
                0..first_local,
                opening,
                0..opening.len(),
                head_style,
            )));
            for piece in &pieces[1..pieces.len() - 1] {
                let mut piece = (**piece).clone();
                if !keep_paragraph_styles {
                    piece.style = head_style;
                }
                piece.version = next_version();
                replacement.push(Arc::new(piece));
            }
            let closing = &pieces[pieces.len() - 1];
            let closing_style = if keep_paragraph_styles {
                closing.style()
            } else {
                head_style
            };
            replacement.push(Arc::new(concat(
                closing,
                0..closing.len(),
                &tail,
                last_local..tail.len(),
                closing_style,
            )));
        }
        self.paragraphs.splice(first..=last, replacement);
        self.recompute_starts();
        range.start..range.start + fragment.len()
    }

    /// Apply `change` to every character in `range`. An empty paragraph the
    /// range covers takes the change too, so typing there follows it.
    pub fn update_char_style(
        &mut self,
        range: Range<usize>,
        mut change: impl FnMut(&mut CharStyle),
    ) {
        let range = self.clamp_range(range);
        if range.is_empty() {
            return;
        }
        let (first, first_local) = self.locate(range.start);
        let (last, last_local) = self.locate(range.end);
        for index in first..=last {
            let paragraph = &self.paragraphs[index];
            let start = if index == first { first_local } else { 0 };
            let end = if index == last {
                last_local
            } else {
                paragraph.len()
            };
            if start == end && !paragraph.is_empty() {
                continue;
            }
            let mut runs = Vec::with_capacity(paragraph.runs.len() + 2);
            for (run_range, style) in paragraph.styled_ranges() {
                let cut_start = run_range.start.max(start);
                let cut_end = run_range.end.min(end);
                if paragraph.is_empty() || cut_start >= cut_end {
                    let mut style = style.clone();
                    if paragraph.is_empty() {
                        change(&mut style);
                    }
                    runs.push(StyledRun {
                        len: run_range.len(),
                        style,
                    });
                    continue;
                }
                if run_range.start < cut_start {
                    runs.push(StyledRun {
                        len: cut_start - run_range.start,
                        style: style.clone(),
                    });
                }
                let mut changed = style.clone();
                change(&mut changed);
                runs.push(StyledRun {
                    len: cut_end - cut_start,
                    style: changed,
                });
                if cut_end < run_range.end {
                    runs.push(StyledRun {
                        len: run_range.end - cut_end,
                        style: style.clone(),
                    });
                }
            }
            let rebuilt = Paragraph::new(paragraph.text.clone(), runs, paragraph.style);
            self.paragraphs[index] = Arc::new(rebuilt);
        }
    }

    /// Apply `change` to the paragraph style of every paragraph `range`
    /// touches (the caret's paragraph for an empty range).
    pub fn update_paragraph_style(
        &mut self,
        range: Range<usize>,
        mut change: impl FnMut(&mut ParagraphStyle),
    ) {
        for index in self.paragraph_indices(range) {
            let paragraph = Arc::make_mut(&mut self.paragraphs[index]);
            let before = paragraph.style;
            change(&mut paragraph.style);
            if paragraph.style != before {
                paragraph.version = next_version();
            }
        }
    }

    /// The number shown before each paragraph of a numbered list (0 for
    /// every other paragraph). Consecutive numbered paragraphs count up.
    pub fn list_numbers(&self) -> Vec<u32> {
        let mut numbers = Vec::with_capacity(self.paragraphs.len());
        let mut counter = 0;
        for paragraph in &self.paragraphs {
            if paragraph.style().list == Some(ListKind::Numbered) {
                counter += 1;
                numbers.push(counter);
            } else {
                counter = 0;
                numbers.push(0);
            }
        }
        numbers
    }

    /// The previous grapheme boundary (a paragraph break is one step).
    pub fn previous_boundary(&self, offset: usize) -> usize {
        let (index, local) = self.locate(offset);
        let start = self.starts[index];
        if local == 0 {
            return start.saturating_sub(1);
        }
        let text = self.paragraphs[index].text();
        let previous = text[..local]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(at, _)| at);
        start + previous
    }

    /// The next grapheme boundary (a paragraph break is one step).
    pub fn next_boundary(&self, offset: usize) -> usize {
        let (index, local) = self.locate(offset);
        let start = self.starts[index];
        let text = self.paragraphs[index].text();
        if local >= text.len() {
            return if index + 1 < self.paragraphs.len() {
                self.starts[index + 1]
            } else {
                self.len()
            };
        }
        let next = text[local..]
            .graphemes(true)
            .next()
            .map_or(text.len(), |grapheme| local + grapheme.len());
        start + next
    }

    /// Start of the word before `offset` (⌥←).
    pub fn previous_word_start(&self, offset: usize) -> usize {
        let (mut index, mut local) = self.locate(offset);
        loop {
            let text = self.paragraphs[index].text();
            let found = text
                .split_word_bound_indices()
                .filter(|(at, word)| *at < local && is_word(word))
                .map(|(at, _)| at)
                .next_back();
            if let Some(at) = found {
                return self.starts[index] + at;
            }
            if index == 0 {
                return 0;
            }
            index -= 1;
            local = self.paragraphs[index].len();
        }
    }

    /// End of the word after `offset` (⌥→).
    pub fn next_word_end(&self, offset: usize) -> usize {
        let (mut index, mut local) = self.locate(offset);
        loop {
            let text = self.paragraphs[index].text();
            let found = text
                .split_word_bound_indices()
                .find(|(at, word)| at + word.len() > local && is_word(word))
                .map(|(at, word)| at + word.len());
            if let Some(end) = found {
                return self.starts[index] + end;
            }
            if index + 1 >= self.paragraphs.len() {
                return self.len();
            }
            index += 1;
            local = 0;
        }
    }

    /// The word (or run of spaces/punctuation) around `offset`, for a
    /// double-click.
    pub fn word_range_at(&self, offset: usize) -> Range<usize> {
        let (index, local) = self.locate(offset);
        let start = self.starts[index];
        let text = self.paragraphs[index].text();
        if text.is_empty() {
            return start..start;
        }
        let mut chosen = None;
        for (at, word) in text.split_word_bound_indices() {
            let end = at + word.len();
            if local < end || end == text.len() {
                chosen = Some(at..end);
                if local < end {
                    break;
                }
            }
        }
        let range = chosen.unwrap_or(0..text.len());
        start + range.start..start + range.end
    }

    /// UTF-8 offset → UTF-16 offset (for the platform input method).
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        let (index, local) = self.locate(self.clamp_offset(offset));
        let before: usize = self.paragraphs[..index]
            .iter()
            .map(|p| p.utf16_len() + 1)
            .sum();
        let text = self.paragraphs[index].text();
        before + text[..local].chars().map(char::len_utf16).sum::<usize>()
    }

    /// UTF-16 offset → UTF-8 offset; one inside a surrogate pair rounds down.
    pub fn offset_from_utf16(&self, target: usize) -> usize {
        let mut remaining = target;
        for (index, paragraph) in self.paragraphs.iter().enumerate() {
            if remaining <= paragraph.utf16_len() {
                let mut local = 0;
                let mut units = 0;
                for character in paragraph.text().chars() {
                    if units + character.len_utf16() > remaining {
                        break;
                    }
                    units += character.len_utf16();
                    local += character.len_utf8();
                }
                return self.starts[index] + local;
            }
            remaining -= paragraph.utf16_len() + 1;
        }
        self.len()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

/// `\r\n` and lone `\r` become `\n`.
pub fn normalize_newlines(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> CharStyle {
        CharStyle {
            bold: true,
            ..CharStyle::default()
        }
    }

    fn runs_of(document: &Document, index: usize) -> Vec<(String, bool)> {
        let paragraph = document.paragraph(index);
        paragraph
            .styled_ranges()
            .map(|(range, style)| (paragraph.text()[range].to_owned(), style.bold))
            .collect()
    }

    #[test]
    fn plain_text_splits_into_paragraphs_with_offsets_counting_breaks() {
        let document = Document::from_plain_text("one\r\ntwo\rthree", &CharStyle::default());
        assert_eq!(document.paragraph_count(), 3);
        assert_eq!(document.text(), "one\ntwo\nthree");
        assert_eq!(document.len(), 13);
        assert_eq!(document.locate(3), (0, 3));
        assert_eq!(document.locate(4), (1, 0));
        assert_eq!(document.paragraph_range(2), 8..13);
        assert_eq!(document.slice(2..9), "e\ntwo\nt");
    }

    #[test]
    fn typing_inserts_with_the_given_style_and_return_keeps_paragraph_style() {
        let mut document = Document::from_plain_text("ab", &CharStyle::default());
        document.update_paragraph_style(0..0, |style| style.alignment = Alignment::Center);
        let inserted = document.replace_text(1..1, "X\nY", &bold());
        assert_eq!(inserted, 1..4);
        assert_eq!(document.text(), "aX\nYb");
        assert_eq!(
            runs_of(&document, 0),
            [("a".into(), false), ("X".into(), true)]
        );
        assert_eq!(
            runs_of(&document, 1),
            [("Y".into(), true), ("b".into(), false)]
        );
        assert_eq!(document.paragraph(1).style().alignment, Alignment::Center);
    }

    #[test]
    fn deleting_across_paragraphs_joins_them_and_keeps_the_first_style() {
        let mut document = Document::from_plain_text("one\ntwo\nthree", &CharStyle::default());
        document.update_paragraph_style(9..9, |style| style.alignment = Alignment::Right);
        document.replace_text(2..9, "", &CharStyle::default());
        assert_eq!(document.text(), "onhree");
        assert_eq!(document.paragraph_count(), 1);
        assert_eq!(document.paragraph(0).style().alignment, Alignment::Left);
    }

    #[test]
    fn styling_a_range_splits_and_merges_runs() {
        let mut document = Document::from_plain_text("hello world", &CharStyle::default());
        document.update_char_style(6..11, |style| style.bold = true);
        assert_eq!(
            runs_of(&document, 0),
            [("hello ".into(), false), ("world".into(), true)]
        );
        document.update_char_style(0..6, |style| style.bold = true);
        assert_eq!(runs_of(&document, 0), [("hello world".into(), true)]);
        assert!(document.style_at(11).bold);
        assert!(document.style_of_char_at(0).bold);
    }

    #[test]
    fn styling_reaches_an_empty_paragraph_inside_the_range() {
        let mut document = Document::from_plain_text("a\n\nb", &CharStyle::default());
        document.update_char_style(0..4, |style| style.italic = true);
        assert!(document.paragraph(1).style_at(0).italic);
        document.replace_text(2..2, "z", &document.style_at(2));
        assert!(document.style_of_char_at(2).italic);
    }

    #[test]
    fn fragments_round_trip_styled_text() {
        let mut document = Document::from_plain_text("one\ntwo", &CharStyle::default());
        document.update_char_style(1..5, |style| style.underline = true);
        let fragment = document.fragment(1..5);
        assert_eq!(fragment.text(), "ne\nt");
        let mut target = Document::from_plain_text("[]", &CharStyle::default());
        target.replace_fragment(1..1, &fragment, true);
        assert_eq!(target.text(), "[ne\nt]");
        assert!(target.style_of_char_at(1).underline);
        assert!(target.style_of_char_at(4).underline);
        assert!(!target.style_of_char_at(5).underline);
    }

    #[test]
    fn snapshots_compare_equal_until_an_edit() {
        let mut document = Document::from_plain_text("one\ntwo", &CharStyle::default());
        let saved = document.clone();
        assert_eq!(document, saved);
        document.update_char_style(0..1, |style| style.bold = true);
        assert_ne!(document, saved);
        document.update_char_style(0..1, |style| style.bold = false);
        assert_eq!(document, saved);
    }

    #[test]
    fn edits_change_only_the_touched_paragraph_versions() {
        let mut document = Document::from_plain_text("one\ntwo\nthree", &CharStyle::default());
        let before: Vec<u64> = document.paragraphs().iter().map(|p| p.version()).collect();
        document.replace_text(5..5, "x", &CharStyle::default());
        let after: Vec<u64> = document.paragraphs().iter().map(|p| p.version()).collect();
        assert_eq!(before[0], after[0]);
        assert_ne!(before[1], after[1]);
        assert_eq!(before[2], after[2]);
    }

    #[test]
    fn utf16_offsets_count_surrogates_and_breaks() {
        let document = Document::from_plain_text("a😀\nb", &CharStyle::default());
        assert_eq!(document.utf16_len(), 5);
        assert_eq!(document.offset_to_utf16(5), 3);
        assert_eq!(document.offset_to_utf16(6), 4);
        assert_eq!(document.offset_from_utf16(3), 5);
        assert_eq!(document.offset_from_utf16(4), 6);
        assert_eq!(document.offset_from_utf16(2), 1);
    }

    #[test]
    fn boundaries_and_words_cross_paragraphs() {
        let document = Document::from_plain_text("hi there\nyou", &CharStyle::default());
        assert_eq!(document.previous_boundary(9), 8);
        assert_eq!(document.next_boundary(8), 9);
        assert_eq!(document.previous_word_start(9), 3);
        assert_eq!(document.next_word_end(8), 12);
        assert_eq!(document.next_word_end(0), 2);
        assert_eq!(document.word_range_at(4), 3..8);
    }

    #[test]
    fn numbered_lists_count_consecutive_paragraphs() {
        let mut document = Document::from_plain_text("a\nb\nc\nd", &CharStyle::default());
        document.update_paragraph_style(0..3, |style| style.list = Some(ListKind::Numbered));
        document.update_paragraph_style(6..6, |style| style.list = Some(ListKind::Numbered));
        assert_eq!(document.list_numbers(), [1, 2, 0, 1]);
    }
}
