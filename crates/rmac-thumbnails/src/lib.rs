//! Cross-platform, invalidation-safe image thumbnail generation.

use std::collections::hash_map::DefaultHasher;
#[cfg(any(not(target_os = "macos"), test))]
use std::ffi::OsStr;
use std::fmt;
use std::hash::{Hash, Hasher as _};
use std::io;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

const MAX_DIMENSION: u32 = 96;
#[cfg(target_os = "macos")]
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FALLBACK_CACHE: OnceLock<PathBuf> = OnceLock::new();

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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Decode { source, .. } | Self::Encode { source, .. } => Some(source),
            Self::Converter { .. } => None,
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
fn linux_cache_directory(xdg_cache: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
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
    let cache = cache_directory();
    std::fs::create_dir_all(&cache).map_err(|source| Error::Io {
        operation: "create thumbnail cache",
        path: cache.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| Error::Io {
                operation: "secure thumbnail cache",
                path: cache.clone(),
                source,
            },
        )?;
    }
    let output = cached_path(source, &cache)?;
    if output.is_file() {
        return Ok(output);
    }

    #[cfg(target_os = "macos")]
    let png = render_with_sips(source, &cache)?;
    #[cfg(not(target_os = "macos"))]
    let png = render_portable(source)?;
    rmac_storage::atomic_write(&output, &png).map_err(|source| Error::Io {
        operation: "write thumbnail",
        path: output.clone(),
        source,
    })?;
    Ok(output)
}

pub fn is_current(source: &Path, thumbnail: &Path) -> bool {
    cached_path(source, &cache_directory()).is_ok_and(|expected| expected == thumbnail)
        && thumbnail.is_file()
}

fn cached_path(source: &Path, cache: &Path) -> Result<PathBuf, Error> {
    let metadata = source.metadata().map_err(|source_error| Error::Io {
        operation: "read image metadata",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);
    MAX_DIMENSION.hash(&mut hasher);
    Ok(cache.join(format!("{:016x}.png", hasher.finish())))
}

#[cfg(any(not(target_os = "macos"), test))]
fn render_portable(source: &Path) -> Result<Vec<u8>, Error> {
    let image = image::ImageReader::open(source)
        .map_err(|source_error| Error::Io {
            operation: "open image",
            path: source.to_path_buf(),
            source: source_error,
        })?
        .with_guessed_format()
        .map_err(|source_error| Error::Io {
            operation: "detect image format",
            path: source.to_path_buf(),
            source: source_error,
        })?
        .decode()
        .map_err(|source_error| Error::Decode {
            path: source.to_path_buf(),
            source: source_error,
        })?;
    let thumbnail = image.thumbnail(MAX_DIMENSION, MAX_DIMENSION);
    let mut bytes = std::io::Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|source_error| Error::Encode {
            path: source.to_path_buf(),
            source: source_error,
        })?;
    Ok(bytes.into_inner())
}

#[cfg(target_os = "macos")]
fn render_with_sips(source: &Path, cache: &Path) -> Result<Vec<u8>, Error> {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = cache.join(format!(
        ".sips-{}-{sequence}-{}.png",
        std::process::id(),
        source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image")
    ));
    let output = Command::new("sips")
        .args(["-s", "format", "png", "-Z", "96"])
        .arg(source)
        .arg("--out")
        .arg(&temporary)
        .output()
        .map_err(|source_error| Error::Io {
            operation: "start sips",
            path: source.to_path_buf(),
            source: source_error,
        })?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&temporary);
        return Err(Error::Converter {
            path: source.to_path_buf(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let result = std::fs::read(&temporary).map_err(|source_error| Error::Io {
        operation: "read converted thumbnail",
        path: temporary.clone(),
        source: source_error,
    });
    let _ = std::fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn portable_thumbnail_fits_bounds_and_is_png() {
        let root = temporary_directory("render");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("wide.png");
        image::RgbaImage::new(240, 120).save(&source).unwrap();

        let bytes = render_portable(&source).unwrap();
        let thumbnail =
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (96, 48));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_key_changes_with_source_content() {
        let root = temporary_directory("invalidation");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("image.png");
        std::fs::write(&source, b"one").unwrap();
        let first = cached_path(&source, &root).unwrap();
        std::fs::write(&source, b"a different length").unwrap();
        let second = cached_path(&source, &root).unwrap();

        assert_ne!(first, second);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_images_return_decode_errors() {
        let root = temporary_directory("corrupt");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("broken.png");
        std::fs::write(&source, b"not a png").unwrap();

        assert!(matches!(
            render_portable(&source),
            Err(Error::Decode { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn linux_cache_path_honors_absolute_xdg_then_home_fallback() {
        assert_eq!(
            linux_cache_directory(Some(OsStr::new("/cache")), Some(OsStr::new("/home/user"))),
            Some(PathBuf::from("/cache/rmac/finder/thumbnails"))
        );
        assert_eq!(
            linux_cache_directory(Some(OsStr::new("relative")), Some(OsStr::new("/home/user"))),
            Some(PathBuf::from("/home/user/.cache/rmac/finder/thumbnails"))
        );
    }

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-thumbnails-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
