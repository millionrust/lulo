use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use image::imageops::FilterType;
use image::{DynamicImage, ImageError, ImageFormat, ImageReader};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::{
    DecodeRequest, DecodedIcon, Error, ErrorKind, SourceFormat, MAX_ICON_DECODE_BYTES,
    MAX_ICON_FILE_BYTES, MAX_ICON_SOURCE_DIMENSION, MAX_ICON_SOURCE_PIXELS, MAX_SVG_ATTRIBUTES,
    MAX_SVG_COORDINATE, MAX_SVG_DEPTH, MAX_SVG_ELEMENTS,
};

/// Read and decode one external icon without retaining or reporting its path.
pub fn decode_file(path: &Path, request: DecodeRequest) -> Result<DecodedIcon, Error> {
    let mut file = open_nonblocking(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err(Error::new(ErrorKind::Unsupported));
    }
    if metadata.len() > MAX_ICON_FILE_BYTES as u64 {
        return Err(Error::new(ErrorKind::TooLarge));
    }

    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(MAX_ICON_FILE_BYTES)
            .min(MAX_ICON_FILE_BYTES),
    );
    file.by_ref()
        .take(MAX_ICON_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > MAX_ICON_FILE_BYTES {
        return Err(Error::new(ErrorKind::TooLarge));
    }
    decode_bytes(&bytes, request)
}

/// Decode bounded in-memory icon data. The format is detected from content,
/// never from a potentially misleading desktop-entry filename.
pub fn decode_bytes(bytes: &[u8], request: DecodeRequest) -> Result<DecodedIcon, Error> {
    if bytes.is_empty() {
        return Err(Error::new(ErrorKind::Malformed));
    }
    if bytes.len() > MAX_ICON_FILE_BYTES {
        return Err(Error::new(ErrorKind::TooLarge));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_raster(bytes, request, ImageFormat::Png, SourceFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        decode_raster(bytes, request, ImageFormat::Jpeg, SourceFormat::Jpeg)
    } else {
        let text = std::str::from_utf8(bytes).map_err(|_| Error::new(ErrorKind::Unsupported))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        decode_svg(text, request)
    }
}

#[cfg(unix)]
fn open_nonblocking(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_nonblocking(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

pub(crate) fn io_error(error: std::io::Error) -> Error {
    Error::new(ErrorKind::Io(error.kind()))
}

fn decode_raster(
    bytes: &[u8],
    request: DecodeRequest,
    image_format: ImageFormat,
    source_format: SourceFormat,
) -> Result<DecodedIcon, Error> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_ICON_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_ICON_SOURCE_DIMENSION);
    limits.max_alloc = Some(MAX_ICON_DECODE_BYTES);
    reader.limits(limits);
    let decoded = reader.decode().map_err(map_image_error)?;
    validate_source_dimensions(decoded.width(), decoded.height())?;
    raster_to_square(decoded, request, source_format)
}

fn map_image_error(error: ImageError) -> Error {
    match error {
        ImageError::Limits(_) => Error::new(ErrorKind::TooLarge),
        _ => Error::new(ErrorKind::Malformed),
    }
}

fn validate_source_dimensions(width: u32, height: u32) -> Result<(), Error> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| Error::new(ErrorKind::TooLarge))?;
    if width == 0
        || height == 0
        || width > MAX_ICON_SOURCE_DIMENSION
        || height > MAX_ICON_SOURCE_DIMENSION
        || pixels > MAX_ICON_SOURCE_PIXELS
    {
        Err(Error::new(ErrorKind::TooLarge))
    } else {
        Ok(())
    }
}

fn raster_to_square(
    decoded: DynamicImage,
    request: DecodeRequest,
    format: SourceFormat,
) -> Result<DecodedIcon, Error> {
    let source = decoded.into_rgba8();
    let (width, height) = fit_dimensions(source.width(), source.height(), request.edge);
    let resized = image::imageops::resize(&source, width, height, FilterType::Lanczos3);
    let output_len = output_len(request.edge)?;
    let mut rgba = vec![0; output_len];
    let left = (request.edge - width) / 2;
    let top = (request.edge - height) / 2;
    for y in 0..height {
        let source_start = usize::try_from(u64::from(y) * u64::from(width) * 4)
            .map_err(|_| Error::new(ErrorKind::TooLarge))?;
        let destination_start =
            usize::try_from((u64::from(top + y) * u64::from(request.edge) + u64::from(left)) * 4)
                .map_err(|_| Error::new(ErrorKind::TooLarge))?;
        let row_len =
            usize::try_from(u64::from(width) * 4).map_err(|_| Error::new(ErrorKind::TooLarge))?;
        rgba[destination_start..destination_start + row_len]
            .copy_from_slice(&resized.as_raw()[source_start..source_start + row_len]);
    }
    finish(request, format, rgba)
}

fn fit_dimensions(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let scale = (edge as f64 / f64::from(width)).min(edge as f64 / f64::from(height));
    let fitted_width = (f64::from(width) * scale).round().clamp(1.0, edge as f64) as u32;
    let fitted_height = (f64::from(height) * scale).round().clamp(1.0, edge as f64) as u32;
    (fitted_width, fitted_height)
}

fn decode_svg(text: &str, request: DecodeRequest) -> Result<DecodedIcon, Error> {
    preflight_svg(text)?;
    let options = resvg::usvg::Options {
        resources_dir: None,
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_str(text, &options)
        .map_err(|_| Error::new(ErrorKind::Malformed))?;
    let width = tree.size().width();
    let height = tree.size().height();
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > MAX_SVG_COORDINATE
        || height > MAX_SVG_COORDINATE
    {
        return Err(Error::new(ErrorKind::TooLarge));
    }

    let mut pixmap = resvg::tiny_skia::Pixmap::new(request.edge, request.edge)
        .ok_or_else(|| Error::new(ErrorKind::TooLarge))?;
    let scale = (request.edge as f32 / width).min(request.edge as f32 / height);
    let left = (request.edge as f32 - width * scale) / 2.0;
    let top = (request.edge as f32 - height * scale) / 2.0;
    let transform = resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, left, top);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    unpremultiply(&mut rgba);
    finish(request, SourceFormat::Svg, rgba)
}

fn preflight_svg(text: &str) -> Result<(), Error> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut elements = 0_usize;
    let mut attributes = 0_usize;
    let mut depth = 0_usize;
    let mut root_seen = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                inspect_svg_element(&start, &mut elements, &mut attributes, &mut root_seen)?;
                depth = depth.saturating_add(1);
                if depth > MAX_SVG_DEPTH {
                    return Err(Error::new(ErrorKind::TooLarge));
                }
            }
            Ok(Event::Empty(start)) => {
                inspect_svg_element(&start, &mut elements, &mut attributes, &mut root_seen)?;
            }
            Ok(Event::End(_)) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::new(ErrorKind::Malformed))?;
            }
            Ok(Event::DocType(_) | Event::PI(_)) => {
                return Err(Error::new(ErrorKind::UnsafeSvg));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(Error::new(ErrorKind::Malformed)),
        }
    }
    if !root_seen || depth != 0 {
        return Err(Error::new(ErrorKind::Malformed));
    }
    Ok(())
}

fn inspect_svg_element(
    start: &BytesStart<'_>,
    elements: &mut usize,
    attributes: &mut usize,
    root_seen: &mut bool,
) -> Result<(), Error> {
    *elements = elements.saturating_add(1);
    if *elements > MAX_SVG_ELEMENTS {
        return Err(Error::new(ErrorKind::TooLarge));
    }
    let name = start.local_name();
    let name = name.as_ref();
    if !*root_seen {
        if name != b"svg" {
            return Err(Error::new(ErrorKind::Unsupported));
        }
        *root_seen = true;
    }
    if matches!(
        name,
        b"script" | b"image" | b"foreignObject" | b"filter" | b"use"
    ) || name.starts_with(b"fe")
    {
        return Err(Error::new(ErrorKind::UnsafeSvg));
    }
    for attribute in start.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| Error::new(ErrorKind::Malformed))?;
        *attributes = attributes.saturating_add(1);
        if *attributes > MAX_SVG_ATTRIBUTES {
            return Err(Error::new(ErrorKind::TooLarge));
        }
        if attribute.key.local_name().as_ref().starts_with(b"on") {
            return Err(Error::new(ErrorKind::UnsafeSvg));
        }
    }
    Ok(())
}

fn output_len(edge: u32) -> Result<usize, Error> {
    usize::try_from(u64::from(edge) * u64::from(edge) * 4)
        .map_err(|_| Error::new(ErrorKind::TooLarge))
}

fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
}

fn finish(
    request: DecodeRequest,
    format: SourceFormat,
    rgba: Vec<u8>,
) -> Result<DecodedIcon, Error> {
    if rgba.len() != output_len(request.edge)? {
        return Err(Error::new(ErrorKind::Malformed));
    }
    if !rgba.chunks_exact(4).any(|pixel| pixel[3] != 0) {
        return Err(Error::new(ErrorKind::Empty));
    }
    Ok(DecodedIcon {
        edge: request.edge,
        format,
        rgba: Arc::from(rgba),
    })
}
