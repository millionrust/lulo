use std::fmt;
use std::sync::Arc;

pub const MAX_ICON_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ICON_SOURCE_DIMENSION: u32 = 8_192;
pub const MAX_ICON_SOURCE_PIXELS: u64 = 16_777_216;
pub const MAX_ICON_DECODE_BYTES: u64 = 80 * 1024 * 1024;
pub const MAX_ICON_EDGE: u32 = 512;
pub const DEFAULT_ICON_CACHE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_CACHED_ICONS: usize = 256;
pub(crate) const MAX_SVG_ELEMENTS: usize = 4_096;
pub(crate) const MAX_SVG_ATTRIBUTES: usize = 16_384;
pub(crate) const MAX_SVG_DEPTH: usize = 64;
pub(crate) const MAX_SVG_COORDINATE: f32 = 16_384.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFormat {
    Png,
    Jpeg,
    Svg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeRequest {
    pub(crate) edge: u32,
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
    pub(crate) fn new(kind: ErrorKind) -> Self {
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
    pub(crate) edge: u32,
    pub(crate) format: SourceFormat,
    pub(crate) rgba: Arc<[u8]>,
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
