//! Editable page-relative annotations and PDF persistence.
//! Coordinates use the unrotated page, top-left origin, in the unit interval.

use std::path::Path;

use lopdf::{dictionary, Dictionary, Document, Object, Stream};

use crate::layout::UnitRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Highlight,
    Text,
    Rectangle,
    Oval,
    Line,
    Arrow,
    Sketch,
    Signature,
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
        Self { page, tool, rect: normal(rect), color, width, text: String::new(), path: Vec::new() }
    }

    pub fn contains(&self, point: (f32, f32)) -> bool {
        let r = self.rect;
        point.0 >= r.x0 && point.0 <= r.x1 && point.1 >= r.y0 && point.1 <= r.y1
    }

    pub fn translate(&mut self, delta: (f32, f32)) {
        let r = self.rect;
        let dx = delta.0.clamp(-r.x0, 1.0 - r.x1);
        let dy = delta.1.clamp(-r.y0, 1.0 - r.y1);
        self.rect = UnitRect { x0: r.x0 + dx, x1: r.x1 + dx, y0: r.y0 + dy, y1: r.y1 + dy };
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
}

impl Markup {
    pub fn checkpoint(&mut self) {
        self.undo.push(self.items.clone());
        self.redo.clear();
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.items, previous));
            self.dirty = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.items, next));
            self.dirty = true;
        }
    }

    pub fn revert(&mut self) {
        self.items.clear();
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
    }
}

fn number(value: f32) -> Object { Object::Real(value) }

fn rect_array(r: UnitRect, size: (f32, f32)) -> Vec<Object> {
    vec![number(r.x0 * size.0), number((1.0 - r.y1) * size.1), number(r.x1 * size.0), number((1.0 - r.y0) * size.1)]
}

fn rgb(color: u32) -> Vec<Object> {
    [16, 8, 0].into_iter().map(|shift| number(((color >> shift) & 255) as f32 / 255.0)).collect()
}

fn escape_pdf_text(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).take(2000)
        .flat_map(|c| if matches!(c, '(' | ')' | '\\') { vec!['\\', c] } else { vec![c] }).collect()
}

fn appearance(annotation: &Annotation, size: (f32, f32)) -> Vec<u8> {
    let r = annotation.rect;
    let w = ((r.x1 - r.x0) * size.0).max(1.0);
    let h = ((r.y1 - r.y0) * size.1).max(1.0);
    let c = rgb(annotation.color);
    let f = |index: usize| c[index].as_float().unwrap_or(0.0);
    let mut s = format!("q {} {} {} RG {} w ", f(0), f(1), f(2), annotation.width.max(0.5));
    match annotation.tool {
        Tool::Rectangle => s.push_str(&format!("1 1 {} {} re S ", w - 2.0, h - 2.0)),
        Tool::Oval => {
            let (x, y, k) = (w / 2.0, h / 2.0, 0.552_284_8);
            s.push_str(&format!("{} 0 m {} 0 {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} 0 {} c {} {} {} {} {} 0 c S ", x, x + x*k, y - y*k, w, y, w, y + y*k, x + x*k, h, x, h, x - x*k, h, 0, y + y*k, 0, y, 0, y - y*k, x - x*k, 0, x, 0));
        }
        Tool::Line | Tool::Arrow => {
            s.push_str(&format!("0 {} m {} 0 l S ", h, w));
            if annotation.tool == Tool::Arrow {
                s.push_str(&format!("{} {} m {} 0 l {} {} l S ", (w - 12.0).max(0.0), 1.0, w, w - 1.0, 12.0_f32.min(h)));
            }
        }
        Tool::Sketch | Tool::Signature => {
            if let Some(&(x, y)) = annotation.path.first() {
                s.push_str(&format!("{} {} m ", x*w, (1.0-y)*h));
                for &(x, y) in annotation.path.iter().skip(1) { s.push_str(&format!("{} {} l ", x*w, (1.0-y)*h)); }
                s.push_str("S ");
            }
        }
        Tool::Text => s.push_str(&format!("BT /F1 16 Tf {} {} {} rg 2 {} Td ({}) Tj ET ", f(0), f(1), f(2), (h - 18.0).max(1.0), escape_pdf_text(&annotation.text))),
        Tool::Highlight | Tool::Select => {}
    }
    s.push_str("Q");
    s.into_bytes()
}

/// Writes real PDF annotations, preserving page contents and pre-existing annotations.
/// Caller performs file I/O on a blocking worker and atomically replaces the source.
pub fn write_pdf(source: &Path, destination: &Path, items: &[Annotation]) -> Result<(), String> {
    let mut doc = Document::load(source).map_err(|e| e.to_string())?;
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    for item in items {
        if item.tool == Tool::Select { continue; }
        let page_id = *pages.get(item.page).ok_or("annotation page is out of range")?;
        let page = doc.get_dictionary(page_id).map_err(|e| e.to_string())?;
        let media = page.get(b"MediaBox").map_err(|e| e.to_string())?.as_array().map_err(|e| e.to_string())?;
        let coord = |i: usize| media.get(i).and_then(|v| v.as_float().ok()).unwrap_or(0.0);
        let size = ((coord(2)-coord(0)).max(1.0), (coord(3)-coord(1)).max(1.0));
        let mut annotation = dictionary! {
            "Type" => "Annot",
            "Subtype" => match item.tool { Tool::Highlight => "Highlight", Tool::Text => "FreeText", Tool::Oval => "Circle", Tool::Line | Tool::Arrow => "Line", Tool::Sketch | Tool::Signature => "Ink", _ => "Square" },
            "Rect" => Object::Array(rect_array(item.rect, size)),
            "C" => Object::Array(rgb(item.color)),
            "F" => 4,
            "NM" => format!("Lulo-{}-{}", item.page, doc.max_id + 1),
        };
        annotation.set("BS", dictionary! { "W" => number(item.width.max(0.5)) });
        if item.tool == Tool::Highlight {
            let q = rect_array(item.rect, size);
            annotation.set("QuadPoints", vec![q[0].clone(), q[3].clone(), q[2].clone(), q[3].clone(), q[0].clone(), q[1].clone(), q[2].clone(), q[1].clone()]);
            annotation.set("CA", number(0.4));
        } else {
            let r = item.rect;
            let w = ((r.x1-r.x0)*size.0).max(1.0);
            let h = ((r.y1-r.y0)*size.1).max(1.0);
            let mut stream = Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), number(w), number(h)] }, appearance(item, size));
            if item.tool == Tool::Text {
                let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
                stream.dict.set("Resources", dictionary! { "Font" => dictionary! { "F1" => font } });
                annotation.set("Contents", item.text.clone());
            }
            let ap = doc.add_object(stream);
            annotation.set("AP", dictionary! { "N" => ap });
            if matches!(item.tool, Tool::Sketch | Tool::Signature) {
                let points: Vec<Object> = item.path.iter().flat_map(|&(x,y)| [number((r.x0 + x*(r.x1-r.x0))*size.0), number((1.0-r.y0-y*(r.y1-r.y0))*size.1)]).collect();
                annotation.set("InkList", vec![Object::Array(points)]);
            }
            if matches!(item.tool, Tool::Line | Tool::Arrow) {
                annotation.set("L", vec![number(r.x0*size.0), number((1.0-r.y0)*size.1), number(r.x1*size.0), number((1.0-r.y1)*size.1)]);
                if item.tool == Tool::Arrow { annotation.set("LE", vec![Object::Name(b"None".to_vec()), Object::Name(b"OpenArrow".to_vec())]); }
            }
        }
        let id = doc.add_object(annotation);
        let existing = doc.get_dictionary(page_id).ok().and_then(|p| p.get(b"Annots").ok()).cloned();
        let mut annots = match existing { Some(Object::Array(a)) => a, Some(Object::Reference(r)) => doc.get_object(r).ok().and_then(|o| o.as_array().ok()).cloned().unwrap_or_default(), _ => Vec::new() };
        annots.push(Object::Reference(id));
        doc.get_dictionary_mut(page_id).map_err(|e| e.to_string())?.set("Annots", annots);
    }
    doc.save(destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_move_resize_and_revert() {
        let mut m = Markup::default();
        m.checkpoint();
        m.items.push(Annotation::new(0, Tool::Rectangle, UnitRect {x0:0.1,y0:0.2,x1:0.4,y1:0.5}, 0xff0000, 2.0));
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
        doc.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", root);
        doc.save(&source).unwrap();
        let items = [Annotation::new(0, Tool::Rectangle, UnitRect{x0:0.1,y0:0.2,x1:0.4,y1:0.5}, 0xff0000, 2.0), Annotation::new(0, Tool::Highlight, UnitRect{x0:0.2,y0:0.1,x1:0.6,y1:0.14}, 0xffff00, 1.0)];
        write_pdf(&source, &saved, &items).unwrap();
        let after = Document::load(&saved).unwrap();
        assert_eq!(after.get_page_annotations(page).unwrap().len(), 2);
        assert_eq!(after.get_page_content(page).unwrap(), b"q Q");
        let rectangle = after.get_page_annotations(page).unwrap()[0];
        let bounds = rectangle.get(b"Rect").unwrap().as_array().unwrap();
        assert!((bounds[0].as_float().unwrap() - 61.2).abs() < 0.01);
        assert!((bounds[1].as_float().unwrap() - 396.0).abs() < 0.01);
        std::fs::remove_dir_all(base).unwrap();
    }
}
