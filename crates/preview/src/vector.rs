//! SVG documents (PREV-09) and the SVG-based drawing used to bake markup
//! into image pixels, both rendered by `resvg` (already part of Lulo).

use std::sync::{Arc, OnceLock};

use image::RgbaImage;
use resvg::{tiny_skia, usvg};

/// Longest side Preview rasterises an SVG to; vector art is drawn crisply
/// at this size and scaled by the view from there.
const LONG_SIDE: u32 = 2048;
/// Largest SVG document accepted.
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_COORDINATE: f32 = 1.0e6;

/// Whether `bytes` start like an SVG document: optional BOM, XML prolog and
/// comments, then an `<svg` element.
pub fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(2048)];
    let Ok(text) = std::str::from_utf8(head).or_else(|error| {
        // A multi-byte character cut by the 2 KiB window is fine.
        std::str::from_utf8(&head[..error.valid_up_to()])
    }) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with("<?xml")
        || text.starts_with("<svg")
        || text.starts_with("<!--")
        || text.starts_with("<!DOCTYPE"))
        && text.contains("<svg")
}

fn options() -> usvg::Options<'static> {
    // Fonts are loaded once; scanning the system fonts is the slow part.
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut database = usvg::fontdb::Database::new();
        database.load_system_fonts();
        Arc::new(database)
    });
    usvg::Options {
        resources_dir: None,
        // A document must not read other files or the network.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        fontdb: Arc::clone(fonts),
        ..Default::default()
    }
}

fn unpremultiply(pixmap: tiny_skia::Pixmap, width: u32, height: u32) -> Result<RgbaImage, String> {
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(width, height, rgba).ok_or_else(|| "bad SVG bitmap".to_owned())
}

/// Renders an SVG document to RGBA, returning the picture and the
/// document's own size in points.
pub fn decode(bytes: &[u8]) -> Result<(RgbaImage, (u32, u32)), String> {
    if bytes.len() > MAX_BYTES {
        return Err("the SVG is too large".to_owned());
    }
    let tree = usvg::Tree::from_data(bytes, &options()).map_err(|error| error.to_string())?;
    let (width, height) = (tree.size().width(), tree.size().height());
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > MAX_COORDINATE
        || height > MAX_COORDINATE
    {
        return Err("the SVG has an unsupported size".to_owned());
    }
    let scale = LONG_SIDE as f32 / width.max(height);
    let pixel_width = (width * scale).round().clamp(1.0, LONG_SIDE as f32) as u32;
    let pixel_height = (height * scale).round().clamp(1.0, LONG_SIDE as f32) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(pixel_width, pixel_height)
        .ok_or_else(|| "the SVG is too large".to_owned())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(pixel_width as f32 / width, pixel_height as f32 / height),
        &mut pixmap.as_mut(),
    );
    let picture = unpremultiply(pixmap, pixel_width, pixel_height)?;
    Ok((picture, (width.round() as u32, height.round() as u32)))
}

/// Renders drawing `svg` (an `<svg>` document of exactly `width` x `height`
/// user units) onto a transparent bitmap of that size.
pub fn render_overlay(svg: &str, width: u32, height: u32) -> Result<RgbaImage, String> {
    let tree = usvg::Tree::from_str(svg, &options()).map_err(|error| error.to_string())?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| "the image is too large to draw on".to_owned())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    unpremultiply(pixmap, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#ff0000"/></svg>"##;

    #[test]
    fn an_svg_is_recognised_by_its_content() {
        assert!(is_svg(SQUARE.as_bytes()));
        assert!(is_svg(b"<?xml version=\"1.0\"?>\n<!-- c --><svg></svg>"));
        assert!(!is_svg(b"<html><body>no</body></html>"));
        assert!(!is_svg(b"\x89PNG\r\n"));
    }

    #[test]
    fn an_svg_renders_at_a_crisp_size_with_its_own_size_reported() {
        let (picture, natural) = decode(SQUARE.as_bytes()).unwrap();
        assert_eq!(natural, (40, 20));
        assert_eq!(picture.dimensions(), (2048, 1024));
        assert_eq!(picture.get_pixel(1000, 500).0, [255, 0, 0, 255]);
    }

    #[test]
    fn external_files_are_never_read_and_garbage_is_an_error() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10"><image xlink:href="/etc/hostname" width="10" height="10"/></svg>"#;
        let (picture, _) = decode(svg.as_bytes()).unwrap();
        assert_eq!(picture.get_pixel(5, 5).0[3], 0);
        assert!(decode(b"<svg").is_err());
    }

    #[test]
    fn an_overlay_is_transparent_except_where_it_draws() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect x="2" y="2" width="4" height="4" fill="#0000ff"/></svg>"##;
        let picture = render_overlay(svg, 10, 10).unwrap();
        assert_eq!(picture.get_pixel(0, 0).0[3], 0);
        assert_eq!(picture.get_pixel(3, 3).0, [0, 0, 255, 255]);
    }
}
