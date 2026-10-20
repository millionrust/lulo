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

use super::model::{
    has_rtl, line_start_share, Alignment, CharStyle, Ligatures, ListKind, Paragraph, Rgb,
    WritingDirection,
};

/// TextEdit's default tab stops: every 28 points.
const TAB_INTERVAL: f32 = 28.0;

/// Where lists and quotes sit: a list item's text starts `list_indent`
/// points in per level, its marker `marker_x` points in from its level's
/// start; a block quote's text starts `quote_indent` points in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListMetrics {
    pub list_indent: f32,
    pub marker_x: f32,
    pub quote_indent: f32,
}

impl Default for ListMetrics {
    /// TextEdit's: 36-point list levels with the marker 11 points in.
    fn default() -> Self {
        Self {
            list_indent: 36.0,
            marker_x: 11.0,
            quote_indent: 16.0,
        }
    }
}
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
    pub metrics: ListMetrics,
    /// View ▸ Show Highlights (Notes): highlight backgrounds are drawn.
    pub show_highlights: bool,
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
    let highlight = style.highlight.filter(|_| params.show_highlights);
    let color = if style.link.is_some() && style.color.is_none() {
        params.link_color
    } else if let (None, Some(highlight)) = (style.color, highlight) {
        // Automatic text stays readable on a pale highlight, even when the
        // automatic colour is light (a dark background).
        if luminance(highlight) > 0.6 && params.default_color.l > 0.6 {
            gpui::black()
        } else {
            params.default_color
        }
    } else {
        style.color.map(hsla).unwrap_or(params.default_color)
    };
    TextRun {
        len,
        font: font_for(style, params),
        color,
        background_color: highlight.map(hsla),
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

/// Relative brightness of a document colour, 0 to 1.
fn luminance(color: Rgb) -> f32 {
    (0.2126 * f32::from(color.r) + 0.7152 * f32::from(color.g) + 0.0722 * f32::from(color.b))
        / 255.0
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
    /// End of the line's text before its trailing white space.
    pub visual_end: usize,
    /// Bidirectional text: the line's directional runs in visual order,
    /// left to right. Empty for a line that reads plainly left to right.
    pub runs: Vec<VisualRun>,
}

/// One directional run of a bidirectional line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VisualRun {
    /// The logical bytes the run holds.
    pub range: Range<usize>,
    pub rtl: bool,
    /// Visual left edge, in paragraph coordinates.
    pub x: Pixels,
    pub width: Pixels,
}

/// A Notes checklist item's round checkbox.
pub(crate) struct Checkbox {
    pub x: Pixels,
    pub diameter: Pixels,
    pub checked: bool,
}

pub(crate) struct Marker {
    pub x: Pixels,
    /// The marker's glyphs: a list label, or a ticked checkbox's check.
    pub line: Option<ShapedLine>,
    pub checkbox: Option<Checkbox>,
}

pub(crate) struct ParagraphLayout {
    pub text: SharedString,
    pub lines: Vec<LineBox>,
    pub height: Pixels,
    /// The right edge selection highlights reach.
    pub right: Pixels,
    pub marker: Option<Marker>,
    /// Format ▸ Block Quote: the x of a bar down the paragraph.
    pub quote_bar: Option<Pixels>,
    /// The paragraph reads right to left.
    pub rtl: bool,
    /// `(byte, x)` for every glyph start, sorted, ending at the text's end.
    /// x runs along the text in logical (reading) order.
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

/// Embedding levels of `paragraph`'s characters (indexed by byte; only a
/// character's first byte is meaningful) from the Unicode bidirectional
/// algorithm, with the paragraph's base direction. A run carrying its own
/// Format ▸ Text ▸ Writing Direction ▸ Selection is embedded as LRE/RLE …
/// PDF would embed it.
pub(crate) fn bidi_levels(paragraph: &Paragraph, base_rtl: bool) -> Vec<u8> {
    use unicode_bidi::{BidiInfo, Level};
    let text = paragraph.text();
    let mut analysis = String::with_capacity(text.len() + 8);
    let mut map = vec![0_usize; text.len()];
    for (range, style) in paragraph.styled_ranges() {
        let embedding = match style.embedding {
            WritingDirection::Default => None,
            WritingDirection::LeftToRight => Some('\u{202A}'),
            WritingDirection::RightToLeft => Some('\u{202B}'),
        };
        if let Some(control) = embedding {
            analysis.push(control);
        }
        for (offset, character) in text[range.clone()].char_indices() {
            map[range.start + offset] = analysis.len();
            analysis.push(character);
        }
        if embedding.is_some() {
            analysis.push('\u{202C}');
        }
    }
    let base = if base_rtl { Level::rtl() } else { Level::ltr() };
    let mut levels = vec![base.number(); text.len()];
    if analysis.is_empty() {
        return levels;
    }
    let info = BidiInfo::new(&analysis, Some(base));
    for (at, _) in text.char_indices() {
        levels[at] = info
            .levels
            .get(map[at])
            .map_or(base.number(), |level| level.number());
    }
    levels
}

/// Rules L1 and L2 for the line `range` of `text`: its directional runs in
/// visual order, left to right, each with whether it reads right to left.
/// Tabs and the white space before them take the paragraph's own level.
pub(crate) fn visual_runs(
    text: &str,
    levels: &[u8],
    range: Range<usize>,
    base: u8,
) -> Vec<(Range<usize>, bool)> {
    let mut chars: Vec<(usize, char, u8)> = text[range.clone()]
        .char_indices()
        .map(|(offset, character)| {
            let at = range.start + offset;
            (at, character, levels.get(at).copied().unwrap_or(base))
        })
        .collect();
    let mut before_tab = true;
    for (_, character, level) in chars.iter_mut().rev() {
        if *character == '\t' {
            *level = base;
            before_tab = true;
        } else if before_tab && character.is_whitespace() {
            *level = base;
        } else {
            before_tab = false;
        }
    }
    let mut runs: Vec<(Range<usize>, u8)> = Vec::new();
    for (at, character, level) in chars {
        let end = at + character.len_utf8();
        match runs.last_mut() {
            Some((run, run_level)) if *run_level == level && run.end == at => run.end = end,
            _ => runs.push((at..end, level)),
        }
    }
    let highest = runs.iter().map(|(_, level)| *level).max().unwrap_or(base);
    let lowest_odd = runs
        .iter()
        .map(|(_, level)| *level)
        .filter(|level| level % 2 == 1)
        .min();
    if let Some(lowest_odd) = lowest_odd {
        let mut level = highest;
        while level >= lowest_odd {
            let mut index = 0;
            while index < runs.len() {
                if runs[index].1 >= level {
                    let mut end = index;
                    while end < runs.len() && runs[end].1 >= level {
                        end += 1;
                    }
                    runs[index..end].reverse();
                    index = end;
                } else {
                    index += 1;
                }
            }
            if level == 0 {
                break;
            }
            level -= 1;
        }
    }
    runs.into_iter()
        .map(|(range, level)| (range, level % 2 == 1))
        .collect()
}

/// Glyph starts of one shaped stretch, in logical order, as `(byte, x)`
/// with x the advance of everything before the glyph in reading order.
/// A right-to-left stretch is shaped right to left, so its glyphs' visual
/// positions run backwards; their advances are added up in reading order.
fn logical_stops(shaped: &gpui::LineLayout, rtl: bool) -> Vec<(usize, Pixels)> {
    let glyphs: Vec<(usize, Pixels)> = shaped
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .map(|glyph| (glyph.index, glyph.position.x))
        .collect();
    if !rtl {
        return glyphs;
    }
    let mut by_x = glyphs.clone();
    by_x.sort_by(|left, right| f32::from(left.1).total_cmp(&f32::from(right.1)));
    let mut advances: Vec<(usize, Pixels)> = by_x
        .iter()
        .enumerate()
        .map(|(index, (byte, x))| {
            let next = by_x.get(index + 1).map_or(shaped.width, |(_, x)| *x);
            (*byte, (next - *x).max(px(0.0)))
        })
        .collect();
    advances.sort_by_key(|(byte, _)| *byte);
    let mut x = px(0.0);
    advances
        .into_iter()
        .map(|(byte, advance)| {
            let stop = (byte, x);
            x += advance;
            stop
        })
        .collect()
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
    let list_metrics = params.metrics;
    let level = f32::from(style.list_level);
    let rtl = style.direction.is_rtl(text);
    let quote_indent = if style.quote {
        px(list_metrics.quote_indent * zoom)
    } else {
        px(0.0)
    };
    // A list item is indented one level more than its nesting; any other
    // paragraph by its indentation level (Format ▸ Indentation).
    let indent = quote_indent
        + if style.list.is_some() {
            px(list_metrics.list_indent * (level + 1.0) * zoom)
        } else {
            px(list_metrics.list_indent * level * zoom)
        };
    let available = (params.width - indent).max(px(1.0));
    // A right-to-left paragraph is indented from the right.
    let indent_left = if rtl { px(0.0) } else { indent };
    let tab = px(TAB_INTERVAL * zoom);
    let text_system = window.text_system();

    // Bidirectional text needs the bidi algorithm; plain left-to-right
    // text (nearly every paragraph) skips it.
    let embedded = paragraph
        .runs()
        .iter()
        .any(|run| run.style.embedding != WritingDirection::Default);
    let base_level = u8::from(rtl);
    let levels = (rtl || embedded || has_rtl(text)).then(|| bidi_levels(paragraph, rtl));
    let level_at = |at: usize| -> u8 {
        levels
            .as_ref()
            .and_then(|levels| levels.get(at).copied())
            .unwrap_or(base_level)
    };

    // Glyph positions along one unwrapped line, in reading order.
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
                // Each stretch of one direction is shaped on its own.
                let mut stretches: Vec<Range<usize>> = Vec::new();
                if levels.is_some() {
                    let mut stretch_start = range.start;
                    let mut stretch_level = level_at(range.start);
                    for (offset, _) in text[range.clone()].char_indices() {
                        let at = range.start + offset;
                        let at_level = level_at(at);
                        if at_level != stretch_level {
                            stretches.push(stretch_start..at);
                            stretch_start = at;
                            stretch_level = at_level;
                        }
                    }
                    stretches.push(stretch_start..range.end);
                } else {
                    stretches.push(range.clone());
                }
                for stretch in stretches {
                    let runs: Vec<TextRun> = style_ranges
                        .iter()
                        .filter_map(|(sub, style)| {
                            let start = sub.start.max(stretch.start);
                            let end = sub.end.min(stretch.end);
                            (start < end).then(|| text_run(style, end - start, params))
                        })
                        .collect();
                    let shaped =
                        text_system.layout_line(&text[stretch.clone()], size, &runs, None);
                    let stretch_rtl = level_at(stretch.start) % 2 == 1;
                    let mut glyphs = 0_usize;
                    for (byte, glyph_x) in logical_stops(&shaped, stretch_rtl) {
                        stops.push((stretch.start + byte, x + glyph_x + extra * glyphs as f32));
                        glyphs += 1;
                    }
                    x += shaped.width + extra * glyphs as f32;
                }
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
        let offset = free * line_start_share(style.alignment, rtl, justify);
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
        let left = indent_left + offset;
        let shift_at = |at: usize| gap_extra * gaps.iter().filter(|gap| **gap <= at).count() as f32;
        // Logical position along the line, justification included.
        let along = |at: usize| x_at(at) - base_x + shift_at(at);

        // The line's directional runs, placed left to right.
        let mut runs = Vec::new();
        if let Some(levels) = &levels {
            let ordered = visual_runs(text, levels, line.start..trimmed_end, base_level);
            let plain = ordered.len() <= 1 && ordered.iter().all(|(_, run_rtl)| !run_rtl);
            if !plain {
                let mut cursor = left;
                for (range, run_rtl) in ordered {
                    let run_width = along(range.end) - along(range.start);
                    runs.push(VisualRun {
                        range,
                        rtl: run_rtl,
                        x: cursor,
                        width: run_width,
                    });
                    cursor += run_width;
                }
            }
        }

        // Pieces: each text atom's share of the line, cut again at
        // justification gaps so each word can move, at every character of
        // a tracked (kerned) atom, and where the direction changes.
        let mut pieces = Vec::new();
        let piece_end_limit = if runs.is_empty() {
            line.content_end
        } else {
            trimmed_end
        };
        for atom in &atoms {
            let Atom::Text {
                range,
                style_ranges,
            } = atom
            else {
                continue;
            };
            let from = range.start.max(line.start);
            let to = range.end.min(piece_end_limit);
            if from >= to {
                continue;
            }
            let atom_style = &style_ranges[0].1;
            let mut cuts: Vec<usize> = gaps.clone();
            if tracking(atom_style, params) != px(0.0) {
                cuts.extend(text[from..to].char_indices().map(|(at, _)| from + at));
            }
            cuts.extend(runs.iter().map(|run: &VisualRun| run.range.start));
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
                    let piece_x = match runs
                        .iter()
                        .find(|run| run.range.start <= sub.start && sub.start < run.range.end)
                    {
                        Some(run) if run.rtl => run.x + along(run.range.end) - along(sub.end),
                        Some(run) => run.x + along(sub.start) - along(run.range.start),
                        None => left + along(sub.start),
                    };
                    pieces.push(Piece {
                        x: piece_x,
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
                x: left + along(at),
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
            visual_end: trimmed_end,
            runs,
        });
        top += height;
    }

    let marker = style.list.map(|kind| {
        let marker_style = paragraph.style_of_char_at(0);
        let marker_in = px((list_metrics.marker_x + list_metrics.list_indent * level) * zoom);
        let flip = |x: Pixels, width: Pixels| {
            if rtl {
                params.width - x - width
            } else {
                x
            }
        };
        if kind == ListKind::Checkbox {
            // The circle is a little taller than the text's capitals and
            // centred on the first line, as in Notes.
            let diameter = px((marker_style.size * 1.3 * zoom).round().max(10.0));
            let line = style.checked.then(|| {
                let check = marker_style.size * 0.9;
                let check_style = CharStyle {
                    bold: true,
                    size: check,
                    ..CharStyle::default()
                };
                let mut run = text_run(&check_style, "\u{2713}".len(), params);
                run.color = gpui::white();
                text_system.shape_line(
                    SharedString::from("\u{2713}"),
                    px(check * zoom),
                    &[run],
                    None,
                )
            });
            return Marker {
                x: flip(marker_in - diameter * 0.5 + px(2.0 * zoom), diameter),
                line,
                checkbox: Some(Checkbox {
                    x: flip(marker_in - diameter * 0.5 + px(2.0 * zoom), diameter),
                    diameter,
                    checked: style.checked,
                }),
            };
        }
        let label = kind.marker(list_number.max(1));
        let shaped = text_system.shape_line(
            SharedString::from(label.clone()),
            size_px(marker_style, params),
            &[text_run(marker_style, label.len(), params)],
            None,
        );
        let width = shaped.width;
        Marker {
            x: flip(marker_in, width),
            line: Some(shaped),
            checkbox: None,
        }
    });
    let quote_bar = style.quote.then(|| {
        if rtl {
            params.width - px(3.0 * zoom)
        } else {
            px(0.0)
        }
    });

    ParagraphLayout {
        text: SharedString::from(text.to_owned()),
        lines,
        height: top,
        right: params.width,
        marker,
        quote_bar,
        rtl,
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

    /// Position of `local` along `line` in reading order, from the line's
    /// start, justification included.
    fn along(&self, line: &LineBox, local: usize) -> Pixels {
        let shift = line.gap_extra * line.gaps.iter().filter(|gap| **gap <= local).count() as f32;
        self.x_at(local) - line.base_x + shift
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

    /// x of `local` inside one directional run.
    fn x_in_run(&self, line: &LineBox, run: &VisualRun, local: usize) -> Pixels {
        if run.rtl {
            run.x + self.along(line, run.range.end) - self.along(line, local)
        } else {
            run.x + self.along(line, local) - self.along(line, run.range.start)
        }
    }

    /// The caret's x for `local` on line `line`.
    pub fn x_for(&self, line: usize, local: usize) -> Pixels {
        let line = &self.lines[line];
        let local = local.clamp(line.start, line.content_end.max(line.start));
        if line.runs.is_empty() {
            return line.left + self.along(line, local);
        }
        if local >= line.visual_end {
            // Trailing white space continues past the logical end.
            let beyond = self.along(line, local) - self.along(line, line.visual_end);
            let (left, right) = line.runs.iter().fold((Pixels::MAX, px(0.0)), |acc, run| {
                (acc.0.min(run.x), acc.1.max(run.x + run.width))
            });
            return if self.rtl { left - beyond } else { right + beyond };
        }
        // At a direction change the caret stays with the text before it
        // when that text reads in the paragraph's own direction.
        if local > line.start {
            let previous = self.text[..local]
                .char_indices()
                .next_back()
                .map_or(line.start, |(at, _)| at);
            if let Some(run) = line
                .runs
                .iter()
                .find(|run| run.range.start <= previous && previous < run.range.end)
            {
                if run.range.end == local && run.rtl == self.rtl {
                    return self.x_in_run(line, run, local);
                }
            }
        }
        match line
            .runs
            .iter()
            .find(|run| run.range.start <= local && local < run.range.end)
        {
            Some(run) => self.x_in_run(line, run, local),
            None => line.left + self.along(line, local),
        }
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

    /// The left and right edges of the character `from..to` on `line`.
    fn char_extent(&self, index: usize, from: usize, to: usize) -> (Pixels, Pixels) {
        let line = &self.lines[index];
        match line
            .runs
            .iter()
            .find(|run| run.range.start <= from && from < run.range.end)
        {
            Some(run) => {
                let a = self.x_in_run(line, run, from);
                let b = self.x_in_run(line, run, to.min(run.range.end));
                (a.min(b), a.max(b))
            }
            None => (self.x_for(index, from), self.x_for(index, to)),
        }
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
            .find(|(from, to)| {
                let (left, right) = self.char_extent(index, *from, *to);
                x >= left && x < right
            })
            .map(|(from, _)| from)
    }

    /// Caret rectangle `(x, top, height)` for `local`.
    pub fn caret(&self, local: usize) -> (Pixels, Pixels, Pixels) {
        let line_index = self.line_for_offset(local);
        let line = &self.lines[line_index];
        (self.x_for(line_index, local), line.top, line.height)
    }

    /// Selection rectangles `(left, top, right, bottom)` for `range`
    /// (local); `through_end` extends the last line to the trailing edge
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
            let bottom = line.top + line.height;
            if !line.runs.is_empty() {
                // One highlight per directional run the selection covers.
                let mut far = (Pixels::MAX, px(0.0));
                for run in &line.runs {
                    far = (far.0.min(run.x), far.1.max(run.x + run.width));
                    let from = range.start.max(run.range.start);
                    let to = range.end.min(run.range.end);
                    if from < to {
                        let a = self.x_in_run(line, run, from);
                        let b = self.x_in_run(line, run, to);
                        rects.push((a.min(b), line.top, a.max(b), bottom));
                    }
                }
                if continues {
                    if self.rtl {
                        rects.push((px(0.0), line.top, far.0.max(px(0.0)), bottom));
                    } else {
                        rects.push((far.1, line.top, self.right.max(far.1), bottom));
                    }
                }
                continue;
            }
            let from = self.x_for(index, range.start.max(line.start));
            let to = if continues {
                self.right.max(from)
            } else {
                self.x_for(index, range.end.min(line.content_end))
            };
            if to > from {
                rects.push((from, line.top, to, bottom));
            }
        }
        rects
    }

    /// The checkbox's bounds `(x, top, diameter)` in paragraph
    /// coordinates, for painting and clicking.
    pub fn checkbox_bounds(&self) -> Option<(Pixels, Pixels, Pixels, bool)> {
        let checkbox = self.marker.as_ref()?.checkbox.as_ref()?;
        let first = self.lines.first()?;
        let top = first.top + (first.height - checkbox.diameter) / 2.0;
        Some((checkbox.x, top, checkbox.diameter, checkbox.checked))
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::ParagraphStyle;
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

    fn runs_of(text: &str, style: ParagraphStyle) -> Vec<(String, bool)> {
        let paragraph = Paragraph::plain(text, &CharStyle::default(), style);
        let rtl = style.direction.is_rtl(text);
        let levels = bidi_levels(&paragraph, rtl);
        visual_runs(text, &levels, 0..text.len(), u8::from(rtl))
            .into_iter()
            .map(|(range, rtl)| (text[range].to_owned(), rtl))
            .collect()
    }

    #[test]
    fn hebrew_inside_a_left_to_right_line_is_its_own_right_to_left_run() {
        let runs = runs_of("abc \u{5d0}\u{5d1}\u{5d2} def", ParagraphStyle::default());
        assert_eq!(
            runs,
            [
                ("abc ".to_owned(), false),
                ("\u{5d0}\u{5d1}\u{5d2}".to_owned(), true),
                (" def".to_owned(), false),
            ]
        );
    }

    #[test]
    fn a_right_to_left_line_puts_its_first_word_on_the_right() {
        let text = "\u{5d0}\u{5d1}\u{5d2} abc \u{5d3}\u{5d4}\u{5d5}";
        let runs = runs_of(text, ParagraphStyle::default());
        assert_eq!(runs.len(), 3);
        assert!(runs[0].1 && !runs[1].1 && runs[2].1);
        assert_eq!(runs[1].0, "abc");
        assert!(runs[2].0.contains('\u{5d0}'));
        assert!(runs[0].0.contains('\u{5d3}'));
    }

    #[test]
    fn a_forced_right_to_left_paragraph_reverses_latin_words() {
        // Latin in a right-to-left paragraph keeps its letters in order
        // but the paragraph reads from the right: one run, left to right.
        let style = ParagraphStyle {
            direction: WritingDirection::RightToLeft,
            ..ParagraphStyle::default()
        };
        let runs = runs_of("abc def", style);
        assert_eq!(runs, [("abc def".to_owned(), false)]);
    }

    #[test]
    fn a_right_to_left_selection_embedding_reorders_its_words() {
        let text = "abc \u{5d0}\u{5d1} def";
        let mut document =
            super::super::model::Document::from_plain_text(text, &CharStyle::default());
        let len = text.len();
        let words = |levels: &[u8]| -> Vec<String> {
            visual_runs(text, levels, 0..len, 0)
                .into_iter()
                .map(|(range, _)| text[range].trim().to_owned())
                .collect()
        };
        let plain = bidi_levels(document.paragraph(0), false);
        assert_eq!(words(&plain), ["abc", "\u{5d0}\u{5d1}", "def"]);
        document.update_char_style(0..len, |style| {
            style.embedding = WritingDirection::RightToLeft;
        });
        let embedded = bidi_levels(document.paragraph(0), false);
        assert_eq!(words(&embedded), ["def", "\u{5d0}\u{5d1}", "abc"]);
    }

    #[test]
    fn right_to_left_glyphs_are_measured_in_reading_order() {
        use gpui::{point, FontId, GlyphId, LineLayout, ShapedGlyph, ShapedRun};
        // Three 10-wide glyphs shaped right to left: byte 0 is drawn
        // rightmost.
        let glyph = |index: usize, x: f32| ShapedGlyph {
            id: GlyphId(0),
            position: point(px(x), px(0.0)),
            index,
            is_emoji: false,
        };
        let shaped = LineLayout {
            width: px(30.0),
            runs: vec![ShapedRun {
                font_id: FontId(0),
                glyphs: vec![glyph(4, 0.0), glyph(2, 10.0), glyph(0, 20.0)],
            }],
            len: 6,
            ..LineLayout::default()
        };
        assert_eq!(
            logical_stops(&shaped, true),
            [(0, px(0.0)), (2, px(10.0)), (4, px(20.0))]
        );
        assert_eq!(logical_stops(&shaped, false).len(), 3);
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
