//! Tools ▸ Redact (PRV-MENU-015): real content removal, not a box drawn
//! over the original. For an image, the selected pixels are permanently
//! overwritten with solid black (`redact_image_region`) — the original
//! colour values are gone from the buffer that gets saved. For a PDF
//! page, the whole page is rasterised, the redaction rectangle is
//! painted solid black on that raster *before* it is embedded, and the
//! page's `/Contents`/`/Resources` are replaced with just that one image
//! (`redact_pdf_page`) — the original vector text and fonts for that
//! page are discarded, not hidden under an overlay, so a reader (or
//! `render::extract_text`) finds nothing left in the redacted area. The
//! page's other content (other pages, the document's other resources)
//! is untouched.

use std::path::Path;

use image::{Rgba, RgbaImage};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};

use crate::layout::Rotation;
use crate::render;

/// A rectangle in PDF point space (the MediaBox's own coordinate system,
/// origin at the bottom-left) — the same frame `markup::write_pdf` uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl PdfRect {
    fn normalized(self) -> (f32, f32, f32, f32) {
        (
            self.x0.min(self.x1),
            self.y0.min(self.y1),
            self.x0.max(self.x1),
            self.y0.max(self.y1),
        )
    }
}

/// Overwrites `rect` (pixel coordinates, clamped to the image) with solid
/// opaque black — a real, destructive edit to the pixel buffer the
/// caller then saves, not a drawn overlay a viewer could peel back.
pub fn redact_image_region(pixels: &mut RgbaImage, rect: (u32, u32, u32, u32)) {
    let (width, height) = pixels.dimensions();
    let (x, y, w, h) = rect;
    let x1 = x.saturating_add(w).min(width);
    let y1 = y.saturating_add(h).min(height);
    for py in y.min(height)..y1 {
        for px in x.min(width)..x1 {
            pixels.put_pixel(px, py, Rgba([0, 0, 0, 255]));
        }
    }
}

/// Paints `rect` (PDF point space) solid black on a page raster of
/// `pixel_size`, rendered from a page whose MediaBox is `page_size` PDF
/// points — i.e. converts point space to the raster's own pixel space
/// (PDF y grows up from the bottom; image y grows down from the top).
fn paint_black_rect(pixels: &mut RgbaImage, page_size: (f32, f32), rect: PdfRect) {
    let (page_w, page_h) = page_size;
    if page_w <= 0.0 || page_h <= 0.0 {
        return;
    }
    let (img_w, img_h) = pixels.dimensions();
    let (x0, y0, x1, y1) = rect.normalized();
    let px_x = |x: f32| {
        ((x / page_w) * img_w as f32)
            .round()
            .clamp(0.0, img_w as f32) as u32
    };
    let px_y = |y: f32| {
        (((page_h - y) / page_h) * img_h as f32)
            .round()
            .clamp(0.0, img_h as f32) as u32
    };
    let left = px_x(x0);
    let right = px_x(x1);
    let top = px_y(y1);
    let bottom = px_y(y0);
    redact_image_region(
        pixels,
        (
            left,
            top,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        ),
    );
}

fn page_media_box(doc: &Document, mut page_id: ObjectId) -> Result<(f32, f32), String> {
    for _ in 0..32 {
        let page = doc.get_dictionary(page_id).map_err(|e| e.to_string())?;
        if let Ok(value) = page.get(b"MediaBox") {
            let array = value.as_array().map_err(|e| e.to_string())?;
            let coord = |i: usize| array.get(i).and_then(|v| v.as_float().ok()).unwrap_or(0.0);
            return Ok((
                (coord(2) - coord(0)).max(1.0),
                (coord(3) - coord(1)).max(1.0),
            ));
        }
        page_id = page
            .get(b"Parent")
            .map_err(|e| e.to_string())?
            .as_reference()
            .map_err(|e| e.to_string())?;
    }
    Err("PDF page has no MediaBox".into())
}

/// Replaces `page_id`'s `/Contents` and `/Resources` with a single full-
/// page image XObject built from `pixels` (already redacted), discarding
/// whatever text, fonts and vector graphics the page had. Pure `lopdf`
/// (no poppler process), so it is unit-testable without pdftoppm/
/// pdftotext — see `redact_pdf_page` for the rasterising wrapper.
fn write_rasterized_page(
    doc: &mut Document,
    page_id: ObjectId,
    page_size: (f32, f32),
    pixels: &RgbaImage,
) -> Result<(), String> {
    let (jpeg, width, height) = render::encode_print_jpeg(pixels, Rotation::default())?;
    let image_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(width),
            "Height" => i64::from(height),
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg,
    ));
    let content = format!(
        "q {w} 0 0 {h} 0 0 cm /RedactedPage Do Q",
        w = page_size.0,
        h = page_size.1
    );
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let dict = doc.get_dictionary_mut(page_id).map_err(|e| e.to_string())?;
    dict.set("Contents", Object::Reference(content_id));
    dict.set(
        "Resources",
        Object::Dictionary(dictionary! {
            "XObject" => dictionary! { "RedactedPage" => Object::Reference(image_id) },
        }),
    );
    Ok(())
}

/// Tools ▸ Redact for a PDF page (PRV-MENU-015): rasterises `page_index`
/// from `source` at a fixed resolution, burns `rect` in black into that
/// raster, then rewrites the page to show only the result, and saves to
/// `destination`. Caller runs this on a blocking worker, like
/// `markup::write_pdf`.
pub fn redact_pdf_page(
    source: &Path,
    destination: &Path,
    page_index: usize,
    rect: PdfRect,
) -> Result<(), String> {
    let mut doc = Document::load(source).map_err(|e| e.to_string())?;
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let page_id = *pages.get(page_index).ok_or("redact page is out of range")?;
    let page_size = page_media_box(&doc, page_id)?;
    // 2x scale keeps redacted text pages reasonably crisp without the
    // raster becoming huge; matches the general spirit of
    // `metrics`/`zoom`'s existing bounded render scales elsewhere.
    let mut pixels = render::render_page(source, page_index, page_size, 2.0, Rotation::default())?;
    paint_black_rect(&mut pixels, page_size, rect);
    write_rasterized_page(&mut doc, page_id, page_size, &pixels)?;
    doc.save(destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, color: Rgba<u8>) -> RgbaImage {
        let mut pixels = RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                pixels.put_pixel(x, y, color);
            }
        }
        pixels
    }

    #[test]
    fn redact_image_region_overwrites_exactly_the_requested_rect() {
        let mut pixels = solid(6, 4, Rgba([200, 150, 50, 255]));
        redact_image_region(&mut pixels, (1, 1, 2, 2));
        for y in 0..4 {
            for x in 0..6 {
                let inside = (1..3).contains(&x) && (1..3).contains(&y);
                let expected = if inside {
                    Rgba([0, 0, 0, 255])
                } else {
                    Rgba([200, 150, 50, 255])
                };
                assert_eq!(*pixels.get_pixel(x, y), expected, "at ({x},{y})");
            }
        }
    }

    #[test]
    fn redact_image_region_clamps_to_the_image_without_panicking() {
        let mut pixels = solid(3, 3, Rgba([1, 2, 3, 255]));
        redact_image_region(&mut pixels, (2, 2, 100, 100));
        assert_eq!(*pixels.get_pixel(2, 2), Rgba([0, 0, 0, 255]));
        assert_eq!(*pixels.get_pixel(0, 0), Rgba([1, 2, 3, 255]));
    }

    #[test]
    fn paint_black_rect_converts_pdf_points_to_image_pixels() {
        // A 100x100pt page rendered at 10x10 pixels: a rect covering the
        // top 20pt (PDF y near the top of the page, i.e. image rows 0-1)
        // and the left 20pt should blacken pixel columns/rows 0-1.
        let mut pixels = solid(10, 10, Rgba([255, 255, 255, 255]));
        paint_black_rect(
            &mut pixels,
            (100.0, 100.0),
            PdfRect {
                x0: 0.0,
                y0: 80.0,
                x1: 20.0,
                y1: 100.0,
            },
        );
        assert_eq!(*pixels.get_pixel(0, 0), Rgba([0, 0, 0, 255]));
        assert_eq!(*pixels.get_pixel(1, 1), Rgba([0, 0, 0, 255]));
        assert_eq!(*pixels.get_pixel(5, 5), Rgba([255, 255, 255, 255]));
        assert_eq!(*pixels.get_pixel(9, 9), Rgba([255, 255, 255, 255]));
    }

    fn single_page_pdf_with_text(text: &str) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.5");
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let content = format!("BT /F1 24 Tf 10 700 Td ({text}) Tj ET");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content_id,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let root_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", root_id);
        (doc, page_id)
    }

    #[test]
    fn write_rasterized_page_replaces_contents_and_drops_the_font_resource() {
        let (mut doc, page_id) = single_page_pdf_with_text("Secret 123");
        let original_contents = doc
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Contents")
            .unwrap()
            .clone();
        let pixels = solid(20, 20, Rgba([10, 20, 30, 255]));
        write_rasterized_page(&mut doc, page_id, (612.0, 792.0), &pixels).unwrap();

        let page = doc.get_dictionary(page_id).unwrap();
        let new_contents = page.get(b"Contents").unwrap().clone();
        assert_ne!(new_contents, original_contents);

        // The new Resources dictionary carries only the embedded image —
        // the original page's Font resource (and so its ability to show
        // the original text as vector glyphs) is gone.
        let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
        assert!(resources.get(b"Font").is_err());
        assert!(resources.get(b"XObject").is_ok());

        // The new content stream really is just "draw the one image":
        // the original text string is nowhere in it.
        let content_id = new_contents.as_reference().unwrap();
        let stream = doc.get_object(content_id).unwrap().as_stream().unwrap();
        let content_text = String::from_utf8_lossy(&stream.content);
        assert!(!content_text.contains("Secret"));
        assert!(content_text.contains("/RedactedPage"));
    }
}
