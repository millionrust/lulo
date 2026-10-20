//! The geometry of every markup shape, in one place (PREV-03B), so what is
//! drawn over a page, what is baked into an image's pixels and what a PDF
//! appearance stream describes are the same outline.
//!
//! Outlines are in the annotation canvas's own units: PDF points for a page,
//! pixels for an image. `unit` is how many of those units one "point" of
//! line width, arrow head or note icon is.

use crate::markup::{Annotation, Tool};

#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub points: Vec<(f32, f32)>,
    pub closed: bool,
    /// `(colour 0xRRGGBB, alpha)` when the outline is filled.
    pub fill: Option<(u32, f32)>,
    /// `(colour 0xRRGGBB, width)` when the outline is stroked.
    pub stroke: Option<(u32, f32)>,
}

/// How many pixels one markup "point" is on a picture of `size`: line
/// widths, type sizes and arrow heads grow with the picture so they read
/// the same on a thumbnail-sized image and on a 12-megapixel photo.
pub fn image_unit(size: (f32, f32)) -> f32 {
    (size.0.max(size.1) / 900.0).max(1.0)
}

/// Opacity of a highlighter, as the PDF writer's `/CA`.
pub const HIGHLIGHT_ALPHA: f32 = 0.4;

fn ellipse(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32)> {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (rx, ry) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    (0..48)
        .map(|step| {
            let angle = step as f32 * std::f32::consts::TAU / 48.0;
            (cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect()
}

fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// `path` points are fractions of the annotation rectangle, except while the
/// shape is still being drawn (`absolute_path`), when they are fractions of
/// the whole canvas.
fn placed(
    item: &Annotation,
    size: (f32, f32),
    absolute_path: bool,
) -> impl Iterator<Item = (f32, f32)> + '_ {
    let r = item.rect;
    item.path.iter().map(move |&(x, y)| {
        if absolute_path {
            (x * size.0, y * size.1)
        } else {
            (
                (r.x0 + x * (r.x1 - r.x0)) * size.0,
                (r.y0 + y * (r.y1 - r.y0)) * size.1,
            )
        }
    })
}

/// The outlines of `item` on a canvas of `size`. Text, Highlight, Underline
/// and Strike-through are drawn by their own code (text needs a font; the
/// markers follow the text); everything else comes through here.
pub fn outlines(
    item: &Annotation,
    size: (f32, f32),
    unit: f32,
    absolute_path: bool,
) -> Vec<Outline> {
    let r = item.rect;
    let (x0, y0, x1, y1) = (r.x0 * size.0, r.y0 * size.1, r.x1 * size.0, r.y1 * size.1);
    let (w, h) = (x1 - x0, y1 - y0);
    let width = item.width.max(0.5) * unit;
    let stroke = Some((item.color, width));
    let fill = item.fill.map(|colour| (colour, 1.0));
    match item.tool {
        Tool::Rectangle => vec![Outline {
            points: rectangle(x0, y0, x1, y1),
            closed: true,
            fill,
            stroke,
        }],
        Tool::Oval => vec![Outline {
            points: ellipse(x0, y0, x1, y1),
            closed: true,
            fill,
            stroke,
        }],
        Tool::Line | Tool::Arrow => {
            let points: Vec<_> = placed(item, size, absolute_path).collect();
            let start = points.first().copied().unwrap_or((x0, y0));
            let end = points.get(1).copied().unwrap_or((x1, y1));
            let mut out = vec![Outline {
                points: vec![start, end],
                closed: false,
                fill: None,
                stroke,
            }];
            if item.tool == Tool::Arrow {
                let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                let length = dx.hypot(dy).max(1.0);
                let (ux, uy) = (dx / length, dy / length);
                let (head, wing) = (12.0 * unit, 5.0 * unit);
                out.push(Outline {
                    points: vec![
                        (end.0 - head * ux - wing * uy, end.1 - head * uy + wing * ux),
                        end,
                        (end.0 - head * ux + wing * uy, end.1 - head * uy - wing * ux),
                    ],
                    closed: false,
                    fill: None,
                    stroke,
                });
            }
            out
        }
        Tool::Sketch | Tool::Signature => {
            let points: Vec<_> = placed(item, size, absolute_path).collect();
            if points.is_empty() {
                return Vec::new();
            }
            vec![Outline {
                points,
                closed: false,
                fill: None,
                stroke,
            }]
        }
        Tool::Polygon => {
            let points: Vec<_> = placed(item, size, absolute_path).collect();
            if points.len() < 2 {
                return Vec::new();
            }
            vec![Outline {
                points,
                closed: true,
                fill,
                stroke,
            }]
        }
        Tool::Star => {
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let outer = w.min(h) / 2.0;
            let inner = outer * 0.382;
            let points = (0..10)
                .map(|index| {
                    let radius = if index % 2 == 0 { outer } else { inner };
                    let angle =
                        std::f32::consts::FRAC_PI_2 - index as f32 * std::f32::consts::PI / 5.0;
                    // The angle runs counter-clockwise from the right with y
                    // up; canvas y points down.
                    (cx + radius * angle.cos(), cy - radius * angle.sin())
                })
                .collect();
            vec![Outline {
                points,
                closed: true,
                fill,
                stroke,
            }]
        }
        Tool::SpeechBubble => vec![
            Outline {
                points: rectangle(x0, y0, x1, y1),
                closed: true,
                fill,
                stroke,
            },
            // The tail, kept inside the box's own bounds.
            Outline {
                points: vec![
                    (x0 + w * 0.25, y0 + h * 0.85),
                    (x0 + w * 0.08, y0 + h * 0.98),
                    (x0 + w * 0.35, y0 + h * 0.85),
                ],
                closed: false,
                fill: None,
                stroke,
            },
        ],
        Tool::Mask => vec![Outline {
            points: rectangle(x0, y0, x1, y1),
            closed: true,
            fill: Some((item.color, 1.0)),
            stroke: None,
        }],
        Tool::Note => {
            let icon = (20.0 * unit).min(w).min(h);
            vec![
                Outline {
                    points: rectangle(x0, y0, x0 + icon, y0 + icon),
                    closed: true,
                    fill: Some((item.color, 1.0)),
                    stroke: None,
                },
                // The folded corner.
                Outline {
                    points: vec![
                        (x0 + icon * 0.9, y0),
                        (x0 + icon * 0.9, y0 + icon * 0.1),
                        (x0 + icon, y0 + icon * 0.1),
                    ],
                    closed: false,
                    fill: None,
                    stroke: Some((item.color, unit.max(1.0))),
                },
            ]
        }
        Tool::Highlight => vec![Outline {
            points: rectangle(x0, y0, x1, y1),
            closed: true,
            fill: Some((item.color, HIGHLIGHT_ALPHA)),
            stroke: None,
        }],
        Tool::Underline => vec![Outline {
            points: vec![(x0, y1), (x1, y1)],
            closed: false,
            fill: None,
            stroke: Some((item.color, 2.0 * unit)),
        }],
        Tool::StrikeThrough => vec![Outline {
            points: vec![(x0, (y0 + y1) / 2.0), (x1, (y0 + y1) / 2.0)],
            closed: false,
            fill: None,
            stroke: Some((item.color, 2.0 * unit)),
        }],
        Tool::Text | Tool::Select => Vec::new(),
    }
}

/// An SVG path `d` string for `outline`.
pub fn path_data(outline: &Outline) -> String {
    let mut data = String::new();
    for (index, (x, y)) in outline.points.iter().enumerate() {
        data.push_str(&format!(
            "{}{:.2} {:.2} ",
            if index == 0 { "M" } else { "L" },
            x,
            y
        ));
    }
    if outline.closed {
        data.push('Z');
    }
    data
}

fn hex(colour: u32) -> String {
    format!("#{:06x}", colour & 0x00FF_FFFF)
}

/// Escapes text for an SVG text node.
fn xml_escape(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// One SVG element per outline, plus the text of a Text or Note item, for a
/// canvas of `size`: what [`crate::vector::render_overlay`] bakes into an
/// image.
pub fn svg_elements(item: &Annotation, size: (f32, f32), unit: f32) -> String {
    let mut out = String::new();
    for outline in outlines(item, size, unit, false) {
        let fill = outline
            .fill
            .map_or("none".to_owned(), |(colour, _)| hex(colour));
        let fill_opacity = outline.fill.map_or(1.0, |(_, alpha)| alpha);
        let stroke = outline
            .stroke
            .map_or("none".to_owned(), |(colour, _)| hex(colour));
        let stroke_width = outline.stroke.map_or(0.0, |(_, width)| width);
        out.push_str(&format!(
            r#"<path d="{}" fill="{fill}" fill-opacity="{fill_opacity}" stroke="{stroke}" stroke-width="{stroke_width:.2}" stroke-linejoin="round" stroke-linecap="round"/>"#,
            path_data(&outline)
        ));
    }
    if item.tool == Tool::Text && !item.text.trim().is_empty() {
        let size_px = item.font_size * unit;
        let r = item.rect;
        // The text box's first baseline sits a font size below its top.
        let baseline = r.y0 * size.1 + size_px;
        out.push_str(&format!(
            r#"<text x="{:.2}" y="{:.2}" font-family="Inter, Helvetica, Arial, sans-serif" font-size="{size_px:.2}" fill="{}">{}</text>"#,
            r.x0 * size.0 + 2.0 * unit,
            baseline,
            hex(item.color),
            xml_escape(&item.text)
        ));
    }
    out
}

/// A complete SVG document of `size` holding every item in `items`.
pub fn svg_document<'a>(
    items: impl IntoIterator<Item = &'a Annotation>,
    size: (u32, u32),
    unit: f32,
) -> String {
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">"#,
        size.0, size.1, size.0, size.1
    );
    for item in items {
        svg.push_str(&svg_elements(item, (size.0 as f32, size.1 as f32), unit));
    }
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::UnitRect;

    fn item(tool: Tool, rect: (f32, f32, f32, f32)) -> Annotation {
        Annotation::new(
            0,
            tool,
            UnitRect {
                x0: rect.0,
                y0: rect.1,
                x1: rect.2,
                y1: rect.3,
            },
            0xff0000,
            2.0,
        )
    }

    #[test]
    fn every_drawn_tool_has_an_outline_and_text_and_select_have_none() {
        for tool in [
            Tool::Rectangle,
            Tool::Oval,
            Tool::Star,
            Tool::SpeechBubble,
            Tool::Mask,
            Tool::Note,
            Tool::Highlight,
            Tool::Underline,
            Tool::StrikeThrough,
        ] {
            let found = outlines(
                &item(tool, (0.1, 0.1, 0.5, 0.4)),
                (200.0, 100.0),
                1.0,
                false,
            );
            assert!(!found.is_empty(), "{tool:?}");
        }
        assert!(outlines(
            &item(Tool::Text, (0.1, 0.1, 0.5, 0.4)),
            (200.0, 100.0),
            1.0,
            false
        )
        .is_empty());
        assert!(outlines(
            &item(Tool::Select, (0.1, 0.1, 0.5, 0.4)),
            (200.0, 100.0),
            1.0,
            false
        )
        .is_empty());
    }

    #[test]
    fn a_rectangle_covers_its_rect_on_the_canvas_and_fills_when_asked() {
        let mut shape = item(Tool::Rectangle, (0.25, 0.5, 0.75, 1.0));
        let plain = outlines(&shape, (200.0, 100.0), 1.0, false);
        assert_eq!(
            plain[0].points,
            vec![(50.0, 50.0), (150.0, 50.0), (150.0, 100.0), (50.0, 100.0)]
        );
        assert_eq!(plain[0].fill, None);
        shape.fill = Some(0x00ff00);
        let filled = outlines(&shape, (200.0, 100.0), 1.0, false);
        assert_eq!(filled[0].fill, Some((0x00ff00, 1.0)));
        assert_eq!(filled[0].stroke, Some((0xff0000, 2.0)));
    }

    #[test]
    fn a_mask_is_a_solid_cover_and_has_no_outline_stroke() {
        let mask = outlines(
            &item(Tool::Mask, (0.0, 0.0, 1.0, 1.0)),
            (10.0, 10.0),
            1.0,
            false,
        );
        assert_eq!(mask[0].fill, Some((0xff0000, 1.0)));
        assert_eq!(mask[0].stroke, None);
    }

    #[test]
    fn an_arrow_has_a_shaft_and_a_head_at_its_end() {
        let mut arrow = item(Tool::Arrow, (0.0, 0.0, 1.0, 1.0));
        arrow.path = vec![(0.0, 0.5), (1.0, 0.5)];
        let found = outlines(&arrow, (100.0, 100.0), 1.0, false);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].points, vec![(0.0, 50.0), (100.0, 50.0)]);
        assert_eq!(found[1].points[1], (100.0, 50.0));
        assert!(found[1].points[0].0 < 100.0 && found[1].points[2].0 < 100.0);
    }

    #[test]
    fn a_path_still_being_drawn_is_in_canvas_fractions() {
        let mut line = item(Tool::Line, (0.0, 0.0, 0.5, 0.5));
        line.path = vec![(0.1, 0.2), (0.3, 0.4)];
        let drawing = outlines(&line, (100.0, 100.0), 1.0, true);
        assert_eq!(drawing[0].points, vec![(10.0, 20.0), (30.0, 40.0)]);
        let done = outlines(&line, (100.0, 100.0), 1.0, false);
        assert_eq!(done[0].points, vec![(5.0, 10.0), (15.0, 20.0)]);
    }

    #[test]
    fn svg_output_is_well_formed_and_escapes_text() {
        let mut text = item(Tool::Text, (0.1, 0.1, 0.9, 0.3));
        text.text = "a < b & c".into();
        let svg = svg_document(
            [&text, &item(Tool::Oval, (0.1, 0.5, 0.5, 0.9))],
            (100, 100),
            1.0,
        );
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert!(svg.contains("a &lt; b &amp; c"));
        assert!(svg.contains("<path"));
    }
}
