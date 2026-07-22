//! Shared bounded external icon decoding for rmac renderers.
//!
//! Desktop-entry icon paths are untrusted filesystem input. Decoding is
//! synchronous and potentially CPU intensive, so callers must run it on a
//! dedicated worker rather than the GPUI thread.

use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::hash::Hash;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use image::imageops::FilterType;
use image::{DynamicImage, ImageError, ImageFormat, ImageReader};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

pub const MAX_ICON_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ICON_SOURCE_DIMENSION: u32 = 8_192;
pub const MAX_ICON_SOURCE_PIXELS: u64 = 16_777_216;
pub const MAX_ICON_DECODE_BYTES: u64 = 80 * 1024 * 1024;
pub const MAX_ICON_EDGE: u32 = 512;
pub const DEFAULT_ICON_CACHE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_CACHED_ICONS: usize = 256;
const MAX_SVG_ELEMENTS: usize = 4_096;
const MAX_SVG_ATTRIBUTES: usize = 16_384;
const MAX_SVG_DEPTH: usize = 64;
const MAX_SVG_COORDINATE: f32 = 16_384.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFormat {
    Png,
    Jpeg,
    Svg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeRequest {
    edge: u32,
}

impl DecodeRequest {
    pub fn new(edge: u32) -> Result<Self, Error> {
        if edge == 0 || edge > MAX_ICON_EDGE {
            Err(Error::new(ErrorKind::InvalidRequest))
        } else {
            Ok(Self { edge })
        }
    }

    pub fn edge(self) -> u32 {
        self.edge
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    InvalidRequest,
    Io(std::io::ErrorKind),
    TooLarge,
    Unsupported,
    Malformed,
    UnsafeSvg,
    Empty,
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    kind: ErrorKind,
}

impl Error {
    fn new(kind: ErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ErrorKind::InvalidRequest => "the requested icon size is invalid",
            ErrorKind::Io(_) => "the icon file could not be read",
            ErrorKind::TooLarge => "the icon exceeds a safety limit",
            ErrorKind::Unsupported => "the icon format is unsupported",
            ErrorKind::Malformed => "the icon data is malformed",
            ErrorKind::UnsafeSvg => "the SVG uses a disabled feature",
            ErrorKind::Empty => "the icon contains no visible pixels",
            ErrorKind::Changed => "the icon changed while it was being decoded",
        })
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Eq, PartialEq)]
pub struct DecodedIcon {
    edge: u32,
    format: SourceFormat,
    rgba: Arc<[u8]>,
}

impl DecodedIcon {
    pub fn edge(&self) -> u32 {
        self.edge
    }

    pub fn format(&self) -> SourceFormat {
        self.format
    }

    /// Exact square, row-major, non-premultiplied RGBA8 pixels.
    pub fn rgba(&self) -> &Arc<[u8]> {
        &self.rgba
    }
}

impl fmt::Debug for DecodedIcon {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedIcon")
            .field("edge", &self.edge)
            .field("format", &self.format)
            .field("rgba_bytes", &self.rgba.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub decodes: u64,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct FileIdentity {
    byte_len: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    modified_seconds: i64,
    #[cfg(unix)]
    modified_nanoseconds: i64,
    #[cfg(not(unix))]
    modified_nanoseconds: Option<u128>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct CacheKey {
    path: PathBuf,
    identity: FileIdentity,
    edge: u32,
}

struct CacheEntry {
    icon: Arc<DecodedIcon>,
    last_used: u64,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<CacheKey, CacheEntry>,
    sequence: u64,
    bytes: usize,
    decodes: u64,
}

/// Thread-safe, byte- and entry-bounded LRU cache for worker-side decoding.
///
/// File identity is checked before and after every miss. Decodes are serialized
/// so even accidental concurrent callers cannot multiply the decoder's bounded
/// allocation. Paths are retained only inside the bounded private key set.
pub struct Cache {
    state: Mutex<CacheState>,
    decode_lock: Mutex<()>,
    byte_budget: usize,
}

impl Cache {
    pub fn new(byte_budget: usize) -> Self {
        Self {
            state: Mutex::new(CacheState::default()),
            decode_lock: Mutex::new(()),
            byte_budget,
        }
    }

    pub fn get_or_decode(
        &self,
        path: &Path,
        request: DecodeRequest,
    ) -> Result<Arc<DecodedIcon>, Error> {
        let _decoder = self
            .decode_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = file_identity(path)?;
        let key = CacheKey {
            path: path.to_path_buf(),
            identity: before.clone(),
            edge: request.edge,
        };
        if let Some(icon) = self.lookup(&key) {
            return Ok(icon);
        }

        let decoded = Arc::new(decode_file(path, request)?);
        if file_identity(path)? != before {
            return Err(Error::new(ErrorKind::Changed));
        }
        let bytes = decoded.rgba.len();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.decodes = state.decodes.saturating_add(1);
        state
            .entries
            .retain(|candidate, _| candidate.path != path || candidate.identity == before);
        state.bytes = state
            .entries
            .values()
            .map(|entry| entry.icon.rgba.len())
            .sum();
        if self.byte_budget == 0 || bytes > self.byte_budget {
            return Ok(decoded);
        }
        state.sequence = state.sequence.saturating_add(1);
        let last_used = state.sequence;
        if let Some(previous) = state.entries.insert(
            key,
            CacheEntry {
                icon: decoded.clone(),
                last_used,
            },
        ) {
            state.bytes = state.bytes.saturating_sub(previous.icon.rgba.len());
        }
        state.bytes = state.bytes.saturating_add(bytes);
        while state.bytes > self.byte_budget || state.entries.len() > MAX_CACHED_ICONS {
            let Some(oldest) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(removed) = state.entries.remove(&oldest) {
                state.bytes = state.bytes.saturating_sub(removed.icon.rgba.len());
            }
        }
        Ok(decoded)
    }

    /// Remove every rendered size for one private source path.
    pub fn invalidate_path(&self, path: &Path) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.entries.retain(|key, _| key.path != path);
        state.bytes = state
            .entries
            .values()
            .map(|entry| entry.icon.rgba.len())
            .sum();
    }

    pub fn clear(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.entries.clear();
        state.bytes = 0;
    }

    pub fn stats(&self) -> CacheStats {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CacheStats {
            entries: state.entries.len(),
            bytes: state.bytes,
            decodes: state.decodes,
        }
    }

    fn lookup(&self, key: &CacheKey) -> Option<Arc<DecodedIcon>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.sequence = state.sequence.saturating_add(1);
        let last_used = state.sequence;
        let entry = state.entries.get_mut(key)?;
        entry.last_used = last_used;
        Some(entry.icon.clone())
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(DEFAULT_ICON_CACHE_BYTES)
    }
}

impl fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cache")
            .field("byte_budget", &self.byte_budget)
            .field("stats", &self.stats())
            .finish()
    }
}

fn file_identity(path: &Path) -> Result<FileIdentity, Error> {
    let metadata = std::fs::metadata(path).map_err(io_error)?;
    if !metadata.is_file() {
        return Err(Error::new(ErrorKind::Unsupported));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        Ok(FileIdentity {
            byte_len: metadata.len(),
            device: metadata.dev(),
            inode: metadata.ino(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
        })
    }
    #[cfg(not(unix))]
    {
        use std::time::UNIX_EPOCH;

        Ok(FileIdentity {
            byte_len: metadata.len(),
            modified_nanoseconds: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos()),
        })
    }
}

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

fn io_error(error: std::io::Error) -> Error {
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

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::sync::{Arc, Barrier};
    use std::time::{SystemTime, UNIX_EPOCH};

    use image::ImageEncoder as _;

    use super::*;

    fn request(edge: u32) -> DecodeRequest {
        DecodeRequest::new(edge).unwrap()
    }

    fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    fn jpeg(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90)
            .write_image(rgb, width, height, image::ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    fn root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("rmac-icon-{label}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn request_and_input_limits_fail_before_allocation() {
        assert_eq!(
            DecodeRequest::new(0).unwrap_err().kind(),
            ErrorKind::InvalidRequest
        );
        assert_eq!(
            DecodeRequest::new(MAX_ICON_EDGE + 1).unwrap_err().kind(),
            ErrorKind::InvalidRequest
        );
        assert_eq!(
            decode_bytes(&vec![b'x'; MAX_ICON_FILE_BYTES + 1], request(64))
                .unwrap_err()
                .kind(),
            ErrorKind::TooLarge
        );
    }

    #[test]
    fn png_is_detected_from_bytes_and_fitted_into_exact_square_rgba() {
        let bytes = png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]);
        let decoded = decode_bytes(&bytes, request(8)).unwrap();
        assert_eq!(decoded.edge(), 8);
        assert_eq!(decoded.format(), SourceFormat::Png);
        assert_eq!(decoded.rgba().len(), 8 * 8 * 4);
        assert!(decoded.rgba()[..8 * 2 * 4]
            .chunks_exact(4)
            .all(|pixel| pixel[3] == 0));
        assert!(decoded.rgba()[8 * 2 * 4..8 * 6 * 4]
            .chunks_exact(4)
            .any(|pixel| pixel[3] == 255));
    }

    #[test]
    fn png_decoder_enforces_source_dimensions() {
        let width = MAX_ICON_SOURCE_DIMENSION + 1;
        let bytes = png(width, 1, &vec![255; width as usize * 4]);
        assert_eq!(
            decode_bytes(&bytes, request(32)).unwrap_err().kind(),
            ErrorKind::TooLarge
        );
    }

    #[test]
    fn jpeg_is_detected_from_bytes_and_decoded_to_opaque_rgba() {
        let bytes = jpeg(2, 1, &[255, 0, 0, 0, 255, 0]);
        let decoded = decode_bytes(&bytes, request(8)).unwrap();
        assert_eq!(decoded.edge(), 8);
        assert_eq!(decoded.format(), SourceFormat::Jpeg);
        assert_eq!(decoded.rgba().len(), 8 * 8 * 4);
        assert!(decoded
            .rgba()
            .chunks_exact(4)
            .filter(|pixel| pixel[3] != 0)
            .all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn svg_renders_centered_straight_rgba_without_external_resources() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000" fill-opacity="0.5"/></svg>"##;
        let decoded = decode_bytes(svg, request(20)).unwrap();
        assert_eq!(decoded.format(), SourceFormat::Svg);
        assert_eq!(decoded.rgba().len(), 20 * 20 * 4);
        let top = &decoded.rgba()[0..20 * 5 * 4];
        assert!(top.chunks_exact(4).all(|pixel| pixel[3] == 0));
        let center = &decoded.rgba()[(10 * 20 + 10) * 4..][..4];
        assert!(center[0] >= 254);
        assert_eq!(center[1], 0);
        assert!((127..=128).contains(&center[3]));
    }

    #[test]
    fn svg_rejects_active_external_and_expansion_features() {
        let cases: [&[u8]; 6] = [
            br#"<!DOCTYPE svg><svg xmlns="http://www.w3.org/2000/svg"/>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="/private/file"/></svg>"#,
            br##"<svg xmlns="http://www.w3.org/2000/svg"><use href="#x"/></svg>"##,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><filter/></svg>"#,
            br#"<svg xmlns="http://www.w3.org/2000/svg"><rect onclick="x()"/></svg>"#,
        ];
        for bytes in cases {
            assert_eq!(
                decode_bytes(bytes, request(32)).unwrap_err().kind(),
                ErrorKind::UnsafeSvg
            );
        }
    }

    #[test]
    fn malformed_unsupported_and_invisible_inputs_are_distinct() {
        assert_eq!(
            decode_bytes(b"not an icon", request(32))
                .unwrap_err()
                .kind(),
            ErrorKind::Malformed
        );
        assert_eq!(
            decode_bytes(&[0xff, 0xfe], request(32)).unwrap_err().kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(
            decode_bytes(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#,
                request(32)
            )
            .unwrap_err()
            .kind(),
            ErrorKind::Empty
        );
    }

    #[test]
    fn file_boundary_is_bounded_and_diagnostics_do_not_expose_path_or_pixels() {
        let root = root("private");
        let path = root.join("secret-app-icon.png");
        let bytes = png(1, 1, &[12, 34, 56, 255]);
        File::create(&path).unwrap().write_all(&bytes).unwrap();

        let decoded = decode_file(&path, request(4)).unwrap();
        let debug = format!("{decoded:?}");
        assert!(!debug.contains("secret-app-icon"));
        assert!(!debug.contains("12, 34, 56"));
        assert!(debug.contains("rgba_bytes: 64"));

        let missing = root.join("private-missing-icon.svg");
        let error = decode_file(&missing, request(4)).unwrap_err();
        let diagnostics = format!("{error:?} {error}");
        assert!(!diagnostics.contains("private-missing-icon"));
        assert_eq!(error.kind(), ErrorKind::Io(std::io::ErrorKind::NotFound));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_hits_then_invalidates_an_atomically_replaced_file() {
        let root = root("cache-change");
        let path = root.join("private-theme-icon.png");
        std::fs::write(&path, png(1, 1, &[10, 20, 30, 255])).unwrap();
        let cache = Cache::default();

        let first = cache.get_or_decode(&path, request(8)).unwrap();
        let again = cache.get_or_decode(&path, request(8)).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(cache.stats().decodes, 1);

        let replacement = root.join("replacement.png");
        std::fs::write(&replacement, png(2, 1, &[90, 80, 70, 255, 60, 50, 40, 255])).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let changed = cache.get_or_decode(&path, request(8)).unwrap();
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(cache.stats().decodes, 2);
        assert_eq!(cache.stats().entries, 1);

        cache.invalidate_path(&path);
        assert_eq!(cache.stats().entries, 0);
        assert!(!format!("{cache:?}").contains("private-theme-icon"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_enforces_byte_budget_with_lru_eviction() {
        let root = root("cache-budget");
        let first_path = root.join("first.png");
        let second_path = root.join("second.png");
        std::fs::write(&first_path, png(1, 1, &[1, 2, 3, 255])).unwrap();
        std::fs::write(&second_path, png(1, 1, &[4, 5, 6, 255])).unwrap();
        let cache = Cache::new(4 * 4 * 4);

        cache.get_or_decode(&first_path, request(4)).unwrap();
        cache.get_or_decode(&second_path, request(4)).unwrap();
        assert_eq!(cache.stats().entries, 1);
        assert_eq!(cache.stats().bytes, 64);
        assert_eq!(cache.stats().decodes, 2);
        cache.get_or_decode(&first_path, request(4)).unwrap();
        assert_eq!(cache.stats().decodes, 3);
        assert_eq!(cache.stats().entries, 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_misses_are_coalesced_into_one_bounded_decode() {
        let root = root("cache-coalesce");
        let path = root.join("shared.png");
        std::fs::write(&path, png(1, 1, &[7, 8, 9, 255])).unwrap();
        let cache = Arc::new(Cache::default());
        let barrier = Arc::new(Barrier::new(5));
        let mut threads = Vec::new();
        for _ in 0..4 {
            let cache = cache.clone();
            let barrier = barrier.clone();
            let path = path.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                cache.get_or_decode(&path, request(16)).unwrap()
            }));
        }
        barrier.wait();
        let icons = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert!(icons
            .iter()
            .skip(1)
            .all(|icon| Arc::ptr_eq(&icons[0], icon)));
        assert_eq!(cache.stats().decodes, 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
