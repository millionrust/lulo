//! Formatted (rich-text) document rendering: styled runs with sizes, faces,
//! colours, highlights, underline/strikethrough and per-line alignment,
//! rasterised in colour one page at a time so memory stays one page deep.

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight,
    Wrap,
};

use crate::model::{Error, PageLayout, MAX_PDF_BYTES};
use crate::render::{assemble_pdf, compress, validate_layout, PageImage};
use crate::{MAX_PAGES, MAX_SOURCE_BYTES};

/// One styled stretch of text.
#[derive(Clone, Debug, PartialEq)]
pub struct RichSpan {
    pub text: String,
    /// The document's own family name; `None` is the default sans-serif face.
    pub family: Option<String>,
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub color: (u8, u8, u8),
    pub highlight: Option<(u8, u8, u8)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RichAlign {
    #[default]
    Left,
    Center,
    Right,
    Justified,
}

/// One printed line of the source (a paragraph, or part of one ended by a
/// line break). Its text must not contain `\n`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichLine {
    pub spans: Vec<RichSpan>,
    pub align: RichAlign,
    /// Line-height multiple.
    pub line_spacing: f32,
}

fn family(name: Option<&str>) -> Family<'_> {
    let Some(name) = name else {
        return Family::SansSerif;
    };
    let lower = name.to_ascii_lowercase();
    if ["menlo", "monaco", "courier", "sf mono", "andale mono"]
        .iter()
        .any(|mono| lower.starts_with(mono))
    {
        Family::Monospace
    } else if [
        "helvetica",
        ".applesystemuifont",
        "sf pro",
        "lucida grande",
        "arial",
    ]
    .iter()
    .any(|face| lower.starts_with(face))
    {
        Family::SansSerif
    } else {
        Family::Name(name.split('-').next().unwrap_or(name))
    }
}

/// Render formatted lines into a bounded, self-contained colour PDF. Like
/// [`crate::render_pdf`], this is synchronous and belongs on a background
/// executor; no text or path reaches PDF metadata.
pub fn render_rich_pdf(lines: &[RichLine], layout: PageLayout) -> Result<Vec<u8>, Error> {
    let source_bytes: usize = lines
        .iter()
        .flat_map(|line| &line.spans)
        .map(|span| span.text.len())
        .sum();
    if source_bytes > MAX_SOURCE_BYTES {
        return Err(Error::SourceTooLarge);
    }
    let layout = validate_layout(layout)?;
    let px_per_pt = layout.font_size_px / 11.0;
    let spans: Vec<&RichSpan> = lines.iter().flat_map(|line| &line.spans).collect();

    let mut font_system = FontSystem::new();
    let default_metrics = Metrics::new(12.0 * px_per_pt, 14.0 * px_per_pt);
    let mut buffer = Buffer::new(&mut font_system, default_metrics);
    buffer.set_size(&mut font_system, Some(layout.content_width_px as f32), None);
    buffer.set_wrap(&mut font_system, Wrap::WordOrGlyph);

    // One buffer line per source line; span metadata indexes `spans`.
    let mut pieces: Vec<(String, Attrs)> = Vec::new();
    let mut index = 0;
    for (line_index, line) in lines.iter().enumerate() {
        if line_index > 0 {
            pieces.push(("\n".to_owned(), Attrs::new().metadata(usize::MAX)));
        }
        let spacing = if line.line_spacing.is_finite() && line.line_spacing > 0.0 {
            line.line_spacing.clamp(0.5, 4.0)
        } else {
            1.0
        };
        for span in &line.spans {
            let size = span.size_pt.clamp(4.0, 288.0) * px_per_pt;
            let (r, g, b) = span.color;
            let mut attrs = Attrs::new()
                .family(family(span.family.as_deref()))
                .metrics(Metrics::new(size, size * 1.2 * spacing))
                .color(Color::rgb(r, g, b))
                .metadata(index);
            if span.bold {
                attrs = attrs.weight(Weight::BOLD);
            }
            if span.italic {
                attrs = attrs.style(Style::Italic);
            }
            pieces.push((span.text.replace(['\n', '\r'], " "), attrs));
            index += 1;
        }
    }
    buffer.set_rich_text(
        &mut font_system,
        pieces
            .iter()
            .map(|(text, attrs)| (text.as_str(), attrs.clone())),
        &Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
        None,
    );
    for (buffer_line, line) in buffer.lines.iter_mut().zip(lines) {
        buffer_line.set_align(Some(match line.align {
            RichAlign::Left => Align::Left,
            RichAlign::Center => Align::Center,
            RichAlign::Right => Align::Right,
            RichAlign::Justified => Align::Justified,
        }));
    }
    buffer.shape_until_scroll(&mut font_system, false);

    // Paginate whole layout lines: (top, bottom) of each page's slice.
    let content_height = layout.content_height_px as f32;
    let mut pages: Vec<(f32, f32)> = vec![(0.0, 0.0)];
    for run in buffer.layout_runs() {
        let bottom = run.line_top + run.line_height;
        let current = pages.last_mut().expect("one page");
        if bottom - current.0 > content_height && current.1 > current.0 {
            pages.push((run.line_top, bottom));
        } else {
            current.1 = bottom;
        }
    }
    if pages.len() > MAX_PAGES {
        return Err(Error::TooManyPages);
    }

    let row_bytes = layout.width_px.checked_mul(3).ok_or(Error::RasterLimit)?;
    let page_bytes = row_bytes
        .checked_mul(layout.height_px)
        .ok_or(Error::RasterLimit)?;
    let mut cache = SwashCache::new();
    let mut images = Vec::with_capacity(pages.len());
    let mut compressed_total = 0_usize;
    let mut rendered_pixel = false;
    for &(page_top, page_bottom) in &pages {
        let mut raster = vec![u8::MAX; page_bytes];
        let mut blend = |x: i32, y: i32, (r, g, b): (u8, u8, u8), alpha: u8| {
            let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
                return;
            };
            let x = layout.margin_left_px + x;
            let y = layout.margin_top_px + y;
            if x >= layout.width_px || y >= layout.height_px || alpha == 0 {
                return;
            }
            let offset = y * row_bytes + x * 3;
            let a = u32::from(alpha);
            for (channel, value) in raster[offset..offset + 3].iter_mut().zip([r, g, b]) {
                *channel = ((u32::from(value) * a + u32::from(*channel) * (255 - a)) / 255) as u8;
            }
        };
        for run in buffer.layout_runs() {
            if run.line_top < page_top || run.line_top >= page_bottom {
                continue;
            }
            let top = run.line_top - page_top;
            let baseline = run.line_y - page_top;
            // Highlights first, then glyphs, then lines through them.
            for glyph in run.glyphs {
                let Some(span) = spans.get(glyph.metadata) else {
                    continue;
                };
                if let Some(color) = span.highlight {
                    let x0 = glyph.x.floor() as i32;
                    let x1 = (glyph.x + glyph.w).ceil() as i32;
                    let y0 = top.floor() as i32;
                    let y1 = (top + run.line_height).ceil() as i32;
                    for y in y0..y1 {
                        for x in x0..x1 {
                            blend(x, y, color, u8::MAX);
                        }
                    }
                }
            }
            for glyph in run.glyphs {
                let physical = glyph.physical((0.0, baseline), 1.0);
                let base = glyph.color_opt.unwrap_or(Color::rgb(0, 0, 0));
                cache.with_pixels(&mut font_system, physical.cache_key, base, |x, y, color| {
                    if color.a() > 0 {
                        rendered_pixel = true;
                    }
                    blend(
                        physical.x + x,
                        physical.y + y,
                        (color.r(), color.g(), color.b()),
                        color.a(),
                    );
                });
                let Some(span) = spans.get(glyph.metadata) else {
                    continue;
                };
                let thickness = (glyph.font_size / 14.0).max(1.0).round() as i32;
                let x0 = glyph.x.floor() as i32;
                let x1 = (glyph.x + glyph.w).ceil() as i32;
                let mut rule = |y: i32| {
                    for dy in 0..thickness {
                        for x in x0..x1 {
                            blend(x, y + dy, span.color, u8::MAX);
                        }
                    }
                };
                if span.underline {
                    rule((baseline + glyph.font_size * 0.12).round() as i32);
                }
                if span.strikethrough {
                    rule((baseline - glyph.font_size * 0.3).round() as i32);
                }
            }
        }
        let compressed = compress(&raster)?;
        compressed_total += compressed.len();
        if compressed_total > MAX_PDF_BYTES {
            return Err(Error::Encode);
        }
        images.push(PageImage {
            compressed,
            color_space: "/DeviceRGB",
            bits_per_component: 8,
        });
    }
    let has_text = spans.iter().any(|span| {
        span.text
            .chars()
            .any(|character| !character.is_whitespace())
    });
    if !rendered_pixel && has_text {
        return Err(Error::FontUnavailable);
    }
    assemble_pdf(&images, &layout)
}
