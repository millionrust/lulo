//! Paragraph layout for the rich-text editor.
//!
//! GPUI's `TextRun` carries no font size, so a paragraph that mixes sizes
//! cannot be shaped as one wrapped line. Each paragraph is cut into atoms of
//! one size, baseline shift, outline and kerning (tabs are their own
//! atoms), each atom is shaped once unwrapped to learn where every glyph
//! falls, and lines are broken over those positions. Each line is then
//! painted as one shaped piece per atom it holds, all on a common baseline
//! (raised or lowered pieces sit above or below it). Caret and hit-testing
//! math uses the same glyph positions, so what is painted and what is
//! clicked agree.
//!
//! Kern ▸ Tighten / Loosen adds space after every character: such an atom
//! is painted one character per piece, each at its spaced position. Kern ▸
//! Use None, Ligatures and Character Shape are OpenType features of the run's
//! font. Outline is the glyphs drawn in the text colour around a fill of the
//! paper colour.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    font, px, Font, FontFeatures, Hsla, Pixels, ShapedLine, SharedString, StrikethroughStyle,
    TextRun, UnderlineStyle, Window,
};

use super::model::{Alignment, CharStyle, Ligatures, Paragraph, Rgb};

/// TextEdit's default tab stops: every 28 points.
const TAB_INTERVAL: f32 = 28.0;
/// A list item's text starts 36 points in per level, its marker 11 points
/// in from its level's start.
const LIST_INDENT: f32 = 36.0;
const LIST_MARKER_X: f32 = 11.0;
/// Allow Hyphenation: the shortest word split, and the shortest piece on
/// either side of the hyphen.
const HYPHEN_MIN_WORD: usize = 6;
const HYPHEN_MIN_HEAD: usize = 2;
const HYPHEN_MIN_TAIL: usize = 3;

/// What a paragraph's layout depends on besides its own content.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutParams {
    /// Width of the text column.
    pub width: Pixels,
    /// View scale (View ▸ Zoom).
    pub zoom: f32,
    /// Colour of text with no colour of its own.
    pub default_color: Hsla,
    /// The paper behind the text (an outline's fill).
    pub paper_color: Hsla,
    /// Linked text's colour.
    pub link_color: Hsla,
    /// Face for text with no family of its own (TextEdit's Helvetica).
    pub default_family: SharedString,
    /// Face standing in for the Mac's monospaced families.
    pub mono_family: SharedString,
    /// Format ▸ Allow Hyphenation.
    pub hyphenation: bool,
}

pub(crate) fn hsla(color: Rgb) -> Hsla {
    gpui::rgb(color.to_u32()).into()
}

fn family_for(style: &CharStyle, params: &LayoutParams) -> SharedString {
    let Some(family) = style.family.as_deref() else {
        return params.default_family.clone();
    };
    let lower = family.to_ascii_lowercase();
    let is_mono = ["menlo", "monaco", "courier", "sf mono", "andale mono"]
        .iter()
        .any(|mono| lower.starts_with(mono));
    let is_default = [
        "helvetica",
        ".applesystemuifont",
        ".sf ns",
        "sf pro",
        "lucida grande",
        "arial",
    ]
    .iter()
    .any(|face| lower.starts_with(face));
    if is_mono {
        params.mono_family.clone()
    } else if is_default {
        params.default_family.clone()
    } else {
        // An installed face is used as named; a missing one falls back
        // through GPUI's font stack.
        SharedString::from(family.split('-').next().unwrap_or(family).to_owned())
    }
}

/// The OpenType features Kern ▸ Use None, Ligatures and Character Shape
/// ask for; empty for ordinary text.
fn features_for(style: &CharStyle) -> Vec<(String, u32)> {
    let mut features = Vec::new();
    if style.kern == Some(0.0) {
        features.push(("kern".to_owned(), 0));
    }
    match style.ligatures {
        Ligatures::Default => {}
        Ligatures::None => {
            features.push(("liga".to_owned(), 0));
            features.push(("clig".to_owned(), 0));
        }
        Ligatures::All => features.push(("dlig".to_owned(), 1)),
    }
    if style.traditional {
        features.push(("trad".to_owned(), 1));
    }
    features
}

pub(crate) fn font_for(style: &CharStyle, params: &LayoutParams) -> Font {
    let mut face = font(family_for(style, params));
    if style.bold {
        face = face.bold();
    }
    if style.italic {
        face = face.italic();
    }
    let features = features_for(style);
    if !features.is_empty() {
        face.features = FontFeatures(Arc::new(features));
    }
    face
}

pub(crate) fn text_run(style: &CharStyle, len: usize, params: &LayoutParams) -> TextRun {
    let thickness = px((params.zoom * style.size / 12.0).clamp(1.0, 4.0));
    let color = if style.link.is_some() && style.color.is_none() {
        params.link_color
    } else {
        style.color.map(hsla).unwrap_or(params.default_color)
    };
    TextRun {
        len,
        font: font_for(style, params),
        color,
        background_color: style.highlight.map(hsla),
        underline: (style.underline || style.link.is_some()).then_some(UnderlineStyle {
            thickness,
            color: None,
            wavy: false,
        }),
        strikethrough: style.strikethrough.then_some(StrikethroughStyle {
            thickness,
            color: None,
        }),
    }
}

/// The run an outline is filled with: the same glyphs in the paper colour.
fn fill_run(style: &CharStyle, len: usize, params: &LayoutParams) -> TextRun {
    TextRun {
        len,
        font: font_for(style, params),
        color: params.paper_color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn size_px(style: &CharStyle, params: &LayoutParams) -> Pixels {
    px(style.size * params.zoom)
}

/// Whether two styles can share one shaped atom.
fn same_atom(left: &CharStyle, right: &CharStyle) -> bool {
    left.size == right.size
        && left.baseline_shift() == right.baseline_shift()
        && left.outline == right.outline
        && left.kern == right.kern
}

/// Kern ▸ Tighten / Loosen's extra space after each character.
fn tracking(style: &CharStyle, params: &LayoutParams) -> Pixels {
    match style.kern {
        Some(kern) if kern != 0.0 => px(kern * params.zoom),
        _ => px(0.0),
    }
}

/// One shaped stretch of a line, in one size.
pub(crate) struct Piece {
    pub x: Pixels,
    /// How far the piece sits above the line's baseline (negative: below).
    pub rise: Pixels,
    pub line: ShapedLine,
    /// Outline: the same glyphs in the paper colour, painted over the text
    /// colour drawn around them.
    pub fill: Option<ShapedLine>,
}

pub(crate) struct LineBox {
    /// First byte of the line.
    pub start: usize,
    /// End of the line's visible text (before a forced break's separator).
    pub content_end: usize,
    /// Where the next line starts.
    pub end: usize,
    /// The line ends at the paragraph's end or at a forced break, so a
    /// caret may sit at `content_end`.
    pub hard_end: bool,
    pub top: Pixels,
    pub height: Pixels,
    /// Baseline, from `top`.
    pub ascent: Pixels,
    /// x of the line's first character, after indent and alignment.
    pub left: Pixels,
    /// Glyph x of `start` in the paragraph's unwrapped coordinates.
    pub base_x: Pixels,
    /// Justification: extra space added after each of these word starts.
    pub gaps: Vec<usize>,
    pub gap_extra: Pixels,
    pub pieces: Vec<Piece>,
}

pub(crate) struct Marker {
    pub x: Pixels,
    pub line: ShapedLine,
}

pub(crate) struct ParagraphLayout {
    pub text: SharedString,
    pub lines: Vec<LineBox>,
    pub height: Pixels,
    /// The right edge selection highlights reach.
    pub right: Pixels,
    pub marker: Option<Marker>,
    /// `(byte, x)` for every glyph start, sorted, ending at the text's end.
    stops: Vec<(usize, Pixels)>,
}

enum Atom {
    Text {
        range: Range<usize>,
        style_ranges: Vec<(Range<usize>, CharStyle)>,
    },
    Tab {
        at: usize,
    },
    /// A line or page break: never shaped (no font has a glyph for it, and
    /// asking would send GPUI through its whole fallback list).
    Break {
        at: usize,
        len: usize,
    },
}

fn atoms(paragraph: &Paragraph) -> Vec<Atom> {
    let text = paragraph.text();
    let mut atoms: Vec<Atom> = Vec::new();
    for (range, style) in paragraph.styled_ranges() {
        let mut cursor = range.start;
        let push_text = |atoms: &mut Vec<Atom>, sub: Range<usize>| {
            if sub.is_empty() {
                return;
            }
            if let Some(Atom::Text {
                range: last,
                style_ranges,
            }) = atoms.last_mut()
            {
                let joins = style_ranges
                    .last()
                    .is_some_and(|(_, last_style)| same_atom(last_style, style));
                if joins && last.end == sub.start {
                    last.end = sub.end;
                    style_ranges.push((sub, style.clone()));
                    return;
                }
            }
            atoms.push(Atom::Text {
                range: sub.clone(),
                style_ranges: vec![(sub, style.clone())],
            });
        };
        for (offset, character) in text[range.clone()]
            .char_indices()
            .filter(|(_, character)| *character == '\t' || is_forced_break(*character))
        {
            let at = range.start + offset;
            push_text(&mut atoms, cursor..at);
            if character == '\t' {
                atoms.push(Atom::Tab { at });
            } else {
                atoms.push(Atom::Break {
                    at,
                    len: character.len_utf8(),
                });
            }
            cursor = at + character.len_utf8();
        }
        push_text(&mut atoms, cursor..range.end);
    }
    atoms
}

fn runs_for(
    paragraph: &Paragraph,
    range: &Range<usize>,
    params: &LayoutParams,
    run: fn(&CharStyle, usize, &LayoutParams) -> TextRun,
) -> Vec<TextRun> {
    paragraph
        .styled_ranges()
        .filter_map(|(run_range, style)| {
            let start = run_range.start.max(range.start);
            let end = run_range.end.min(range.end);
            (start < end).then(|| run(style, end - start, params))
        })
        .collect()
}

/// Ascent and descent of `style`'s face, for a line with no glyphs.
fn metrics(style: &CharStyle, params: &LayoutParams, window: &Window) -> (Pixels, Pixels) {
    let layout = window.text_system().layout_line(
        " ",
        size_px(style, params),
        &[text_run(style, 1, params)],
        None,
    );
    (layout.ascent, layout.descent)
}

fn is_forced_break(character: char) -> bool {
    matches!(character, '\u{2028}' | '\u{000C}')
}

fn is_vowel(character: char) -> bool {
    matches!(
        character.to_ascii_lowercase(),
        'a' | 'e' | 'i' | 'o' | 'u' | 'y'
    )
}

/// Pairs of consonants a hyphen never separates.
const DIGRAPHS: &[&str] = &[
    "ch", "sh", "th", "ph", "wh", "ck", "ng", "qu", "gh", "kn", "wr",
];

/// Where Allow Hyphenation may split `word` (byte offsets inside it): the
/// syllable rule (a consonant between two vowels starts the next syllable;
/// two consonants between vowels split between them), never inside a
/// digraph and never leaving fewer than two letters before the hyphen or
/// three after it. Words with anything but letters are never split.
pub(crate) fn hyphenation_points(word: &str) -> Vec<usize> {
    let letters: Vec<(usize, char)> = word.char_indices().collect();
    if letters.len() < HYPHEN_MIN_WORD || !letters.iter().all(|(_, c)| c.is_alphabetic()) {
        return Vec::new();
    }
    let lower: Vec<char> = letters
        .iter()
        .map(|(_, c)| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut points = Vec::new();
    for split in HYPHEN_MIN_HEAD..=letters.len() - HYPHEN_MIN_TAIL {
        let before = lower[split - 1];
        let after = lower[split];
        let pair: String = [before, after].iter().collect();
        if DIGRAPHS.contains(&pair.as_str()) {
            continue;
        }
        let next = lower.get(split + 1).copied();
        let previous = split.checked_sub(2).map(|index| lower[index]);
        // V|CV: a single consonant between vowels goes with the next one.
        let vowel_consonant_vowel =
            is_vowel(before) && !is_vowel(after) && next.is_some_and(is_vowel);
        // VC|CV: two consonants between vowels split between them.
        let consonant_pair = !is_vowel(before)
            && !is_vowel(after)
            && previous.is_some_and(is_vowel)
            && next.is_some_and(is_vowel);
        if vowel_consonant_vowel || consonant_pair {
            points.push(letters[split].0);
        }
    }
    points
}

pub(crate) fn layout_paragraph(
    paragraph: &Paragraph,
    list_number: u32,
    params: &LayoutParams,
    window: &Window,
) -> ParagraphLayout {
    let text = paragraph.text();
    let style = paragraph.style();
    let zoom = params.zoom;
    let level = f32::from(style.list_level);
    let indent = if style.list.is_some() {
        px(LIST_INDENT * (level + 1.0) * zoom)
    } else {
        px(0.0)
    };
    let available = (params.width - indent).max(px(1.0));
    let tab = px(TAB_INTERVAL * zoom);
    let text_system = window.text_system();

    // Glyph positions along one unwrapped line.
    let atoms = atoms(paragraph);
    let mut stops: Vec<(usize, Pixels)> = Vec::with_capacity(text.len() + 1);
    let mut x = px(0.0);
    for atom in &atoms {
        match atom {
            Atom::Text {
                range,
                style_ranges,
            } => {
                let first = &style_ranges[0].1;
                let size = size_px(first, params);
                let extra = tracking(first, params);
                let runs: Vec<TextRun> = style_ranges
                    .iter()
                    .map(|(sub, style)| text_run(style, sub.len(), params))
                    .collect();
                let shaped = text_system.layout_line(&text[range.clone()], size, &runs, None);
                let mut glyphs = 0_usize;
                for run in &shaped.runs {
                    for glyph in &run.glyphs {
                        stops.push((
                            range.start + glyph.index,
                            x + glyph.position.x + extra * glyphs as f32,
                        ));
                        glyphs += 1;
                    }
                }
                x += shaped.width + extra * glyphs as f32;
            }
            Atom::Tab { at } => {
                stops.push((*at, x));
                let steps = (f32::from(x) / f32::from(tab)).floor() + 1.0;
                x = tab * steps;
            }
            Atom::Break { at, len } => {
                stops.push((*at, x));
                stops.push((*at + *len, x));
            }
        }
    }
    stops.sort_by_key(|(byte, _)| *byte);
    stops.push((text.len(), x));

    let x_at = |byte: usize| -> Pixels {
        let index = stops.partition_point(|(at, _)| *at < byte);
        stops.get(index).map_or(x, |(_, x)| *x)
    };

    // The width a hyphen takes in the style of the character before `at`.
    let hyphen_width = |at: usize| -> Pixels {
        let style = paragraph.style_at(at);
        text_system
            .layout_line(
                "-",
                size_px(style, params),
                &[text_run(style, 1, params)],
                None,
            )
            .width
    };

    // Break lines: at forced breaks, after the last space that fits (or,
    // with hyphenation, inside the word that does not), or inside a word
    // longer than the line.
    struct Break {
        start: usize,
        content_end: usize,
        end: usize,
        hard: bool,
        hyphen: bool,
    }
    let mut breaks: Vec<Break> = Vec::new();
    let mut start = 0;
    let mut start_x = x_at(0);
    let mut last_space: Option<usize> = None;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        if is_forced_break(character) {
            breaks.push(Break {
                start,
                content_end: index,
                end: next,
                hard: true,
                hyphen: false,
            });
            start = next;
            start_x = x_at(next);
            last_space = None;
            continue;
        }
        if character.is_whitespace() {
            last_space = Some(next);
            continue;
        }
        if x_at(next) - start_x > available && index > start {
            let word_start = match last_space {
                Some(space) if space > start => space,
                _ => start,
            };
            let mut hyphen_at = None;
            if params.hyphenation {
                let word_end = text[word_start..]
                    .find(|c: char| c.is_whitespace() || is_forced_break(c))
                    .map_or(text.len(), |end| word_start + end);
                let word = &text[word_start..word_end];
                hyphen_at = hyphenation_points(word)
                    .into_iter()
                    .rev()
                    .map(|point| word_start + point)
                    .find(|at| {
                        *at <= index && x_at(*at) - start_x + hyphen_width(*at) <= available
                    });
            }
            let (at, hyphen) = match (hyphen_at, last_space) {
                (Some(at), _) => (at, true),
                (None, Some(space)) if space > start => (space, false),
                _ => (index, false),
            };
            breaks.push(Break {
                start,
                content_end: at,
                end: at,
                hard: false,
                hyphen,
            });
            start = at;
            start_x = x_at(at);
            last_space = None;
        }
    }
    breaks.push(Break {
        start,
        content_end: text.len(),
        end: text.len(),
        hard: true,
        hyphen: false,
    });

    let mut lines = Vec::with_capacity(breaks.len());
    let mut top = px(0.0);
    let last_break = breaks.len() - 1;
    for (line_index, line) in breaks.into_iter().enumerate() {
        let trimmed_end = line.start + text[line.start..line.content_end].trim_end().len();
        let base_x = x_at(line.start);
        let hyphen = line
            .hyphen
            .then(|| (line.content_end, hyphen_width(line.content_end)));
        let width = x_at(trimmed_end) - base_x + hyphen.map_or(px(0.0), |(_, width)| width);
        let free = (available - width).max(px(0.0));
        let justify = style.alignment == Alignment::Justified
            && line_index != last_break
            && !text[line.content_end..line.end]
                .chars()
                .any(is_forced_break);
        let offset = match style.alignment {
            Alignment::Left | Alignment::Justified => px(0.0),
            Alignment::Center => free / 2.0,
            Alignment::Right => free,
        };
        let mut gaps = Vec::new();
        if justify {
            let mut previous_space = false;
            for (index, character) in text[line.start..trimmed_end].char_indices() {
                let space = character.is_whitespace();
                if previous_space && !space {
                    gaps.push(line.start + index);
                }
                previous_space = space;
            }
        }
        let gap_extra = if gaps.is_empty() {
            px(0.0)
        } else {
            free / gaps.len() as f32
        };
        let left = indent + offset;
        let shift_at = |at: usize| gap_extra * gaps.iter().filter(|gap| **gap <= at).count() as f32;

        // Pieces: each text atom's share of the line, cut again at
        // justification gaps so each word can move, and at every character
        // of a tracked (kerned) atom.
        let mut pieces = Vec::new();
        for atom in &atoms {
            let Atom::Text {
                range,
                style_ranges,
            } = atom
            else {
                continue;
            };
            let from = range.start.max(line.start);
            let to = range.end.min(line.content_end);
            if from >= to {
                continue;
            }
            let atom_style = &style_ranges[0].1;
            let mut cuts: Vec<usize> = gaps.clone();
            if tracking(atom_style, params) != px(0.0) {
                cuts.extend(text[from..to].char_indices().map(|(at, _)| from + at));
            }
            cuts.sort_unstable();
            cuts.push(usize::MAX);
            let rise = px(atom_style.baseline_shift() * zoom);
            let mut piece_start = from;
            for &cut in &cuts {
                if cut <= piece_start {
                    continue;
                }
                let piece_end = cut.min(to);
                if piece_start < piece_end {
                    let sub = piece_start..piece_end;
                    let size = size_px(paragraph.style_of_char_at(sub.start), params);
                    let content = SharedString::from(text[sub.clone()].to_owned());
                    let shaped = text_system.shape_line(
                        content.clone(),
                        size,
                        &runs_for(paragraph, &sub, params, text_run),
                        None,
                    );
                    let fill = atom_style.outline.then(|| {
                        text_system.shape_line(
                            content,
                            size,
                            &runs_for(paragraph, &sub, params, fill_run),
                            None,
                        )
                    });
                    pieces.push(Piece {
                        x: left + (x_at(sub.start) - base_x) + shift_at(sub.start),
                        rise,
                        line: shaped,
                        fill,
                    });
                }
                piece_start = piece_end;
                if piece_start >= to {
                    break;
                }
            }
        }
        if let Some((at, _)) = hyphen {
            let style = paragraph.style_at(at);
            pieces.push(Piece {
                x: left + (x_at(at) - base_x) + shift_at(at),
                rise: px(style.baseline_shift() * zoom),
                line: text_system.shape_line(
                    SharedString::from("-"),
                    size_px(style, params),
                    &[text_run(style, 1, params)],
                    None,
                ),
                fill: None,
            });
        }

        let (mut ascent, mut descent) = (px(0.0), px(0.0));
        for piece in &pieces {
            ascent = ascent.max(piece.line.ascent + piece.rise);
            descent = descent.max(piece.line.descent - piece.rise);
        }
        if pieces.is_empty() {
            (ascent, descent) = metrics(paragraph.style_at(line.start), params, window);
        }
        let natural = f32::from(ascent + descent);
        let height = px((natural * style.line_spacing.max(0.5)).ceil());
        lines.push(LineBox {
            start: line.start,
            content_end: line.content_end,
            end: line.end,
            hard_end: line.hard,
            top,
            height,
            ascent,
            left,
            base_x,
            gaps,
            gap_extra,
            pieces,
        });
        top += height;
    }

    let marker = style.list.map(|kind| {
        let label = kind.marker(list_number.max(1));
        let marker_style = paragraph.style_of_char_at(0);
        let shaped = text_system.shape_line(
            SharedString::from(label.clone()),
            size_px(marker_style, params),
            &[text_run(marker_style, label.len(), params)],
            None,
        );
        Marker {
            x: px((LIST_MARKER_X + LIST_INDENT * level) * zoom),
            line: shaped,
        }
    });

    ParagraphLayout {
        text: SharedString::from(text.to_owned()),
        lines,
        height: top,
        right: params.width,
        marker,
        stops,
    }
}

impl ParagraphLayout {
    fn x_at(&self, byte: usize) -> Pixels {
        let index = self.stops.partition_point(|(at, _)| *at < byte);
        self.stops
            .get(index)
            .or(self.stops.last())
            .map_or(px(0.0), |(_, x)| *x)
    }

    /// The line a caret at `local` sits on.
    pub fn line_for_offset(&self, local: usize) -> usize {
        let last = self.lines.len() - 1;
        self.lines
            .iter()
            .position(|line| local < line.end || (line.hard_end && local <= line.content_end))
            .unwrap_or(last)
            .min(last)
    }

    /// The caret's x for `local` on line `line`.
    pub fn x_for(&self, line: usize, local: usize) -> Pixels {
        let line = &self.lines[line];
        let local = local.clamp(line.start, line.content_end.max(line.start));
        let shift = line.gap_extra * line.gaps.iter().filter(|gap| **gap <= local).count() as f32;
        line.left + (self.x_at(local) - line.base_x) + shift
    }

    /// The last caret position on `line` that still draws on it.
    pub fn line_caret_end(&self, line: usize) -> usize {
        let line = &self.lines[line];
        if line.hard_end {
            return line.content_end;
        }
        // A soft-wrapped line ends where the next begins; stop before the
        // space the wrap happened after so the caret stays on this line.
        self.text[line.start..line.content_end]
            .char_indices()
            .next_back()
            .filter(|(_, character)| character.is_whitespace())
            .map_or(line.content_end, |(index, _)| line.start + index)
    }

    /// The character boundary on `line` closest to `x`.
    pub fn offset_for_x(&self, line: usize, x: Pixels) -> usize {
        let start = self.lines[line].start;
        let end = self.line_caret_end(line);
        let mut best = start;
        let mut best_distance = f32::MAX;
        let candidates = self.text[start..end]
            .char_indices()
            .map(|(index, _)| start + index)
            .chain(std::iter::once(end));
        for candidate in candidates {
            let distance = (f32::from(self.x_for(line, candidate)) - f32::from(x)).abs();
            if distance < best_distance {
                best = candidate;
                best_distance = distance;
            }
        }
        best
    }

    /// The character boundary nearest a point in paragraph coordinates.
    pub fn offset_for_point(&self, x: Pixels, y: Pixels) -> usize {
        let last = self.lines.len() - 1;
        let line = self
            .lines
            .iter()
            .position(|line| y < line.top + line.height)
            .unwrap_or(last);
        self.offset_for_x(line, x)
    }

    /// The character under a point in paragraph coordinates (not the
    /// nearest boundary): what a click on a link lands on.
    pub fn char_at_point(&self, x: Pixels, y: Pixels) -> Option<usize> {
        let last = self.lines.len() - 1;
        let index = self
            .lines
            .iter()
            .position(|line| y < line.top + line.height)
            .unwrap_or(last);
        let line = &self.lines[index];
        let end = self.line_caret_end(index);
        self.text[line.start..end]
            .char_indices()
            .map(|(at, character)| (line.start + at, line.start + at + character.len_utf8()))
            .find(|(from, to)| x >= self.x_for(index, *from) && x < self.x_for(index, *to))
            .map(|(from, _)| from)
    }

    /// Caret rectangle `(x, top, height)` for `local`.
    pub fn caret(&self, local: usize) -> (Pixels, Pixels, Pixels) {
        let line_index = self.line_for_offset(local);
        let line = &self.lines[line_index];
        (self.x_for(line_index, local), line.top, line.height)
    }

    /// Selection rectangles `(left, top, right, bottom)` for `range`
    /// (local); `through_end` extends the last line to the right edge
    /// because the selection continues past this paragraph.
    pub fn selection_rects(
        &self,
        range: Range<usize>,
        through_end: bool,
    ) -> Vec<(Pixels, Pixels, Pixels, Pixels)> {
        let mut rects = Vec::new();
        let last = self.lines.len() - 1;
        for (index, line) in self.lines.iter().enumerate() {
            let line_last = if index == last {
                line.content_end
            } else {
                line.end
            };
            if range.end < line.start || range.start > line_last {
                continue;
            }
            let continues = range.end > line.content_end || (index == last && through_end);
            if range.start == range.end && !continues {
                continue;
            }
            let from = self.x_for(index, range.start.max(line.start));
            let to = if continues {
                self.right.max(from)
            } else {
                self.x_for(index, range.end.min(line.content_end))
            };
            if to > from {
                rects.push((from, line.top, to, line.top + line.height));
            }
        }
        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyphenation_splits_between_syllables_only_in_long_words() {
        let points = hyphenation_points("hyphenation");
        assert!(!points.is_empty());
        for point in &points {
            assert!(*point >= HYPHEN_MIN_HEAD);
            assert!("hyphenation".len() - point >= HYPHEN_MIN_TAIL);
        }
        // "hy-phen-a-tion": never inside the "ph" digraph.
        assert!(!points.contains(&3));
        assert!(hyphenation_points("short").is_empty());
        assert!(hyphenation_points("abc123def").is_empty());
        assert!(hyphenation_points("").is_empty());
    }

    #[test]
    fn hyphenation_points_land_on_char_boundaries() {
        let word = "caf\u{e9}terias";
        for point in hyphenation_points(word) {
            assert!(word.is_char_boundary(point));
        }
    }

    #[test]
    fn only_non_default_styles_ask_for_font_features() {
        assert!(features_for(&CharStyle::default()).is_empty());
        let none = CharStyle {
            kern: Some(0.0),
            ligatures: Ligatures::None,
            traditional: true,
            ..CharStyle::default()
        };
        let features = features_for(&none);
        assert!(features.contains(&("kern".to_owned(), 0)));
        assert!(features.contains(&("liga".to_owned(), 0)));
        assert!(features.contains(&("trad".to_owned(), 1)));
        let tight = CharStyle {
            kern: Some(-1.0),
            ligatures: Ligatures::All,
            ..CharStyle::default()
        };
        assert_eq!(features_for(&tight), [("dlig".to_owned(), 1)]);
    }
}
