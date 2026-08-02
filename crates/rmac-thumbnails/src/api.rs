#[cfg(any(not(target_os = "macos"), test))]
use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use crate::renderer::generate_at;

pub(crate) const THUMBNAIL_DIMENSION: u32 = 96;
pub(crate) const PREVIEW_DIMENSION: u32 = 1024;
pub(crate) const MAX_SOURCE_DIMENSION: u32 = 32_768;
pub(crate) const MAX_DECODE_ALLOC: u64 = 128 * 1024 * 1024;
pub(crate) const CONVERTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
pub(crate) const MAX_CONVERTER_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const PDF_CACHE_KEY: u32 = 2_001;
pub(crate) const VIDEO_CACHE_KEY: u32 = 2_002;
pub(crate) const AUDIO_CACHE_KEY: u32 = 2_003;
static FALLBACK_CACHE: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Pdf,
    Video,
    Audio,
}

impl MediaKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaPreview {
    pub kind: MediaKind,
    pub preview: PathBuf,
}

#[derive(Debug)]
pub enum Error {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Decode {
        path: PathBuf,
        source: image::ImageError,
    },
    Encode {
        path: PathBuf,
        source: image::ImageError,
    },
    Converter {
        path: PathBuf,
        message: String,
    },
    ConverterUnavailable {
        kind: MediaKind,
        program: &'static str,
    },
    Cancelled,
    SourceChanged {
        path: PathBuf,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} {}: {source}",
                path.display()
            ),
            Self::Decode { path, source } => {
                write!(formatter, "could not decode {}: {source}", path.display())
            }
            Self::Encode { path, source } => {
                write!(
                    formatter,
                    "could not encode thumbnail for {}: {source}",
                    path.display()
                )
            }
            Self::Converter { path, message } => {
                write!(
                    formatter,
                    "could not thumbnail {}: {message}",
                    path.display()
                )
            }
            Self::ConverterUnavailable { kind, program } => {
                write!(
                    formatter,
                    "{} preview support requires {program}",
                    kind.label()
                )
            }
            Self::Cancelled => formatter.write_str("preview cancelled"),
            Self::SourceChanged { path } => {
                write!(
                    formatter,
                    "{} changed while it was being decoded",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Decode { source, .. } | Self::Encode { source, .. } => Some(source),
            Self::Converter { .. }
            | Self::ConverterUnavailable { .. }
            | Self::Cancelled
            | Self::SourceChanged { .. } => None,
        }
    }
}

pub fn is_supported(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif"
    ) || cfg!(target_os = "macos") && extension == "heic"
}

pub fn cache_directory() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) {
            return PathBuf::from(home).join("Library/Caches/rmac/finder/thumbnails");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let xdg_cache = std::env::var_os("XDG_CACHE_HOME");
        let home = std::env::var_os("HOME");
        if let Some(cache) = linux_cache_directory(xdg_cache.as_deref(), home.as_deref()) {
            return cache;
        }
    }
    FALLBACK_CACHE
        .get_or_init(|| {
            let nonce = std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            std::env::temp_dir().join(format!(
                "rmac-finder-thumbnails-{}-{nonce}",
                std::process::id()
            ))
        })
        .clone()
}

#[cfg(any(not(target_os = "macos"), test))]
pub(crate) fn linux_cache_directory(
    xdg_cache: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    if let Some(cache) = xdg_cache
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Some(cache.join("rmac/finder/thumbnails"));
    }
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".cache/rmac/finder/thumbnails"))
}

pub fn generate(source: &Path) -> Result<PathBuf, Error> {
    generate_at(source, THUMBNAIL_DIMENSION)
}

/// Generate a larger, invalidation-safe image for the Files Quick Look
/// surface. The portable decoder enforces source dimension and allocation
/// limits before returning pixels.
pub fn generate_preview(source: &Path) -> Result<PathBuf, Error> {
    generate_at(source, PREVIEW_DIMENSION)
}
