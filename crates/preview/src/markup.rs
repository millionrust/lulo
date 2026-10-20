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
    /// Set for an annotation read from the PDF itself (PREV-03A): the PDF
    /// object it came from, so a Save can replace or delete exactly that
    /// object. `None` for one made in this session.
    pub origin: Option<(u32, u16)>,
    /// Interior colour of a closed shape (Rectangle, Oval, Polygon, Star,
    /// Speech Bubble), `None` for an unfilled outline.
    pub fill: Option<u32>,
    /// Point size of a Text box.
    pub font_size: f32,
}

/// Default size of a Text box's type.
pub const DEFAULT_FONT_SIZE: f32 = 16.0;

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
            origin: None,
            fill: None,
            font_size: DEFAULT_FONT_SIZE,
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

/// The vertices of a regular `sides`-gon inscribed in the annotation
/// rectangle (fractions of it), point up, for a new Polygon.
pub fn regular_polygon(sides: usize) -> Vec<(f32, f32)> {
    (0..sides.max(3))
        .map(|index| {
            let angle = -std::f32::consts::FRAC_PI_2
                + index as f32 * std::f32::consts::TAU / sides.max(3) as f32;
            (0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin())
        })
        .collect()
}

pub fn normal(rect: UnitRect) -> UnitRect {
    UnitRect {
        x0: rect.x0.min(rect.x1).clamp(0.0, 1.0),
        y0: rect.y0.min(rect.y1).clamp(0.0, 1.0),
        x1: rect.x0.max(rect.x1).clamp(0.0, 1.0),
        y1: rect.y0.max(rect.y1).clamp(0.0, 1.0),
    }
}

impl Tool {
    /// The name stored in an annotation's `/LuloTool`, so a shape Lulo
    /// wrote as a plain `Square` (Star, Speech Bubble, Mask, Signature)
    /// reads back as itself.
    fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Highlight => "Highlight",
            Tool::Underline => "Underline",
            Tool::StrikeThrough => "StrikeThrough",
            Tool::Text => "Text",
            Tool::Rectangle => "Rectangle",
            Tool::Oval => "Oval",
            Tool::Line => "Line",
            Tool::Arrow => "Arrow",
            Tool::Sketch => "Sketch",
            Tool::Signature => "Signature",
            Tool::Polygon => "Polygon",
            Tool::Star => "Star",
            Tool::SpeechBubble => "SpeechBubble",
            Tool::Mask => "Mask",
            Tool::Note => "Note",
        }
    }

    fn from_name(name: &[u8]) -> Option<Tool> {
        Some(match name {
            b"Highlight" => Tool::Highlight,
            b"Underline" => Tool::Underline,
            b"StrikeThrough" => Tool::StrikeThrough,
            b"Text" => Tool::Text,
            b"Rectangle" => Tool::Rectangle,
            b"Oval" => Tool::Oval,
            b"Line" => Tool::Line,
            b"Arrow" => Tool::Arrow,
            b"Sketch" => Tool::Sketch,
            b"Signature" => Tool::Signature,
            b"Polygon" => Tool::Polygon,
            b"Star" => Tool::Star,
            b"SpeechBubble" => Tool::SpeechBubble,
            b"Mask" => Tool::Mask,
            b"Note" => Tool::Note,
            _ => return None,
        })
    }
}

#[derive(Default, Clone)]
pub struct Markup {
    pub items: Vec<Annotation>,
    /// The annotations the PDF already had when it was opened (PREV-03A),
    /// as read; `items` starts as a copy and Revert returns to it.
    pub loaded: Vec<Annotation>,
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

    /// Adopts the annotations a PDF already carries as editable items
    /// without marking the document edited.
    pub fn adopt(&mut self, existing: Vec<Annotation>) {
        self.items = existing.clone();
        self.loaded = existing;
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        self.revision += 1;
    }

    pub fn revert(&mut self) {
        self.items = self.loaded.clone();
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

/// A PDF text string: plain ASCII as it is, anything else as UTF-16BE with
/// the byte-order mark, so other readers show the same characters.
fn pdf_text(text: &str) -> Object {
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(2000)
        .collect();
    if text.is_ascii() {
        Object::string_literal(text)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Object::String(bytes, lopdf::StringFormat::Hexadecimal)
    }
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
    let colour = |value: u32| -> (f32, f32, f32) {
        let c = rgb(value);
        let f = |index: usize| c[index].as_float().unwrap_or(0.0);
        (f(0), f(1), f(2))
    };
    if annotation.tool == Tool::Text {
        let (red, green, blue) = colour(annotation.color);
        let font = annotation.font_size.max(4.0);
        return format!(
            "q BT /F1 {font} Tf {red} {green} {blue} rg 2 {} Td ({}) Tj ET Q",
            (h - font - 2.0).max(1.0),
            escape_pdf_text(&annotation.text)
        )
        .into_bytes();
    }
    // Strokes are drawn inside the annotation's box so its edge does not
    // clip them; Mask, Note and the markers fill the whole box.
    let pad = match annotation.tool {
        Tool::Mask | Tool::Note | Tool::Highlight | Tool::Underline | Tool::StrikeThrough => 0.0,
        _ => (annotation.width.max(0.5) / 2.0 + 0.5)
            .min(w / 2.0)
            .min(h / 2.0),
    };
    let mut inner = annotation.clone();
    inner.rect = UnitRect {
        x0: r.x0 + pad / size.0,
        y0: r.y0 + pad / size.1,
        x1: r.x1 - pad / size.0,
        y1: r.y1 - pad / size.1,
    };
    let mut s = String::from("q 1 j 1 J ");
    for outline in crate::shapes::outlines(&inner, size, 1.0, false) {
        // Page space (y down) to the appearance's own box (y up).
        let place = |p: (f32, f32)| (p.0 - r.x0 * size.0, r.y1 * size.1 - p.1);
        if let Some((value, _)) = outline.fill {
            let (red, green, blue) = colour(value);
            s.push_str(&format!("{red} {green} {blue} rg "));
        }
        if let Some((value, width)) = outline.stroke {
            let (red, green, blue) = colour(value);
            s.push_str(&format!("{red} {green} {blue} RG {width} w "));
        }
        for (index, point) in outline.points.iter().enumerate() {
            let (x, y) = place(*point);
            s.push_str(&format!("{x} {y} {} ", if index == 0 { "m" } else { "l" }));
        }
        s.push_str(
            match (
                outline.fill.is_some(),
                outline.stroke.is_some(),
                outline.closed,
            ) {
                (true, true, true) => "b ",
                (true, true, false) => "B ",
                (true, false, _) => "f ",
                (false, true, true) => "s ",
                (false, true, false) => "S ",
                (false, false, _) => "n ",
            },
        );
    }
    s.push('Q');
    s.into_bytes()
}

/// Edit > Insert > Blank Page (PRV-MENU-007): writes `destination` as
/// `source` with one empty page, the same size as the page it follows,
/// spliced in directly after page index `after` (clamped to the last
/// page). Every ancestor `Pages` node's `/Count` is bumped, so nested page
/// trees stay valid. Caller does the file I/O on a blocking worker.
pub fn insert_blank_page(source: &Path, destination: &Path, after: usize) -> Result<(), String> {
    let mut doc = Document::load(source).map_err(|e| e.to_string())?;
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let anchor = *pages
        .get(after.min(pages.len().saturating_sub(1)))
        .ok_or("PDF has no pages")?;
    let mut ancestor = anchor;
    let mut media = None;
    for _ in 0..32 {
        let node = doc.get_dictionary(ancestor).map_err(|e| e.to_string())?;
        if let Ok(value) = node.get(b"MediaBox") {
            media = Some(value.clone());
            break;
        }
        ancestor = node
            .get(b"Parent")
            .map_err(|e| e.to_string())?
            .as_reference()
            .map_err(|e| e.to_string())?;
    }
    let media = media.ok_or("PDF page has no MediaBox")?;
    let parent = doc
        .get_dictionary(anchor)
        .map_err(|e| e.to_string())?
        .get(b"Parent")
        .map_err(|e| e.to_string())?
        .as_reference()
        .map_err(|e| e.to_string())?;
    let blank = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => parent,
        "MediaBox" => media,
        "Resources" => Object::Dictionary(lopdf::Dictionary::new()),
    });
    {
        let kids = doc
            .get_dictionary_mut(parent)
            .map_err(|e| e.to_string())?
            .get_mut(b"Kids")
            .map_err(|e| e.to_string())?
            .as_array_mut()
            .map_err(|e| e.to_string())?;
        let position = kids
            .iter()
            .position(|kid| kid.as_reference().ok() == Some(anchor))
            .ok_or("page is not listed in its parent")?;
        kids.insert(position + 1, Object::Reference(blank));
    }
    let mut node = Some(parent);
    for _ in 0..32 {
        let Some(id) = node else { break };
        let dictionary = doc.get_dictionary_mut(id).map_err(|e| e.to_string())?;
        let count = dictionary
            .get(b"Count")
            .ok()
            .and_then(|count| count.as_i64().ok())
            .unwrap_or(0);
        dictionary.set("Count", count + 1);
        node = dictionary
            .get(b"Parent")
            .ok()
            .and_then(|parent| parent.as_reference().ok());
    }
    doc.save(destination).map_err(|e| e.to_string())?;
    Ok(())
}

/// Edit ▸ Delete on a page (PREV-12): writes `destination` as `source`
/// without page index `page`. Every ancestor `Pages` node's `/Count` drops
/// by one and the page's objects that nothing else uses are pruned. A PDF
/// keeps at least one page.
pub fn delete_page(source: &Path, destination: &Path, page: usize) -> Result<(), String> {
    let mut doc = Document::load(source).map_err(|e| e.to_string())?;
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    if pages.len() < 2 {
        return Err("a PDF needs at least one page".to_owned());
    }
    let target = *pages.get(page).ok_or("the page does not exist")?;
    let parent = doc
        .get_dictionary(target)
        .map_err(|e| e.to_string())?
        .get(b"Parent")
        .map_err(|e| e.to_string())?
        .as_reference()
        .map_err(|e| e.to_string())?;
    {
        let kids = doc
            .get_dictionary_mut(parent)
            .map_err(|e| e.to_string())?
            .get_mut(b"Kids")
            .map_err(|e| e.to_string())?
            .as_array_mut()
            .map_err(|e| e.to_string())?;
        let position = kids
            .iter()
            .position(|kid| kid.as_reference().ok() == Some(target))
            .ok_or("page is not listed in its parent")?;
        kids.remove(position);
    }
    let mut node = Some(parent);
    for _ in 0..32 {
        let Some(id) = node else { break };
        let dictionary = doc.get_dictionary_mut(id).map_err(|e| e.to_string())?;
        let count = dictionary
            .get(b"Count")
            .ok()
            .and_then(|count| count.as_i64().ok())
            .unwrap_or(1);
        dictionary.set("Count", (count - 1).max(0));
        node = dictionary
            .get(b"Parent")
            .ok()
            .and_then(|parent| parent.as_reference().ok());
    }
    doc.delete_object(target);
    doc.prune_objects();
    doc.save(destination).map_err(|e| e.to_string())?;
    Ok(())
}

/// A PDF document that shows `path`, a PDF or an image, for Edit ▸ Insert ▸
/// Page from File… (PRV-MENU-005): a PDF is loaded as it is, an image
/// becomes a one-page PDF at its pixel size. An encrypted PDF is refused.
pub fn document_for_insertion(path: &Path) -> Result<Document, String> {
    use image::{ImageDecoder as _, ImageEncoder as _};
    let mut head = [0u8; 8];
    {
        use std::io::Read as _;
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let _ = file.read(&mut head).map_err(|e| e.to_string())?;
    }
    if head.starts_with(b"%PDF") {
        let document = Document::load(path).map_err(|e| e.to_string())?;
        if document.is_encrypted() {
            return Err("That PDF is password-protected.".to_owned());
        }
        return Ok(document);
    }
    let mut decoder = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().ok();
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    if let Some(orientation) = orientation {
        decoded.apply_orientation(orientation);
    }
    let rgb = decoded.to_rgb8();
    let (width, height) = rgb.dimensions();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .write_image(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Document::load_mem(&crate::pdfwriter::wrap_jpeg(&jpeg, width, height))
        .map_err(|e| e.to_string())
}

/// Edit ▸ Insert ▸ Page from File… (PRV-MENU-005): writes `destination` as
/// `source` with every page of `incoming` spliced in after page index
/// `after` (clamped to the last page), in order. Each inserted page keeps
/// the size, rotation and resources it had in its own file (inherited
/// page attributes are copied onto the page itself), and every ancestor
/// `Pages` node's `/Count` grows. Returns how many pages were inserted.
pub fn insert_pages_from(
    source: &Path,
    mut incoming: Document,
    destination: &Path,
    after: usize,
) -> Result<usize, String> {
    let mut doc = Document::load(source).map_err(|e| e.to_string())?;
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let anchor = *pages
        .get(after.min(pages.len().saturating_sub(1)))
        .ok_or("PDF has no pages")?;
    let parent = doc
        .get_dictionary(anchor)
        .map_err(|e| e.to_string())?
        .get(b"Parent")
        .map_err(|e| e.to_string())?
        .as_reference()
        .map_err(|e| e.to_string())?;

    let incoming_pages: Vec<_> = incoming.get_pages().into_values().collect();
    if incoming_pages.is_empty() {
        return Err("That file has no pages.".to_owned());
    }
    // Page attributes a page may inherit from its `Pages` ancestors.
    const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
    for &page in &incoming_pages {
        let mut inherited: Vec<(&[u8], Object)> = Vec::new();
        let mut ancestor = incoming
            .get_dictionary(page)
            .ok()
            .and_then(|dictionary| dictionary.get(b"Parent").ok())
            .and_then(|parent| parent.as_reference().ok());
        for _ in 0..32 {
            let Some(id) = ancestor else { break };
            let Ok(node) = incoming.get_dictionary(id) else {
                break;
            };
            for key in INHERITED {
                if let Ok(value) = node.get(key) {
                    if !inherited.iter().any(|(name, _)| *name == key) {
                        inherited.push((key, value.clone()));
                    }
                }
            }
            ancestor = node
                .get(b"Parent")
                .ok()
                .and_then(|parent| parent.as_reference().ok());
        }
        let dictionary = incoming
            .get_dictionary_mut(page)
            .map_err(|e| e.to_string())?;
        for (key, value) in inherited {
            if !dictionary.has(key) {
                dictionary.set(key, value);
            }
        }
    }

    // Bring the incoming objects over under fresh numbers.
    incoming.renumber_objects_with(doc.max_id + 1);
    let incoming_pages: Vec<_> = incoming.get_pages().into_values().collect();
    doc.max_id = incoming.max_id;
    doc.objects.extend(std::mem::take(&mut incoming.objects));
    for &page in &incoming_pages {
        doc.get_dictionary_mut(page)
            .map_err(|e| e.to_string())?
            .set("Parent", parent);
    }
    {
        let kids = doc
            .get_dictionary_mut(parent)
            .map_err(|e| e.to_string())?
            .get_mut(b"Kids")
            .map_err(|e| e.to_string())?
            .as_array_mut()
            .map_err(|e| e.to_string())?;
        let position = kids
            .iter()
            .position(|kid| kid.as_reference().ok() == Some(anchor))
            .ok_or("page is not listed in its parent")?;
        for (offset, &page) in incoming_pages.iter().enumerate() {
            kids.insert(position + 1 + offset, Object::Reference(page));
        }
    }
    let added = incoming_pages.len() as i64;
    let mut node = Some(parent);
    for _ in 0..32 {
        let Some(id) = node else { break };
        let dictionary = doc.get_dictionary_mut(id).map_err(|e| e.to_string())?;
        let count = dictionary
            .get(b"Count")
            .ok()
            .and_then(|count| count.as_i64().ok())
            .unwrap_or(0);
        dictionary.set("Count", count + added);
        node = dictionary
            .get(b"Parent")
            .ok()
            .and_then(|parent| parent.as_reference().ok());
    }
    // The other file's own catalogue and page tree are now unreferenced.
    doc.prune_objects();
    doc.save(destination).map_err(|e| e.to_string())?;
    Ok(incoming_pages.len())
}

/// Writes real PDF annotations, preserving page contents and pre-existing annotations.
/// Caller performs file I/O on a blocking worker and atomically replaces the source.
pub fn write_pdf(source: &Path, destination: &Path, items: &[Annotation]) -> Result<(), String> {
    // Settings ▸ PDF ▸ Add name to annotations. Every caller already runs
    // on a blocking worker, so reading the small settings file here is fine.
    let author = crate::settings_store::load_settings()
        .ok()
        .and_then(|settings| settings.annotation_author());
    write_pdf_with_author(source, destination, items, author.as_deref())
}

/// [`write_pdf`], each new annotation carrying `author` as its `/T`.
pub fn write_pdf_with_author(
    source: &Path,
    destination: &Path,
    items: &[Annotation],
    author: Option<&str>,
) -> Result<(), String> {
    let mut doc = Document::load(source).map_err(|e| e.to_string())?;
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    // PREV-03A: annotations the PDF already had. Ones the user moved,
    // restyled or deleted are removed here (and the changed ones written
    // again below); untouched ones stay as the PDF's own objects.
    let loaded = editable_annotations(&doc);
    let mut removed = std::collections::HashSet::new();
    for before in &loaded {
        let Some(origin) = before.origin else {
            continue;
        };
        let still_same = items
            .iter()
            .any(|item| item.origin == Some(origin) && item == before);
        if !still_same {
            removed.insert(origin);
        }
    }
    remove_annotation_objects(&mut doc, &pages, &removed);
    for item in items {
        if item.tool == Tool::Select {
            continue;
        }
        if item.origin.is_some_and(|origin| !removed.contains(&origin)) {
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
        annotation.set(
            "LuloTool",
            Object::Name(item.tool.name().as_bytes().to_vec()),
        );
        if let Some(author) = author {
            annotation.set("T", Object::string_literal(author));
        }
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
                annotation.set("Contents", pdf_text(&item.text));
                let c = rgb(item.color);
                annotation.set(
                    "DA",
                    Object::string_literal(format!(
                        "/F1 {} Tf {} {} {} rg",
                        item.font_size.max(4.0),
                        c[0].as_float().unwrap_or(0.0),
                        c[1].as_float().unwrap_or(0.0),
                        c[2].as_float().unwrap_or(0.0)
                    )),
                );
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
                annotation.set("Contents", pdf_text(&item.text));
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

/// A copy of a PDF without the annotations Preview can edit, used only to
/// draw pages: those annotations are drawn by Preview itself (so a moved
/// one is not also drawn where it used to be). Removes its file when
/// dropped.
pub struct BaseCopy(std::path::PathBuf);

impl BaseCopy {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for BaseCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Reads the editable annotations of the PDF at `path` and, when there are
/// any, writes the page-drawing copy without them (see [`BaseCopy`]).
/// Blocking; call it from a worker. Unreadable or encrypted PDFs have none.
pub fn prepare_editable(path: &Path, tag: u64) -> (Vec<Annotation>, Option<BaseCopy>) {
    let Ok(mut doc) = Document::load(path) else {
        return (Vec::new(), None);
    };
    let existing = editable_annotations(&doc);
    if existing.is_empty() {
        return (existing, None);
    }
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let ids = existing.iter().filter_map(|item| item.origin).collect();
    remove_annotation_objects(&mut doc, &pages, &ids);
    let base = std::env::temp_dir().join(format!(
        "rmac-preview-base-{}-{tag}.pdf",
        std::process::id()
    ));
    if doc.save(&base).is_err() {
        let _ = std::fs::remove_file(&base);
        return (Vec::new(), None);
    }
    (existing, Some(BaseCopy(base)))
}

fn annots_of(doc: &Document, page_id: (u32, u16)) -> Vec<Object> {
    match doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get(b"Annots").ok())
    {
        Some(Object::Array(items)) => items.clone(),
        Some(Object::Reference(id)) => doc
            .get_object(*id)
            .ok()
            .and_then(|object| object.as_array().ok())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn remove_annotation_objects(
    doc: &mut Document,
    pages: &[(u32, u16)],
    ids: &std::collections::HashSet<(u32, u16)>,
) {
    if ids.is_empty() {
        return;
    }
    for &page_id in pages {
        let annots = annots_of(doc, page_id);
        if !annots
            .iter()
            .any(|item| item.as_reference().is_ok_and(|id| ids.contains(&id)))
        {
            continue;
        }
        let kept: Vec<Object> = annots
            .into_iter()
            .filter(|item| !item.as_reference().is_ok_and(|id| ids.contains(&id)))
            .collect();
        if let Ok(page) = doc.get_dictionary_mut(page_id) {
            page.set("Annots", kept);
        }
    }
    for id in ids {
        doc.delete_object(*id);
    }
}

fn media_size(doc: &Document, page_id: (u32, u16)) -> Option<(f32, f32)> {
    let mut ancestor = page_id;
    for _ in 0..32 {
        let page = doc.get_dictionary(ancestor).ok()?;
        if let Ok(value) = page.get(b"MediaBox") {
            let media = match value {
                Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?,
                other => other.as_array().ok()?,
            };
            let coord = |i: usize| media.get(i).and_then(|v| v.as_float().ok());
            let (x0, y0, x1, y1) = (coord(0)?, coord(1)?, coord(2)?, coord(3)?);
            // The writer places annotations in zero-origin page space.
            if x0 != 0.0 || y0 != 0.0 || x1 - x0 < 1.0 || y1 - y0 < 1.0 {
                return None;
            }
            return Some((x1 - x0, y1 - y0));
        }
        ancestor = page.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn floats(object: &Object, doc: &Document) -> Option<Vec<f32>> {
    let array = match object {
        Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?,
        other => other.as_array().ok()?,
    };
    array.iter().map(|value| value.as_float().ok()).collect()
}

/// Reads the annotations Preview can edit from every page of `doc`:
/// highlights, underlines, strike-throughs, text boxes, rectangles, ovals,
/// lines, arrows, ink, polygons and notes, plus Lulo's own shapes. Anything
/// that cannot be reproduced faithfully (a highlight spanning several
/// lines, a filled shape, a hidden annotation, several ink strokes) is
/// skipped and stays part of the page as the PDF drew it.
fn editable_annotations(doc: &Document) -> Vec<Annotation> {
    let mut out = Vec::new();
    for (index, page_id) in doc.get_pages().into_values().enumerate() {
        let Some(size) = media_size(doc, page_id) else {
            continue;
        };
        for entry in annots_of(doc, page_id) {
            let Ok(id) = entry.as_reference() else {
                continue;
            };
            let Ok(dict) = doc.get_dictionary(id) else {
                continue;
            };
            if let Some(mut item) = parse_annotation(doc, dict, index, size) {
                item.origin = Some(id);
                out.push(item);
            }
        }
    }
    out
}

fn parse_annotation(
    doc: &Document,
    dict: &lopdf::Dictionary,
    page: usize,
    size: (f32, f32),
) -> Option<Annotation> {
    let subtype = dict.get(b"Subtype").ok()?.as_name().ok()?;
    if dict
        .get(b"F")
        .ok()
        .and_then(|flags| flags.as_i64().ok())
        .is_some_and(|flags| flags & 0b10 != 0)
    {
        return None;
    }
    let lulo = dict
        .get(b"LuloTool")
        .ok()
        .and_then(|name| name.as_name().ok())
        .and_then(Tool::from_name);
    let tool = match (lulo, subtype) {
        (Some(tool), _) => tool,
        (None, b"Highlight") => Tool::Highlight,
        (None, b"Underline") => Tool::Underline,
        (None, b"StrikeOut") => Tool::StrikeThrough,
        (None, b"FreeText") => Tool::Text,
        (None, b"Square") => Tool::Rectangle,
        (None, b"Circle") => Tool::Oval,
        (None, b"Line") => {
            let arrow = dict
                .get(b"LE")
                .ok()
                .and_then(|ends| ends.as_array().ok())
                .is_some_and(|ends| {
                    ends.iter().any(|end| {
                        end.as_name()
                            .is_ok_and(|name| name != b"None" && name != b"")
                    })
                });
            if arrow {
                Tool::Arrow
            } else {
                Tool::Line
            }
        }
        (None, b"Ink") => Tool::Sketch,
        (None, b"Polygon") => Tool::Polygon,
        (None, b"Text") => Tool::Note,
        _ => return None,
    };
    // A Lulo shape must still be the PDF subtype it was written as.
    let expected: &[&[u8]] = match tool {
        Tool::Highlight => &[b"Highlight"],
        Tool::Underline => &[b"Underline"],
        Tool::StrikeThrough => &[b"StrikeOut"],
        Tool::Text => &[b"FreeText"],
        Tool::Oval => &[b"Circle"],
        Tool::Line | Tool::Arrow => &[b"Line"],
        Tool::Sketch | Tool::Signature => &[b"Ink"],
        Tool::Polygon => &[b"Polygon"],
        Tool::Note => &[b"Text"],
        _ => &[b"Square"],
    };
    if !expected.contains(&subtype) {
        return None;
    }
    let rect = floats(dict.get(b"Rect").ok()?, doc)?;
    if rect.len() != 4 || rect.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let bounds = normal(UnitRect {
        x0: rect[0] / size.0,
        y0: 1.0 - rect[3] / size.1,
        x1: rect[2] / size.0,
        y1: 1.0 - rect[1] / size.1,
    });
    let color = dict
        .get(b"C")
        .ok()
        .and_then(|colour| floats(colour, doc))
        .filter(|channels| channels.len() == 3)
        .map(|channels| {
            channels.iter().fold(0u32, |packed, channel| {
                (packed << 8) | (channel.clamp(0.0, 1.0) * 255.0).round() as u32
            })
        })
        .unwrap_or(0);
    let width = dict
        .get(b"BS")
        .ok()
        .and_then(|style| match style {
            Object::Reference(id) => doc.get_dictionary(*id).ok(),
            other => other.as_dict().ok(),
        })
        .and_then(|style| style.get(b"W").ok())
        .and_then(|value| value.as_float().ok())
        .or_else(|| {
            dict.get(b"Border")
                .ok()
                .and_then(|border| floats(border, doc))
                .and_then(|border| border.get(2).copied())
        })
        .unwrap_or(1.0);
    let mut item = Annotation::new(page, tool, bounds, color, width);
    item.fill = dict
        .get(b"IC")
        .ok()
        .and_then(|interior| floats(interior, doc))
        .filter(|channels| channels.len() == 3)
        .map(|channels| {
            channels.iter().fold(0u32, |packed, channel| {
                (packed << 8) | (channel.clamp(0.0, 1.0) * 255.0).round() as u32
            })
        });
    if tool == Tool::Text {
        // "/F1 18 Tf 0 0 0 rg": the number before Tf is the size.
        let default_appearance = dict.get(b"DA").ok().and_then(pdf_string);
        if let Some(size) = default_appearance.as_deref().and_then(|da| {
            let tokens: Vec<&str> = da.split_whitespace().collect();
            let at = tokens.iter().position(|token| *token == "Tf")?;
            tokens.get(at.checked_sub(1)?)?.parse::<f32>().ok()
        }) {
            item.font_size = size.clamp(4.0, 400.0);
        }
    }
    item.text = dict
        .get(b"Contents")
        .ok()
        .and_then(pdf_string)
        .unwrap_or_default();
    let spread = (
        (bounds.x1 - bounds.x0).max(1e-4),
        (bounds.y1 - bounds.y0).max(1e-4),
    );
    let relative = |x: f32, y: f32| {
        (
            ((x / size.0 - bounds.x0) / spread.0).clamp(0.0, 1.0),
            (((1.0 - y / size.1) - bounds.y0) / spread.1).clamp(0.0, 1.0),
        )
    };
    let points = |values: &[f32]| -> Vec<(f32, f32)> {
        values
            .chunks_exact(2)
            .map(|pair| relative(pair[0], pair[1]))
            .collect()
    };
    match tool {
        Tool::Highlight | Tool::Underline | Tool::StrikeThrough => {
            // Only a single run of text; a multi-line markup is left as the
            // PDF drew it.
            let quads = dict
                .get(b"QuadPoints")
                .ok()
                .and_then(|quads| floats(quads, doc))?;
            if quads.len() != 8 {
                return None;
            }
        }
        Tool::Sketch | Tool::Signature => {
            let strokes = dict.get(b"InkList").ok()?.as_array().ok()?;
            if strokes.len() != 1 {
                return None;
            }
            item.path = points(&floats(&strokes[0], doc)?);
            if item.path.len() < 2 {
                return None;
            }
        }
        Tool::Line | Tool::Arrow => {
            let line = floats(dict.get(b"L").ok()?, doc)?;
            if line.len() != 4 {
                return None;
            }
            item.path = points(&line);
        }
        Tool::Polygon => {
            let vertices = floats(dict.get(b"Vertices").ok()?, doc)?;
            item.path = points(&vertices);
            if item.path.len() < 3 {
                return None;
            }
        }
        _ => {}
    }
    Some(item)
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

/// A PDF text string, or (as older Lulo files stored it) a name.
fn pdf_string(object: &Object) -> Option<String> {
    match object {
        Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        other => decode_outline_title(other),
    }
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
        // Settings ▸ PDF ▸ Add name to annotations writes the author.
        let named = base.join("named.pdf");
        write_pdf_with_author(&source, &named, &items[..1], Some("Jacob")).unwrap();
        let named = Document::load(&named).unwrap();
        let annotation = named.get_page_annotations(page).unwrap()[0];
        assert_eq!(annotation.get(b"T").unwrap().as_str().unwrap(), b"Jacob");
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

    fn two_page_pdf(path: &Path) -> (lopdf::ObjectId, lopdf::ObjectId) {
        let mut doc = Document::with_version("1.4");
        let pages = doc.new_object_id();
        let first = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()] });
        let second = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()] });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![first.into(), second.into()], "Count" => 2 },
            ),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", root);
        doc.save(path).unwrap();
        (first, second)
    }

    #[test]
    fn insert_blank_page_splices_a_same_size_page_after_the_current_one() {
        let base = std::env::temp_dir().join(format!("rmac-blank-page-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let source = base.join("source.pdf");
        let saved = base.join("saved.pdf");
        let (first, second) = two_page_pdf(&source);
        insert_blank_page(&source, &saved, 0).unwrap();
        let after = Document::load(&saved).unwrap();
        let ids: Vec<_> = after.get_pages().into_values().collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], first);
        assert_eq!(ids[2], second);
        let blank = after.get_dictionary(ids[1]).unwrap();
        let media: Vec<i64> = blank
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        assert_eq!(media, vec![0, 0, 300, 400]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn pages_from_another_file_are_spliced_in_order_with_their_own_size() {
        let base = std::env::temp_dir().join(format!("rmac-page-from-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let source = base.join("source.pdf");
        let other = base.join("other.pdf");
        let saved = base.join("saved.pdf");
        let (first, second) = two_page_pdf(&source);
        two_page_pdf(&other);
        let incoming = document_for_insertion(&other).unwrap();
        let added = insert_pages_from(&source, incoming, &saved, 0).unwrap();
        assert_eq!(added, 2);
        let after = Document::load(&saved).unwrap();
        let ids: Vec<_> = after.get_pages().into_values().collect();
        assert_eq!(ids.len(), 4);
        assert_eq!(ids[0], first);
        assert_eq!(ids[3], second);
        let width = |id| {
            after
                .get_dictionary(id)
                .unwrap()
                .get(b"MediaBox")
                .unwrap()
                .as_array()
                .unwrap()[2]
                .as_i64()
                .unwrap()
        };
        assert_eq!([width(ids[1]), width(ids[2])], [300, 612]);
        let root = after.catalog().unwrap();
        let pages = after
            .get_dictionary(root.get(b"Pages").unwrap().as_reference().unwrap())
            .unwrap();
        assert_eq!(pages.get(b"Count").unwrap().as_i64().unwrap(), 4);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_image_becomes_a_page_at_its_pixel_size() {
        let base = std::env::temp_dir().join(format!("rmac-image-page-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let picture = base.join("picture.png");
        image::RgbImage::from_pixel(40, 20, image::Rgb([200, 30, 30]))
            .save(&picture)
            .unwrap();
        let document = document_for_insertion(&picture).unwrap();
        let ids: Vec<_> = document.get_pages().into_values().collect();
        assert_eq!(ids.len(), 1);
        let media = document
            .get_dictionary(ids[0])
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_float().unwrap() as i64)
            .collect::<Vec<_>>();
        assert_eq!(media, [0, 0, 40, 20]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn insert_blank_page_after_an_out_of_range_index_appends_at_the_end() {
        let base = std::env::temp_dir().join(format!("rmac-blank-end-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let source = base.join("source.pdf");
        let saved = base.join("saved.pdf");
        let (_, second) = two_page_pdf(&source);
        insert_blank_page(&source, &saved, 99).unwrap();
        let ids: Vec<_> = Document::load(&saved)
            .unwrap()
            .get_pages()
            .into_values()
            .collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[1], second);
        let _ = std::fs::remove_dir_all(&base);
    }

    fn one_page_pdf(path: &Path) -> (u32, u16) {
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
        doc.save(path).unwrap();
        page
    }

    #[test]
    fn existing_annotations_are_read_back_edited_and_deleted() {
        let base = std::env::temp_dir().join(format!("rmac-markup-edit-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let (source, first, second) = (
            base.join("source.pdf"),
            base.join("first.pdf"),
            base.join("second.pdf"),
        );
        let page = one_page_pdf(&source);
        let mut line = Annotation::new(0, Tool::Arrow, rect(0.1, 0.1, 0.5, 0.3), 0x0000ff, 3.0);
        line.path = vec![(0.0, 1.0), (1.0, 0.0)];
        let mut note = Annotation::new(0, Tool::Note, rect(0.6, 0.6, 0.7, 0.7), 0xffcc00, 1.0);
        note.text = "remember".into();
        let star = Annotation::new(0, Tool::Star, rect(0.2, 0.5, 0.4, 0.7), 0xff0000, 2.0);
        write_pdf(&source, &first, &[line, note, star]).unwrap();

        // A new session reads all three back as themselves.
        let (existing, copy) = prepare_editable(&first, 9001);
        assert_eq!(existing.len(), 3);
        assert_eq!(existing[0].tool, Tool::Arrow);
        assert_eq!(existing[0].path, vec![(0.0, 1.0), (1.0, 0.0)]);
        assert!((existing[0].width - 3.0).abs() < 1e-3);
        assert_eq!(existing[0].color, 0x0000ff);
        assert_eq!(existing[1].tool, Tool::Note);
        assert_eq!(existing[1].text, "remember");
        assert_eq!(existing[2].tool, Tool::Star);
        assert!((existing[2].rect.x0 - 0.2).abs() < 1e-3);
        assert!(existing.iter().all(|item| item.origin.is_some()));
        // The page-drawing copy has none of them, the file keeps all.
        let copy = copy.expect("a base copy");
        assert!(Document::load(copy.path())
            .unwrap()
            .get_page_annotations(page)
            .unwrap()
            .is_empty());
        assert_eq!(
            Document::load(&first)
                .unwrap()
                .get_page_annotations(page)
                .unwrap()
                .len(),
            3
        );

        // Saving with nothing changed leaves the PDF's own objects alone.
        write_pdf(&first, &second, &existing).unwrap();
        assert_eq!(
            Document::load(&second)
                .unwrap()
                .get_page_annotations(page)
                .unwrap()
                .len(),
            3
        );

        // Move the star and delete the note: the star is replaced (not
        // duplicated), the note is gone, the arrow is untouched.
        let mut edited = existing.clone();
        edited[2].translate((0.3, 0.0));
        edited.remove(1);
        write_pdf(&first, &second, &edited).unwrap();
        let (after, _) = prepare_editable(&second, 9002);
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].tool, Tool::Arrow);
        let star = after.iter().find(|item| item.tool == Tool::Star).unwrap();
        assert!((star.rect.x0 - 0.5).abs() < 1e-3);
        assert!(after.iter().all(|item| item.tool != Tool::Note));
        drop(copy);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn other_peoples_annotations_are_read_when_they_are_simple() {
        let base = std::env::temp_dir().join(format!("rmac-markup-foreign-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let source = base.join("source.pdf");
        let page = one_page_pdf(&source);
        let mut doc = Document::load(&source).unwrap();
        let square = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Square", "Rect" => vec![61.2.into(), 396.0.into(), 244.8.into(), 633.6.into()],
            "C" => vec![1.0.into(), 0.0.into(), 0.0.into()], "BS" => dictionary! { "W" => 2.5 },
        });
        let filled = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Square", "Rect" => vec![10.0.into(), 10.0.into(), 50.0.into(), 50.0.into()],
            "IC" => vec![0.0.into(), 1.0.into(), 0.0.into()],
        });
        let link = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![0.0.into(), 0.0.into(), 5.0.into(), 5.0.into()],
        });
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Annots", vec![square.into(), filled.into(), link.into()]);
        doc.save(&source).unwrap();
        let (existing, copy) = prepare_editable(&source, 9003);
        // Both squares are editable (one is filled); the link stays baked
        // into the page.
        assert_eq!(existing.len(), 2);
        assert_eq!(existing[0].tool, Tool::Rectangle);
        assert_eq!(existing[0].color, 0xff0000);
        assert_eq!(existing[0].fill, None);
        assert_eq!(existing[1].fill, Some(0x00ff00));
        assert!((existing[0].rect.x0 - 0.1).abs() < 1e-3);
        assert!((existing[0].rect.y0 - 0.2).abs() < 1e-3);
        let kept = Document::load(copy.unwrap().path())
            .unwrap()
            .get_page_annotations(page)
            .unwrap()
            .len();
        assert_eq!(kept, 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn revert_returns_to_the_annotations_the_pdf_started_with() {
        let mut markup = Markup::default();
        let mut existing = Annotation::new(0, Tool::Rectangle, rect(0.1, 0.1, 0.2, 0.2), 1, 1.0);
        existing.origin = Some((7, 0));
        markup.adopt(vec![existing.clone()]);
        assert!(!markup.dirty);
        markup.checkpoint();
        markup.items.clear();
        assert!(markup.dirty);
        markup.revert();
        assert_eq!(markup.items, vec![existing]);
        assert!(!markup.dirty);
    }

    #[test]
    fn deleting_a_page_removes_it_and_keeps_the_rest_in_order() {
        let base = std::env::temp_dir().join(format!("rmac-markup-delete-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let (source, shorter) = (base.join("two.pdf"), base.join("one.pdf"));
        let (first, second) = two_page_pdf(&source);
        delete_page(&source, &shorter, 0).unwrap();
        let after = Document::load(&shorter).unwrap();
        let pages: Vec<_> = after.get_pages().into_values().collect();
        assert_eq!(pages, vec![second]);
        assert!(!after.objects.contains_key(&first));
        let count = after
            .get_dictionary(
                after
                    .catalog()
                    .unwrap()
                    .get(b"Pages")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
            )
            .unwrap()
            .get(b"Count")
            .unwrap()
            .as_i64()
            .unwrap();
        assert_eq!(count, 1);
        // The last page cannot go, nor one that is not there.
        assert!(delete_page(&shorter, &base.join("none.pdf"), 0).is_err());
        assert!(delete_page(&source, &base.join("none.pdf"), 5).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}
