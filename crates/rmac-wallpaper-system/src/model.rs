use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const MAX_WALLPAPER_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    RelativePath,
    UnnormalizedPath,
    Io(std::io::ErrorKind),
    NotRegularFile,
    Empty,
    TooLarge,
    UnsupportedFormat,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub(crate) detail: String,
}

impl Error {
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Could not load the wallpaper file")
    }
}

impl std::error::Error for Error {}

pub struct FileAsset {
    file: std::fs::File,
    canonical_path: PathBuf,
    pub byte_len: u64,
    pub modified: Option<SystemTime>,
    pub format: ImageFormat,
}

impl fmt::Debug for FileAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileAsset")
            .field("path", &"<private>")
            .field("byte_len", &self.byte_len)
            .field("modified", &self.modified)
            .field("format", &self.format)
            .finish()
    }
}

impl FileAsset {
    pub(crate) fn new(
        file: std::fs::File,
        canonical_path: PathBuf,
        byte_len: u64,
        modified: Option<SystemTime>,
        format: ImageFormat,
    ) -> Self {
        Self {
            file,
            canonical_path,
            byte_len,
            modified,
            format,
        }
    }

    /// Explicit private-data access for the decoder/cache boundary.
    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    /// Transfer the already validated, rewound handle to the decoder without
    /// reopening a potentially replaced path.
    pub fn into_file(self) -> std::fs::File {
        self.file
    }
}

#[derive(Debug)]
pub enum ResolvedSource {
    BuiltIn(rmac_wallpaper::BuiltInMetadata),
    File(FileAsset),
}

#[derive(Debug)]
pub struct ResolvedSurface {
    pub output: rmac_compositor::OutputId,
    pub logical_size: rmac_compositor::LogicalSize,
    pub scale: f64,
    pub fit: rmac_shell_settings::WallpaperFit,
    pub source: ResolvedSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionIssue {
    pub output: rmac_compositor::OutputId,
    pub kind: ErrorKind,
}

#[derive(Debug, Default)]
pub struct Resolution {
    pub surfaces: Vec<ResolvedSurface>,
    pub issues: Vec<ResolutionIssue>,
}
