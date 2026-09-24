//! Blocking document work that runs on background threads: recognising and
//! decoding files, poppler subprocesses, rotation and clipboard encoding.
//! Pixels are kept in GPUI's BGRA order so they become textures unchanged.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::SystemTime;

use crate::document::{self, Kind};
use crate::layout::Rotation;
use crate::poppler::{self, PdfInfo, TextPage};
use gpui::RenderImage;
use image::{ImageDecoder as _, ImageReader, RgbaImage};

/// Longest image side kept in memory; larger images are downsampled once so
/// a single texture stays within what low-end GPUs accept.
const MAX_IMAGE_SIDE: u32 = 8192;
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
    let thumbnail = image::imageops::thumbnail(
        &pixels,
        THUMB_PIXELS.min(pixels.width()),
        ((THUMB_PIXELS.min(pixels.width()) as u64 * pixels.height() as u64)
            / pixels.width().max(1) as u64)
            .max(1) as u32,
    );
    Ok(ImageContent {
        pixels: Arc::new(pixels),
        size,
        colour_model,
        thumbnail: to_render_image(thumbnail),
    })
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

fn run(tool: &str, args: Vec<std::ffi::OsString>) -> Result<Vec<u8>, String> {
    let output = Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => poppler::missing_tool_message(tool),
            _ => format!("{tool} could not start: {error}"),
        })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.lines().next().unwrap_or("").trim();
        return Err(if detail.contains("Incorrect password") {
            "This PDF is password-protected. Preview can’t unlock PDFs yet.".to_owned()
        } else if detail.is_empty() {
            format!("{tool} failed ({})", output.status)
        } else {
            detail.to_owned()
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
