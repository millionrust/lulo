//! Blocking document work that runs on background threads: recognising and
//! decoding files, poppler subprocesses, rotation and clipboard encoding.
//! Pixels are kept in GPUI's BGRA order so they become textures unchanged.

use std::fs::File;
use std::io::{BufWriter, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crate::document::{self, ImageKind, Kind};
use crate::layout::Rotation;
use crate::poppler::{self, PdfInfo, TextPage};
use gpui::RenderImage;
use image::{ImageDecoder as _, ImageEncoder as _, ImageReader, RgbaImage};

/// Longest image side kept in memory; larger images are downsampled once so
/// a single texture stays within what low-end GPUs accept.
const MAX_IMAGE_SIDE: u32 = 8192;
/// Bound Preview's raster print intermediate even when the opened image is
/// at the maximum decode size. The source is still kept at full display
/// resolution; only the PDF sent to the print portal is reduced.
const MAX_PRINT_IMAGE_SIDE: u32 = 4096;
/// Sidebar thumbnails are drawn at 120 pt; 2× covers HiDPI screens.
const THUMB_PIXELS: u32 = 240;

#[derive(Clone)]
pub struct FileFacts {
    pub size: u64,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
}

#[derive(Clone)]
pub enum Content {
    Image(ImageContent),
    Pdf(Arc<PdfInfo>),
}

#[derive(Clone)]
pub struct ImageContent {
    /// Decoded pixels (BGRA), EXIF orientation applied.
    pub pixels: Arc<RgbaImage>,
    /// The file's pixel size after orientation (before any downsampling).
    pub size: (u32, u32),
    pub colour_model: &'static str,
    pub thumbnail: Arc<RenderImage>,
}

#[derive(Clone)]
pub struct Loaded {
    pub kind: Kind,
    pub facts: FileFacts,
    pub content: Content,
}

impl Loaded {
    pub fn page_count(&self) -> usize {
        match &self.content {
            Content::Image(_) => 1,
            Content::Pdf(info) => info.pages.len(),
        }
    }
}

pub fn sniff_path(path: &Path) -> Result<Kind, String> {
    let mut head = [0_u8; 1024];
    let mut file = File::open(path).map_err(|error| open_error(path, &error.to_string()))?;
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) => return Err(open_error(path, &error.to_string())),
        }
    }
    document::sniff(&head[..filled]).ok_or_else(|| {
        format!(
            "“{}” can’t be opened because it isn’t a PDF or a supported image.",
            document::display_name(path)
        )
    })
}

fn open_error(path: &Path, detail: &str) -> String {
    format!(
        "“{}” couldn’t be opened. {detail}",
        document::display_name(path)
    )
}

pub fn load(path: &Path) -> Result<Loaded, String> {
    let kind = sniff_path(path)?;
    let metadata = std::fs::metadata(path).map_err(|error| open_error(path, &error.to_string()))?;
    let facts = FileFacts {
        size: metadata.len(),
        created: metadata.created().ok(),
        modified: metadata.modified().ok(),
    };
    let content = match kind {
        Kind::Image(_) => Content::Image(load_image(path)?),
        Kind::Pdf => Content::Pdf(Arc::new(load_pdf_info(path)?)),
    };
    Ok(Loaded {
        kind,
        facts,
        content,
    })
}

/// Pixel size of an image after EXIF orientation, from its header only.
pub fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut decoder = ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let (width, height) = decoder.dimensions();
    let swaps = decoder.orientation().is_ok_and(|orientation| {
        use image::metadata::Orientation::*;
        matches!(
            orientation,
            Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH
        )
    });
    Some(if swaps {
        (height, width)
    } else {
        (width, height)
    })
}

fn load_image(path: &Path) -> Result<ImageContent, String> {
    let failure = |detail: String| open_error(path, &detail);
    let mut decoder = ImageReader::open(path)
        .map_err(|error| failure(error.to_string()))?
        .with_guessed_format()
        .map_err(|error| failure(error.to_string()))?
        .into_decoder()
        .map_err(|error| failure(error.to_string()))?;
    let orientation = decoder.orientation().ok();
    let colour_model = match decoder.color_type() {
        image::ColorType::L8
        | image::ColorType::La8
        | image::ColorType::L16
        | image::ColorType::La16 => "Grey",
        _ => "RGB",
    };
    let mut decoded =
        image::DynamicImage::from_decoder(decoder).map_err(|error| failure(error.to_string()))?;
    if let Some(orientation) = orientation {
        decoded.apply_orientation(orientation);
    }
    let size = (decoded.width(), decoded.height());
    if size.0.max(size.1) > MAX_IMAGE_SIDE {
        decoded = decoded.thumbnail(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE);
    }
    let mut pixels = decoded.into_rgba8();
    swap_red_blue(&mut pixels);
    let thumbnail = image_thumbnail(&pixels);
    Ok(ImageContent {
        pixels: Arc::new(pixels),
        size,
        colour_model,
        thumbnail,
    })
}

/// A sidebar/tab-strip-sized thumbnail (same [`THUMB_PIXELS`] cap `load_image`
/// uses) for pixels that did not come straight from disk — e.g. after Tools
/// ▸ Crop / Adjust Size / Flip change `ImageContent::pixels` in place and
/// need a matching thumbnail rebuilt from the edited image.
pub fn image_thumbnail(pixels: &RgbaImage) -> Arc<RenderImage> {
    let width = THUMB_PIXELS.min(pixels.width().max(1));
    let height = ((width as u64 * pixels.height() as u64) / pixels.width().max(1) as u64).max(1);
    to_render_image(image::imageops::thumbnail(pixels, width, height as u32))
}

/// RGBA ⇄ BGRA in place.
pub fn swap_red_blue(pixels: &mut RgbaImage) {
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
}

pub fn to_render_image(pixels: RgbaImage) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}

pub fn rotate(pixels: &RgbaImage, rotation: Rotation) -> RgbaImage {
    match rotation.quarter_turns() {
        1 => image::imageops::rotate90(pixels),
        2 => image::imageops::rotate180(pixels),
        3 => image::imageops::rotate270(pixels),
        _ => pixels.clone(),
    }
}

/// View ▸ Use Dark Appearance for PDF (PRV-MENU-049): a plain RGB invert
/// of the rendered page, the same low-cost "dark mode" trick many PDF
/// readers use. Preview has no ICC colour-management engine (see Soft
/// Proof's note in docs/parity.md), so this is a pixel filter, not a real
/// colour-managed dark rendering — honest about what it is, but a real,
/// visible effect rather than a menu item that does nothing.
pub fn invert(pixels: &mut RgbaImage) {
    for pixel in pixels.pixels_mut() {
        pixel.0[0] = 255 - pixel.0[0];
        pixel.0[1] = 255 - pixel.0[1];
        pixel.0[2] = 255 - pixel.0[2];
    }
}

/// Tools ▸ Flip Horizontal (PRV-MENU-066): mirrors left-right.
pub fn flip_horizontal(pixels: &RgbaImage) -> RgbaImage {
    image::imageops::flip_horizontal(pixels)
}

/// Tools ▸ Flip Vertical (PRV-MENU-067): mirrors top-bottom.
pub fn flip_vertical(pixels: &RgbaImage) -> RgbaImage {
    image::imageops::flip_vertical(pixels)
}

/// Tools ▸ Adjust Size… (PRV-MENU-054): resizes to an exact new pixel size
/// (not a fit/letterbox). The caller works out width/height, including any
/// aspect-ratio locking; this only resamples.
pub fn adjust_size(pixels: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if width == 0 || height == 0 {
        return RgbaImage::new(width.max(1), height.max(1));
    }
    image::imageops::resize(pixels, width, height, image::imageops::FilterType::Triangle)
}

/// File ▸ Export As… (PRV-MENU-013): re-encodes `pixels` (kept in GPUI's
/// BGRA order) to whatever format `destination`'s extension names, by
/// swapping back to RGBA first. Save As / Save, by contrast, send the
/// original file's own bytes unchanged — this is the one path that
/// actually converts formats.
pub fn export_image(pixels: &RgbaImage, destination: &Path) -> Result<(), String> {
    let mut rgba = pixels.clone();
    swap_red_blue(&mut rgba);
    rgba.save(destination).map_err(|error| error.to_string())
}

/// File ▸ Save / Save As for a loaded image: writes `pixels` to `path` in
/// `kind`'s own format rather than converting it (that is Export As' job,
/// `export_image` above). PNG/TIFF/BMP/WebP are written through this
/// crate's lossless encoders for that format; JPEG is written at a fixed
/// high quality (92, versus the encoder's low 75 default) with alpha
/// flattened onto white like a printed page, mirroring
/// `encode_print_jpeg`. EXIF orientation was already applied to `pixels`
/// when the file was first opened (`load_image`'s `apply_orientation`),
/// and none of these encoders write an EXIF block back, so the saved file
/// needs no orientation tag of its own: its pixels are already the right
/// way up. Does not itself write atomically — `save_image` below wraps
/// this in a temp-file-then-rename for a caller writing straight to the
/// live document path; `save_as`, which already writes to its own
/// not-yet-visible temporary file before a single rename, calls this
/// directly.
pub fn write_image(pixels: &RgbaImage, kind: ImageKind, path: &Path) -> Result<(), String> {
    let mut rgba = pixels.clone();
    swap_red_blue(&mut rgba);
    let (width, height) = rgba.dimensions();
    let raw = rgba.as_raw().as_slice();
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    match kind {
        ImageKind::Jpeg => {
            let mut rgb = image::RgbImage::new(width, height);
            for (x, y, pixel) in rgba.enumerate_pixels() {
                let [red, green, blue, alpha] = pixel.0;
                let blend = |channel: u8| {
                    ((u16::from(channel) * u16::from(alpha) + 255 * (255 - u16::from(alpha))) / 255)
                        as u8
                };
                rgb.put_pixel(x, y, image::Rgb([blend(red), blend(green), blend(blue)]));
            }
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 92)
                .write_image(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
                .map_err(|error| error.to_string())?;
        }
        ImageKind::Png => {
            image::codecs::png::PngEncoder::new(&mut writer)
                .write_image(raw, width, height, image::ExtendedColorType::Rgba8)
                .map_err(|error| error.to_string())?;
        }
        ImageKind::Tiff => {
            image::codecs::tiff::TiffEncoder::new(&mut writer)
                .write_image(raw, width, height, image::ExtendedColorType::Rgba8)
                .map_err(|error| error.to_string())?;
        }
        ImageKind::Bmp => {
            image::codecs::bmp::BmpEncoder::new(&mut writer)
                .write_image(raw, width, height, image::ExtendedColorType::Rgba8)
                .map_err(|error| error.to_string())?;
        }
        ImageKind::Gif => {
            image::codecs::gif::GifEncoder::new(&mut writer)
                .write_image(raw, width, height, image::ExtendedColorType::Rgba8)
                .map_err(|error| error.to_string())?;
        }
        ImageKind::Webp => {
            image::codecs::webp::WebPEncoder::new_lossless(&mut writer)
                .write_image(raw, width, height, image::ExtendedColorType::Rgba8)
                .map_err(|error| error.to_string())?;
        }
    }
    writer.flush().map_err(|error| error.to_string())
}

/// File ▸ Save (⌘S) for an edited image: `write_image` into a sibling temp
/// file, then rename over `destination` — the same temp-file-then-rename
/// atomicity `save_markup` already uses for PDF annotations, so a save that
/// is interrupted (power loss, a full disk) never leaves `destination`
/// half-written.
pub fn save_image(pixels: &RgbaImage, kind: ImageKind, destination: &Path) -> Result<(), String> {
    let temporary = destination.with_extension(format!("lulo-saving-{}.tmp", std::process::id()));
    match write_image(pixels, kind, &temporary) {
        Ok(()) => std::fs::rename(&temporary, destination).map_err(|error| error.to_string()),
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

/// Tools ▸ Crop (PRV-MENU-068): crops to a pixel rectangle, top-left
/// origin, clamped to the image bounds so it can never panic. `x`/`y` are
/// clamped inside the image and `width`/`height` are clamped to what
/// remains from there, with a floor of one pixel either way.
pub fn crop(pixels: &RgbaImage, x: u32, y: u32, width: u32, height: u32) -> RgbaImage {
    let (image_width, image_height) = pixels.dimensions();
    let x = x.min(image_width.saturating_sub(1));
    let y = y.min(image_height.saturating_sub(1));
    let width = width.min(image_width.saturating_sub(x)).max(1);
    let height = height.min(image_height.saturating_sub(y)).max(1);
    image::imageops::crop_imm(pixels, x, y, width, height).to_image()
}

fn run(tool: &str, args: Vec<std::ffi::OsString>) -> Result<Vec<u8>, String> {
    use crate::bounded::{self, RunError};

    let output = bounded::run(
        tool,
        args,
        bounded::TOOL_TIMEOUT,
        bounded::MAX_TOOL_OUTPUT_BYTES,
    )
    .map_err(|error| match error {
        RunError::Start(error) if error.kind() == std::io::ErrorKind::NotFound => {
            poppler::missing_tool_message(tool)
        }
        RunError::Start(error) => format!("{tool} could not start: {error}"),
        RunError::TimedOut => format!("{tool} took too long and was stopped"),
        RunError::TooLarge => format!("{tool} produced more output than Preview allows"),
        RunError::Read(error) => format!("{tool} output could not be read: {error}"),
    })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.lines().next().unwrap_or("").trim();
        return Err(if detail.contains("Incorrect password") {
            "This PDF is password-protected. Preview can’t unlock PDFs yet.".to_owned()
        } else {
            // Poppler's own message names the absolute file path and can carry
            // bytes from the document; only the exit status leaves Preview.
            format!("{tool} failed ({})", output.status)
        });
    }
    Ok(output.stdout)
}

/// Absolute path so a file name starting with “-” is never read as an
/// option by poppler.
fn tool_path(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn load_pdf_info(path: &Path) -> Result<PdfInfo, String> {
    let path = tool_path(path);
    let summary = run(poppler::PDFINFO, poppler::info_args(&path, None))
        .map_err(|error| open_error(&path, &error))?;
    let mut info = poppler::parse_info(&String::from_utf8_lossy(&summary))
        .map_err(|error| open_error(&path, &error))?;
    let count = info.pages.len();
    // Per-page boxes; if poppler rejects the range, every page keeps the
    // document's first page size.
    if let Ok(pages) = run(poppler::PDFINFO, poppler::info_args(&path, Some(count))) {
        if let Ok(detailed) = poppler::parse_info(&String::from_utf8_lossy(&pages)) {
            if detailed.pages.len() == count {
                info.pages = detailed.pages;
            }
        }
    }
    Ok(info)
}

/// Rasterise one page `pixel_width` device pixels wide (BGRA, rotated).
pub fn render_page(
    path: &Path,
    page: usize,
    page_size: (f32, f32),
    pixel_scale: f32,
    rotation: Rotation,
) -> Result<RgbaImage, String> {
    let dpi = poppler::render_dpi(page_size, pixel_scale);
    let output = run(
        poppler::PDFTOPPM,
        poppler::render_args(&tool_path(path), page, dpi),
    )?;
    let (width, height, raster) =
        poppler::parse_ppm(&output).ok_or_else(|| "pdftoppm returned no page".to_owned())?;
    let mut bgra = Vec::with_capacity(raster.len() / 3 * 4);
    for rgb in raster.chunks_exact(3) {
        bgra.extend_from_slice(&[rgb[2], rgb[1], rgb[0], 0xFF]);
    }
    let pixels =
        RgbaImage::from_raw(width, height, bgra).ok_or_else(|| "bad page bitmap".to_owned())?;
    Ok(rotate(&pixels, rotation))
}

pub fn extract_text(path: &Path) -> Result<Vec<TextPage>, String> {
    let output = run(poppler::PDFTOTEXT, poppler::text_args(&tool_path(path)))?;
    Ok(poppler::parse_bbox(&String::from_utf8_lossy(&output)))
}

/// Encode BGRA pixels as PNG bytes for the clipboard.
pub fn encode_png(pixels: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut rgba = pixels.clone();
    swap_red_blue(&mut rgba);
    let mut bytes = Vec::new();
    rgba.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Encode the currently displayed image as a JPEG suitable for embedding in
/// a one-page PDF. Resize before rotating so the temporary print raster stays
/// bounded, then flatten transparency onto white as a printed page does.
pub fn encode_print_jpeg(
    pixels: &RgbaImage,
    rotation: Rotation,
) -> Result<(Vec<u8>, u32, u32), String> {
    if pixels.width() == 0 || pixels.height() == 0 {
        return Err("the image has no pixels".into());
    }
    let scale = (f64::from(MAX_PRINT_IMAGE_SIDE) / f64::from(pixels.width()))
        .min(f64::from(MAX_PRINT_IMAGE_SIDE) / f64::from(pixels.height()))
        .min(1.0);
    let width = (f64::from(pixels.width()) * scale).round().max(1.0) as u32;
    let height = (f64::from(pixels.height()) * scale).round().max(1.0) as u32;
    let bounded = if scale < 1.0 {
        image::imageops::resize(pixels, width, height, image::imageops::FilterType::Triangle)
    } else {
        pixels.clone()
    };
    let rotated = rotate(&bounded, rotation);
    let (width, height) = rotated.dimensions();
    let mut rgb = image::RgbImage::new(width, height);
    for (x, y, pixel) in rotated.enumerate_pixels() {
        // Preview's render pixels are BGRA; PDF JPEGs are RGB. Composite
        // translucent image pixels on white paper.
        let [blue, green, red, alpha] = pixel.0;
        let blend = |channel: u8| {
            ((u16::from(channel) * u16::from(alpha) + 255 * (255 - u16::from(alpha))) / 255) as u8
        };
        rgb.put_pixel(x, y, image::Rgb([blend(red), blend(green), blend(blue)]));
    }
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90)
        .write_image(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .map_err(|error| error.to_string())?;
    Ok((bytes, width, height))
}

/// Image files beside `path`, in Finder order.
pub fn folder_images(path: &Path) -> Vec<PathBuf> {
    let Some(directory) = path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let names = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    document::folder_images(names)
        .into_iter()
        .map(|name| directory.join(name))
        .collect()
}

#[cfg(test)]
mod print_tests {
    use super::*;
    use image::GenericImageView as _;

    #[test]
    fn print_jpeg_preserves_rotation_and_bounds_dimensions() {
        let pixels = RgbaImage::from_pixel(5000, 1, image::Rgba([0, 0, 255, 255]));
        let (jpeg, width, height) = encode_print_jpeg(&pixels, Rotation::from_degrees(90)).unwrap();
        assert_eq!((width, height), (1, MAX_PRINT_IMAGE_SIDE));

        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(decoded.dimensions(), (width, height));
        let pixel = decoded.to_rgb8().get_pixel(0, height / 2).0;
        assert!(pixel[0] > 240 && pixel[1] < 20 && pixel[2] < 20);
    }

    #[test]
    fn print_jpeg_flattens_transparency_onto_white() {
        let pixels = RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 255, 0]));
        let (jpeg, _, _) = encode_print_jpeg(&pixels, Rotation::default()).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        assert!(decoded
            .get_pixel(0, 0)
            .0
            .iter()
            .all(|channel| *channel > 240));
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    /// A 2×2 image with a distinct colour in each corner: red top-left,
    /// green top-right, blue bottom-left, white bottom-right.
    fn swatch() -> RgbaImage {
        let mut pixels = RgbaImage::new(2, 2);
        pixels.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        pixels.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        pixels.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        pixels.put_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        pixels
    }

    #[test]
    fn flip_horizontal_mirrors_left_and_right() {
        let flipped = flip_horizontal(&swatch());
        assert_eq!(flipped.get_pixel(0, 0).0, [0, 255, 0, 255]); // was top-right
        assert_eq!(flipped.get_pixel(1, 0).0, [255, 0, 0, 255]); // was top-left
        assert_eq!(flipped.get_pixel(0, 1).0, [255, 255, 255, 255]); // was bottom-right
        assert_eq!(flipped.get_pixel(1, 1).0, [0, 0, 255, 255]); // was bottom-left
    }

    #[test]
    fn flip_vertical_mirrors_top_and_bottom() {
        let flipped = flip_vertical(&swatch());
        assert_eq!(flipped.get_pixel(0, 0).0, [0, 0, 255, 255]); // was bottom-left
        assert_eq!(flipped.get_pixel(1, 0).0, [255, 255, 255, 255]); // was bottom-right
        assert_eq!(flipped.get_pixel(0, 1).0, [255, 0, 0, 255]); // was top-left
        assert_eq!(flipped.get_pixel(1, 1).0, [0, 255, 0, 255]); // was top-right
    }

    #[test]
    fn adjust_size_resamples_to_the_requested_pixel_size() {
        let resized = adjust_size(&swatch(), 10, 4);
        assert_eq!(resized.dimensions(), (10, 4));
    }

    #[test]
    fn crop_returns_the_requested_rectangle_and_clamps_without_panicking() {
        let image = swatch();
        let cropped = crop(&image, 1, 0, 1, 2);
        assert_eq!(cropped.dimensions(), (1, 2));
        assert_eq!(cropped.get_pixel(0, 0).0, [0, 255, 0, 255]);
        // A rectangle starting past the image's far edge clamps down to a
        // single pixel instead of panicking.
        let far_edge = crop(&image, 5, 5, 3, 3);
        assert_eq!(far_edge.dimensions(), (1, 1));
        // A rectangle wider than the image clamps instead of panicking.
        let clamped = crop(&image, 0, 0, 100, 100);
        assert_eq!(clamped.dimensions(), (2, 2));
    }

    /// A unique path per test: the process id makes it unique across test
    /// *binaries*, but within one binary `cargo test`'s default thread
    /// pool runs every `#[test]` fn concurrently, and `save_image` itself
    /// derives its intermediate temp file from `destination`'s own stem
    /// (`Path::with_extension`, which drops whatever extension `name` ends
    /// in) — so `name` must give each caller a distinct *stem*, not just a
    /// distinct extension, or two tests racing on that shared temp file is
    /// exactly the bug this suite is here to catch, not a flake to retry.
    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-preview-save-test-{}-{name}",
            std::process::id()
        ))
    }

    /// File ▸ Save for a PNG: lossless, so reloading gives back the exact
    /// edited pixels (bytes identical, not just visually close).
    #[test]
    fn save_image_round_trips_png_losslessly() {
        let path = temp_path("roundtrip-png.png");
        let edited = rotate(&swatch(), Rotation::from_degrees(90));
        save_image(&edited, ImageKind::Png, &path).unwrap();
        let reloaded = load_image(&path).unwrap();
        assert_eq!(reloaded.pixels.dimensions(), edited.dimensions());
        assert_eq!(*reloaded.pixels, edited);
        let _ = std::fs::remove_file(&path);
    }

    /// TIFF and BMP are written through this crate's own lossless
    /// encoders too.
    #[test]
    fn save_image_round_trips_tiff_and_bmp_losslessly() {
        for (kind, name) in [
            (ImageKind::Tiff, "roundtrip-tiff.tiff"),
            (ImageKind::Bmp, "roundtrip-bmp.bmp"),
        ] {
            let path = temp_path(name);
            let edited = crop(&swatch(), 0, 0, 2, 1);
            save_image(&edited, kind, &path).unwrap();
            let reloaded = load_image(&path).unwrap();
            assert_eq!(*reloaded.pixels, edited, "{name}");
            let _ = std::fs::remove_file(&path);
        }
    }

    /// JPEG is written at a fixed high quality rather than the encoder's
    /// low 75 default, and flattens transparency onto white like a printed
    /// page; a solid, opaque swatch should survive the round trip within a
    /// few levels of JPEG's own lossy compression.
    #[test]
    fn save_image_writes_jpeg_at_high_quality() {
        let path = temp_path("roundtrip-jpg.jpg");
        // BGRA (this module's convention): blue 200, green/red 40.
        let solid = RgbaImage::from_pixel(4, 4, image::Rgba([200, 40, 40, 255]));
        save_image(&solid, ImageKind::Jpeg, &path).unwrap();
        let reloaded = load_image(&path).unwrap();
        assert_eq!(reloaded.pixels.dimensions(), (4, 4));
        let pixel = reloaded.pixels.get_pixel(0, 0).0;
        assert!((i32::from(pixel[0]) - 200).abs() < 10, "{pixel:?}");
        assert!((i32::from(pixel[1]) - 40).abs() < 10, "{pixel:?}");
        assert!((i32::from(pixel[2]) - 40).abs() < 10, "{pixel:?}");
        let _ = std::fs::remove_file(&path);
    }

    /// A failed encode (an unwritable destination directory) must not leave
    /// its temp file behind.
    #[test]
    fn save_image_cleans_up_its_temp_file_on_failure() {
        let path = PathBuf::from("/nonexistent-rmac-preview-dir/roundtrip.png");
        assert!(save_image(&swatch(), ImageKind::Png, &path).is_err());
        let temporary = path.with_extension(format!("lulo-saving-{}.tmp", std::process::id()));
        assert!(!temporary.exists());
    }
}
