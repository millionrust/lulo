//! Editable page-relative annotations and PDF persistence.
//! Coordinates use the unrotated page, top-left origin, in the unit interval.

use std::path::Path;

use lopdf::{dictionary, Document, Object, Stream};

use crate::layout::UnitRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Highlight,
    Underline,
    StrikeThrough,
    Text,
    Rectangle,
    Oval,
    Line,
    Arrow,
    Sketch,
    Signature,
    /// A closed, multi-point shape (Tools ▸ Annotate ▸ Polygon): `path`
    /// holds its vertices, like `Sketch`, but the outline closes back to
    /// the first point instead of staying open.
    Polygon,
    /// A 5-point star inscribed in `rect` (Tools ▸ Annotate ▸ Star).
    Star,
    /// A rectangle with a small tail (Tools ▸ Annotate ▸ Speech Bubble).
    SpeechBubble,
    /// An opaque, filled cover over `rect` (Tools ▸ Annotate ▸ Mask):
    /// unlike the other shapes this fills rather than strokes, so the
    /// content underneath is actually hidden, not just outlined.
    Mask,
    /// A small sticky-note icon placed at `rect`'s top-left corner
    /// (Tools ▸ Annotate ▸ Note); `text` holds the note's body.
    Note,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub page: usize,
    pub tool: Tool,
    pub rect: UnitRect,
    pub color: u32,
    pub width: f32,
    pub text: String,
    /// Points in the annotation rectangle, each in 0..=1.
    pub path: Vec<(f32, f32)>,
}

impl Annotation {
    pub fn new(page: usize, tool: Tool, rect: UnitRect, color: u32, width: f32) -> Self {
        Self {
            page,
            tool,
            rect: normal(rect),
            color,
            width,
            text: String::new(),
            path: Vec::new(),
        }
    }

    pub fn contains(&self, point: (f32, f32)) -> bool {
        let r = self.rect;
        point.0 >= r.x0 && point.0 <= r.x1 && point.1 >= r.y0 && point.1 <= r.y1
    }

    pub fn translate(&mut self, delta: (f32, f32)) {
        let r = self.rect;
        let dx = delta.0.clamp(-r.x0, 1.0 - r.x1);
        let dy = delta.1.clamp(-r.y0, 1.0 - r.y1);
        self.rect = UnitRect {
            x0: r.x0 + dx,
            x1: r.x1 + dx,
            y0: r.y0 + dy,
            y1: r.y1 + dy,
        };
    }

    pub fn resize(&mut self, point: (f32, f32)) {
        self.rect.x1 = point.0.clamp(self.rect.x0 + 0.005, 1.0);
        self.rect.y1 = point.1.clamp(self.rect.y0 + 0.005, 1.0);
    }
}

pub fn normal(rect: UnitRect) -> UnitRect {
    UnitRect {
        x0: rect.x0.min(rect.x1).clamp(0.0, 1.0),
        y0: rect.y0.min(rect.y1).clamp(0.0, 1.0),
        x1: rect.x0.max(rect.x1).clamp(0.0, 1.0),
        y1: rect.y0.max(rect.y1).clamp(0.0, 1.0),
    }
}

#[derive(Default, Clone)]
pub struct Markup {
    pub items: Vec<Annotation>,
    undo: Vec<Vec<Annotation>>,
    redo: Vec<Vec<Annotation>>,
    pub dirty: bool,
    pub revision: u64,
}

impl Markup {
    pub fn checkpoint(&mut self) {
        self.undo.push(self.items.clone());
        self.redo.clear();
        self.dirty = true;
        self.revision += 1;
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.items, previous));
            self.dirty = true;
            self.revision += 1;
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.items, next));
            self.dirty = true;
            self.revision += 1;
        }
    }

    pub fn revert(&mut self) {
        self.items.clear();
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        self.revision += 1;
    }
}

fn number(value: f32) -> Object {
    Object::Real(value)
}

fn rect_array(r: UnitRect, size: (f32, f32)) -> Vec<Object> {
    vec![
        number(r.x0 * size.0),
        number((1.0 - r.y1) * size.1),
        number(r.x1 * size.0),
        number((1.0 - r.y0) * size.1),
    ]
}

fn rgb(color: u32) -> Vec<Object> {
    [16, 8, 0]
        .into_iter()
        .map(|shift| number(((color >> shift) & 255) as f32 / 255.0))
        .collect()
}

fn escape_pdf_text(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .take(2000)
        .flat_map(|c| {
            if matches!(c, '(' | ')' | '\\') {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

fn appearance(annotation: &Annotation, size: (f32, f32)) -> Vec<u8> {
    let r = annotation.rect;
    let w = ((r.x1 - r.x0) * size.0).max(1.0);
    let h = ((r.y1 - r.y0) * size.1).max(1.0);
    let c = rgb(annotation.color);
    let f = |index: usize| c[index].as_float().unwrap_or(0.0);
    let mut s = format!(
        "q {} {} {} RG {} w ",
        f(0),
        f(1),
        f(2),
        annotation.width.max(0.5)
    );
    match annotation.tool {
        Tool::Rectangle => s.push_str(&format!("1 1 {} {} re S ", w - 2.0, h - 2.0)),
        Tool::Oval => {
            for step in 0..=32 {
                let angle = step as f32 * std::f32::consts::TAU / 32.0;
                let x = w * (1.0 + angle.cos()) / 2.0;
                let y = h * (1.0 + angle.sin()) / 2.0;
                s.push_str(&format!("{x} {y} {} ", if step == 0 { "m" } else { "l" }));
            }
            s.push_str("S ");
        }
        Tool::Line | Tool::Arrow => {
            let start = annotation.path.first().copied().unwrap_or((0.0, 0.0));
            let end = annotation.path.get(1).copied().unwrap_or((1.0, 1.0));
            let (x0, y0) = (start.0 * w, (1.0 - start.1) * h);
            let (x1, y1) = (end.0 * w, (1.0 - end.1) * h);
            s.push_str(&format!("{x0} {y0} m {x1} {y1} l S "));
            if annotation.tool == Tool::Arrow {
                let (dx, dy) = (x1 - x0, y1 - y0);
                let length = dx.hypot(dy).max(1.0);
                let (ux, uy) = (dx / length, dy / length);
                s.push_str(&format!(
                    "{} {} m {x1} {y1} l {} {} l S ",
                    x1 - 12.0 * ux - 5.0 * uy,
                    y1 - 12.0 * uy + 5.0 * ux,
                    x1 - 12.0 * ux + 5.0 * uy,
                    y1 - 12.0 * uy - 5.0 * ux
                ));
            }
        }
        Tool::Sketch | Tool::Signature => {
            if let Some(&(x, y)) = annotation.path.first() {
                s.push_str(&format!("{} {} m ", x * w, (1.0 - y) * h));
                for &(x, y) in annotation.path.iter().skip(1) {
                    s.push_str(&format!("{} {} l ", x * w, (1.0 - y) * h));
                }
                s.push_str("S ");
            }
        }
        Tool::Polygon => {
            if annotation.path.len() >= 2 {
                for (index, &(x, y)) in annotation.path.iter().enumerate() {
                    let (px, py) = (x * w, (1.0 - y) * h);
                    s.push_str(&format!(
                        "{px} {py} {} ",
                        if index == 0 { "m" } else { "l" }
                    ));
                }
                // Close the outline back to the first vertex.
                if let Some(&(x, y)) = annotation.path.first() {
                    s.push_str(&format!("{} {} l ", x * w, (1.0 - y) * h));
                }
                s.push_str("S ");
            }
        }
        Tool::Star => {
            // 10 vertices alternating the outer and inner radius of a
            // standard 5-point star (inner = outer * 0.382), starting at
            // the top and going clockwise, closed back to the first point.
            let (cx, cy) = (w / 2.0, h / 2.0);
            let outer = w.min(h) / 2.0;
            let inner = outer * 0.382;
            for step in 0..=10 {
                let index = step % 10;
                let radius = if index % 2 == 0 { outer } else { inner };
                let angle = std::f32::consts::FRAC_PI_2 - index as f32 * std::f32::consts::PI / 5.0;
                let x = cx + radius * angle.cos();
                let y = cy + radius * angle.sin();
                s.push_str(&format!("{x} {y} {} ", if step == 0 { "m" } else { "l" }));
            }
            s.push_str("S ");
        }
        Tool::SpeechBubble => {
            s.push_str(&format!("1 1 {} {} re S ", w - 2.0, h - 2.0));
            // A small tail near the bottom-left, kept inside the box's own
            // 0‥w/0‥h bounds so the Form XObject's BBox does not clip it.
            s.push_str(&format!(
                "{} {} m {} {} l {} {} l S ",
                w * 0.25,
                h * 0.15,
                w * 0.08,
                h * 0.02,
                w * 0.35,
                h * 0.15,
            ));
        }
        Tool::Mask => {
            // Filled, not stroked: a Mask actually covers the content
            // underneath instead of just outlining it.
            s.push_str(&format!("{} {} {} rg 0 0 {w} {h} re f ", f(0), f(1), f(2)));
        }
        Tool::Note => {
            // A small fixed-size sticky-note icon at the top-left corner,
            // regardless of how big `rect` is, plus a folded-corner mark.
            let icon = 20.0_f32.min(w).min(h);
            s.push_str(&format!(
                "{} {} {} rg 0 0 {icon} {icon} re f ",
                f(0),
                f(1),
                f(2)
            ));
            s.push_str(&format!(
                "{} {icon} m {icon} {icon} l {icon} {} l S ",
                icon * 0.9,
                icon * 0.9,
            ));
        }
        Tool::Text => s.push_str(&format!(
            "BT /F1 16 Tf {} {} {} rg 2 {} Td ({}) Tj ET ",
            f(0),
            f(1),
            f(2),
            (h - 18.0).max(1.0),
            escape_pdf_text(&annotation.text)
        )),
        Tool::Highlight | Tool::Underline | Tool::StrikeThrough | Tool::Select => {}
    }
    s.push('Q');
    s.into_bytes()
}

/// Writes real PDF annotations, preserving page contents and pre-existing annotations.
/// Caller performs file I/O on a blocking worker and atomically replaces the source.
pub fn write_pdf(source: &Path, destination: &Path, items: &[Annotation]) -> Result<(), String> {
    let mut doc = Document::load(source).map_err(|e| e.to_string())?;
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    for item in items {
        if item.tool == Tool::Select {
            continue;
        }
        let page_id = *pages
            .get(item.page)
            .ok_or("annotation page is out of range")?;
        let mut ancestor = page_id;
        let mut media = None;
        for _ in 0..32 {
            let page = doc.get_dictionary(ancestor).map_err(|e| e.to_string())?;
            if let Ok(value) = page.get(b"MediaBox") {
                media = Some(value.as_array().map_err(|e| e.to_string())?);
                break;
            }
            ancestor = page
                .get(b"Parent")
                .map_err(|e| e.to_string())?
                .as_reference()
                .map_err(|e| e.to_string())?;
        }
        let media = media.ok_or("PDF page has no MediaBox")?;
        let coord = |i: usize| media.get(i).and_then(|v| v.as_float().ok()).unwrap_or(0.0);
        let size = (
            (coord(2) - coord(0)).max(1.0),
            (coord(3) - coord(1)).max(1.0),
        );
        let mut annotation = dictionary! {
            "Type" => "Annot",
            "Subtype" => match item.tool { Tool::Highlight => "Highlight", Tool::Underline => "Underline", Tool::StrikeThrough => "StrikeOut", Tool::Text => "FreeText", Tool::Oval => "Circle", Tool::Line | Tool::Arrow => "Line", Tool::Sketch | Tool::Signature => "Ink", Tool::Polygon => "Polygon", Tool::Note => "Text", _ => "Square" },
            "Rect" => Object::Array(rect_array(item.rect, size)),
            "C" => Object::Array(rgb(item.color)),
            "F" => 4,
            "NM" => format!("Lulo-{}-{}", item.page, doc.max_id + 1),
        };
        annotation.set("BS", dictionary! { "W" => number(item.width.max(0.5)) });
        if matches!(
            item.tool,
            Tool::Highlight | Tool::Underline | Tool::StrikeThrough
        ) {
            let q = rect_array(item.rect, size);
            annotation.set(
                "QuadPoints",
                vec![
                    q[0].clone(),
                    q[3].clone(),
                    q[2].clone(),
                    q[3].clone(),
                    q[0].clone(),
                    q[1].clone(),
                    q[2].clone(),
                    q[1].clone(),
                ],
            );
            if item.tool == Tool::Highlight {
                annotation.set("CA", number(0.4));
            }
        } else {
            let r = item.rect;
            let w = ((r.x1 - r.x0) * size.0).max(1.0);
            let h = ((r.y1 - r.y0) * size.1).max(1.0);
            let mut stream = Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), number(w), number(h)] },
                appearance(item, size),
            );
            if item.tool == Tool::Text {
                let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
                stream.dict.set(
                    "Resources",
                    dictionary! { "Font" => dictionary! { "F1" => font } },
                );
                annotation.set("Contents", item.text.clone());
                annotation.set("DA", "/F1 16 Tf 0 0 0 rg");
            }
            let ap = doc.add_object(stream);
            annotation.set("AP", dictionary! { "N" => ap });
            if matches!(item.tool, Tool::Sketch | Tool::Signature) {
                let points: Vec<Object> = item
                    .path
                    .iter()
                    .flat_map(|&(x, y)| {
                        [
                            number((r.x0 + x * (r.x1 - r.x0)) * size.0),
                            number((1.0 - r.y0 - y * (r.y1 - r.y0)) * size.1),
                        ]
                    })
                    .collect();
                annotation.set("InkList", vec![Object::Array(points)]);
            }
            if matches!(item.tool, Tool::Line | Tool::Arrow) {
                let start = item.path.first().copied().unwrap_or((0.0, 0.0));
                let end = item.path.get(1).copied().unwrap_or((1.0, 1.0));
                let pdf_point = |p: (f32, f32)| {
                    [
                        number((r.x0 + p.0 * (r.x1 - r.x0)) * size.0),
                        number((1.0 - r.y0 - p.1 * (r.y1 - r.y0)) * size.1),
                    ]
                };
                let mut line = pdf_point(start).to_vec();
                line.extend(pdf_point(end));
                annotation.set("L", line);
                if item.tool == Tool::Arrow {
                    annotation.set(
                        "LE",
                        vec![
                            Object::Name(b"None".to_vec()),
                            Object::Name(b"OpenArrow".to_vec()),
                        ],
                    );
                }
            }
            if item.tool == Tool::Polygon {
                let vertices: Vec<Object> = item
                    .path
                    .iter()
                    .flat_map(|&(x, y)| {
                        [
                            number((r.x0 + x * (r.x1 - r.x0)) * size.0),
                            number((1.0 - r.y0 - y * (r.y1 - r.y0)) * size.1),
                        ]
                    })
                    .collect();
                annotation.set("Vertices", vertices);
            }
            if item.tool == Tool::Note {
                annotation.set("Contents", item.text.clone());
            }
        }
        let id = doc.add_object(annotation);
        let existing = doc
            .get_dictionary(page_id)
            .ok()
            .and_then(|p| p.get(b"Annots").ok())
            .cloned();
        let mut annots = match existing {
            Some(Object::Array(a)) => a,
            Some(Object::Reference(r)) => doc
                .get_object(r)
                .ok()
                .and_then(|o| o.as_array().ok())
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        annots.push(Object::Reference(id));
        doc.get_dictionary_mut(page_id)
            .map_err(|e| e.to_string())?
            .set("Annots", annots);
    }
    doc.save(destination).map_err(|e| e.to_string())?;
    Ok(())
}

/// One row of View ▸ Table of Contents: a PDF outline (bookmark) entry,
/// flattened from its tree with `depth` recording how nested it was.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineEntry {
    pub depth: usize,
    pub title: String,
    pub page: usize,
}

/// Reads a PDF's `/Outlines` tree (its Table of Contents) into a flat,
/// depth-tagged list in document order. Returns an empty list for a PDF
/// with no outline, a malformed one, or one that cannot be opened — the
/// sidebar then shows "No Table of Contents" rather than failing to show
/// the document itself. Caller runs this on a blocking worker.
pub fn read_outline(path: &Path) -> Vec<OutlineEntry> {
    let Ok(doc) = Document::load(path) else {
        return Vec::new();
    };
    let page_index: std::collections::HashMap<(u32, u16), usize> = doc
        .get_pages()
        .into_values()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();
    let Some(first) = outline_root(&doc) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut visited = std::collections::HashSet::new();
    walk_outline(&doc, first, 0, &page_index, &mut visited, &mut out);
    out
}

fn outline_root(doc: &Document) -> Option<(u32, u16)> {
    let root = doc.trailer.get(b"Root").ok()?.as_reference().ok()?;
    let catalog = doc.get_dictionary(root).ok()?;
    let outlines_id = catalog.get(b"Outlines").ok()?.as_reference().ok()?;
    let outlines = doc.get_dictionary(outlines_id).ok()?;
    outlines.get(b"First").ok()?.as_reference().ok()
}

/// The first page a `/Dest` or `/A` (GoTo action) destination resolves to,
/// for the common "array starting with a direct page reference" shape.
/// Named destinations (looked up through the catalog's `/Names` tree) are
/// not resolved; such an entry simply has no page (falls back to 0 by the
/// caller), which is a real but minor gap rather than a crash or a stub.
fn resolve_outline_page(
    doc: &Document,
    dict: &lopdf::Dictionary,
    page_index: &std::collections::HashMap<(u32, u16), usize>,
) -> Option<usize> {
    fn first_reference(object: &Object) -> Option<(u32, u16)> {
        match object {
            Object::Reference(r) => Some(*r),
            Object::Array(items) => items.first().and_then(|first| first.as_reference().ok()),
            _ => None,
        }
    }
    if let Ok(dest) = dict.get(b"Dest") {
        if let Some(page) = first_reference(dest).and_then(|id| page_index.get(&id).copied()) {
            return Some(page);
        }
    }
    if let Ok(action) = dict.get(b"A") {
        let action_dict = match action {
            Object::Reference(r) => doc.get_dictionary(*r).ok(),
            Object::Dictionary(inline) => Some(inline),
            _ => None,
        };
        if let Some(dest) = action_dict.and_then(|action| action.get(b"D").ok()) {
            if let Some(page) = first_reference(dest).and_then(|id| page_index.get(&id).copied()) {
                return Some(page);
            }
        }
    }
    None
}

fn decode_outline_title(object: &Object) -> Option<String> {
    let Object::String(bytes, _) = object else {
        return None;
    };
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        Some(String::from_utf16_lossy(&units))
    } else {
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Walks one outline level (siblings via `/Next`) and recurses into each
/// item's children (`/First`), bounded by a visited-set (a malformed PDF
/// could otherwise cycle) and a hard cap so a pathological file cannot
/// hang the blocking worker that calls this.
fn walk_outline(
    doc: &Document,
    first: (u32, u16),
    depth: usize,
    page_index: &std::collections::HashMap<(u32, u16), usize>,
    visited: &mut std::collections::HashSet<(u32, u16)>,
    out: &mut Vec<OutlineEntry>,
) {
    const MAX_ENTRIES: usize = 2000;
    let mut current = Some(first);
    while let Some(id) = current {
        if out.len() >= MAX_ENTRIES || !visited.insert(id) {
            break;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            break;
        };
        let title = dict
            .get(b"Title")
            .ok()
            .and_then(decode_outline_title)
            .unwrap_or_default();
        if !title.trim().is_empty() {
            let page = resolve_outline_page(doc, dict, page_index).unwrap_or(0);
            out.push(OutlineEntry { depth, title, page });
        }
        if let Ok(child) = dict.get(b"First").and_then(Object::as_reference) {
            walk_outline(doc, child, depth + 1, page_index, visited, out);
        }
        current = dict
            .get(b"Next")
            .ok()
            .and_then(|next| next.as_reference().ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_move_resize_and_revert() {
        let mut m = Markup::default();
        m.checkpoint();
        m.items.push(Annotation::new(
            0,
            Tool::Rectangle,
            UnitRect {
                x0: 0.1,
                y0: 0.2,
                x1: 0.4,
                y1: 0.5,
            },
            0xff0000,
            2.0,
        ));
        m.checkpoint();
        m.items[0].translate((0.8, 0.8));
        assert_eq!(m.items[0].rect.x1, 1.0);
        m.undo();
        assert_eq!(m.items[0].rect.x0, 0.1);
        m.redo();
        assert_eq!(m.items[0].rect.x1, 1.0);
        m.revert();
        assert!(m.items.is_empty());
    }

    #[test]
    fn pdf_round_trip_preserves_content_and_positions() {
        let base = std::env::temp_dir().join(format!("rmac-markup-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let source = base.join("source.pdf");
        let saved = base.join("saved.pdf");
        let mut doc = Document::with_version("1.4");
        let pages = doc.new_object_id();
        let content = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", root);
        doc.save(&source).unwrap();
        let items = [
            Annotation::new(
                0,
                Tool::Rectangle,
                UnitRect {
                    x0: 0.1,
                    y0: 0.2,
                    x1: 0.4,
                    y1: 0.5,
                },
                0xff0000,
                2.0,
            ),
            Annotation::new(
                0,
                Tool::Highlight,
                UnitRect {
                    x0: 0.2,
                    y0: 0.1,
                    x1: 0.6,
                    y1: 0.14,
                },
                0xffff00,
                1.0,
            ),
            Annotation::new(
                0,
                Tool::Underline,
                UnitRect {
                    x0: 0.2,
                    y0: 0.2,
                    x1: 0.6,
                    y1: 0.24,
                },
                0xff0000,
                1.0,
            ),
            Annotation::new(
                0,
                Tool::StrikeThrough,
                UnitRect {
                    x0: 0.2,
                    y0: 0.3,
                    x1: 0.6,
                    y1: 0.34,
                },
                0xff0000,
                1.0,
            ),
        ];
        write_pdf(&source, &saved, &items).unwrap();
        let original_content = Document::load(&source).unwrap().get_page_content(page);
        let after = Document::load(&saved).unwrap();
        let annotations = after.get_page_annotations(page).unwrap();
        assert_eq!(annotations.len(), 4);
        assert_eq!(
            annotations[2].get(b"Subtype").unwrap().as_name().unwrap(),
            b"Underline"
        );
        assert_eq!(
            annotations[3].get(b"Subtype").unwrap().as_name().unwrap(),
            b"StrikeOut"
        );
        assert_eq!(after.get_page_content(page), original_content);
        let rectangle = after.get_page_annotations(page).unwrap()[0];
        let bounds = rectangle.get(b"Rect").unwrap().as_array().unwrap();
        assert!((bounds[0].as_float().unwrap() - 61.2).abs() < 0.01);
        assert!((bounds[1].as_float().unwrap() - 396.0).abs() < 0.01);
        std::fs::remove_dir_all(base).unwrap();
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> UnitRect {
        UnitRect { x0, y0, x1, y1 }
    }

    #[test]
    fn polygon_round_trips_as_a_polygon_with_vertices() {
        let base =
            std::env::temp_dir().join(format!("rmac-markup-test-polygon-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let source = base.join("source.pdf");
        let saved = base.join("saved.pdf");
        let mut doc = Document::with_version("1.4");
        let pages = doc.new_object_id();
        let content = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", root);
        doc.save(&source).unwrap();
        let mut polygon =
            Annotation::new(0, Tool::Polygon, rect(0.1, 0.1, 0.5, 0.5), 0x00ff00, 1.5);
        polygon.path = vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)];
        write_pdf(&source, &saved, &[polygon]).unwrap();
        let after = Document::load(&saved).unwrap();
        let annotations = after.get_page_annotations(page).unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(
            annotations[0].get(b"Subtype").unwrap().as_name().unwrap(),
            b"Polygon"
        );
        let vertices = annotations[0].get(b"Vertices").unwrap().as_array().unwrap();
        // Three path points, each an (x, y) pair.
        assert_eq!(vertices.len(), 6);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn note_round_trips_as_a_text_annotation() {
        let base =
            std::env::temp_dir().join(format!("rmac-markup-test-note-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let source = base.join("source.pdf");
        let saved = base.join("saved.pdf");
        let mut doc = Document::with_version("1.4");
        let pages = doc.new_object_id();
        let content = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", root);
        doc.save(&source).unwrap();
        let mut note = Annotation::new(0, Tool::Note, rect(0.1, 0.1, 0.15, 0.15), 0xffff00, 1.0);
        note.text = "Remember this".to_owned();
        write_pdf(&source, &saved, &[note]).unwrap();
        let after = Document::load(&saved).unwrap();
        let annotations = after.get_page_annotations(page).unwrap();
        assert_eq!(
            annotations[0].get(b"Subtype").unwrap().as_name().unwrap(),
            b"Text"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn read_outline_flattens_titles_depth_and_destination_pages() {
        let base =
            std::env::temp_dir().join(format!("rmac-markup-test-outline-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("outline.pdf");
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let content = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page0 = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content });
        let page1 = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page0.into(), page1.into()], "Count" => 2 }),
        );
        // A child item under "Chapter 1", pointing at page1 (index 1).
        let child = doc.add_object(dictionary! {
            "Title" => Object::String(b"Section 1.1".to_vec(), lopdf::StringFormat::Literal),
            "Dest" => vec![Object::Reference(page1), "XYZ".into(), 0.into(), 792.into(), 0.into()],
        });
        let chapter = doc.add_object(dictionary! {
            "Title" => Object::String(b"Chapter 1".to_vec(), lopdf::StringFormat::Literal),
            "Dest" => vec![Object::Reference(page0), "XYZ".into(), 0.into(), 792.into(), 0.into()],
            "First" => child,
        });
        let outlines = doc.add_object(dictionary! { "Type" => "Outlines", "First" => chapter });
        let root = doc.add_object(
            dictionary! { "Type" => "Catalog", "Pages" => pages_id, "Outlines" => outlines },
        );
        doc.trailer.set("Root", root);
        doc.save(&path).unwrap();

        let entries = read_outline(&path);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Chapter 1");
        assert_eq!(entries[0].depth, 0);
        assert_eq!(entries[0].page, 0);
        assert_eq!(entries[1].title, "Section 1.1");
        assert_eq!(entries[1].depth, 1);
        assert_eq!(entries[1].page, 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn read_outline_is_empty_for_a_pdf_with_no_outline() {
        let base = std::env::temp_dir().join(format!(
            "rmac-markup-test-no-outline-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("plain.pdf");
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let content = doc.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Contents" => content });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", root);
        doc.save(&path).unwrap();

        assert!(read_outline(&path).is_empty());
        assert!(read_outline(&base.join("missing.pdf")).is_empty());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn star_speech_bubble_and_mask_each_draw_distinct_appearances() {
        let annotation_with =
            |tool| Annotation::new(0, tool, rect(0.1, 0.1, 0.6, 0.6), 0x3366ff, 2.0);
        let size = (400.0, 400.0);
        let rectangle = appearance(&annotation_with(Tool::Rectangle), size);
        let star = appearance(&annotation_with(Tool::Star), size);
        let speech_bubble = appearance(&annotation_with(Tool::SpeechBubble), size);
        let mask = appearance(&annotation_with(Tool::Mask), size);
        let note = appearance(&annotation_with(Tool::Note), size);
        for drawing in [&star, &speech_bubble, &mask, &note] {
            assert!(!drawing.is_empty());
            assert_ne!(drawing, &rectangle);
        }
        // Mask fills (rg/f); it does not just stroke an outline.
        let mask_text = String::from_utf8(mask).unwrap();
        assert!(mask_text.contains(" rg "));
        assert!(mask_text.contains(" f "));
    }
}
