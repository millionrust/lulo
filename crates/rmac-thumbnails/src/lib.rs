//! Cross-platform, invalidation-safe image thumbnail generation.

use std::collections::hash_map::DefaultHasher;
#[cfg(any(not(target_os = "macos"), test))]
use std::ffi::OsStr;
use std::fmt;
use std::hash::{Hash, Hasher as _};
use std::io;
use std::io::BufReader;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};
#[cfg(unix)]
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt as _};

const THUMBNAIL_DIMENSION: u32 = 96;
const PREVIEW_DIMENSION: u32 = 1024;
const MAX_SOURCE_DIMENSION: u32 = 32_768;
const MAX_DECODE_ALLOC: u64 = 128 * 1024 * 1024;
#[cfg(target_os = "macos")]
const CONVERTER_TIMEOUT: Duration = Duration::from_secs(8);
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
            Self::Converter { .. } | Self::SourceChanged { .. } => None,
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
    generate_at(source, THUMBNAIL_DIMENSION)
}

/// Generate a larger, invalidation-safe image for the Files Quick Look
/// surface. The portable decoder enforces source dimension and allocation
/// limits before returning pixels.
pub fn generate_preview(source: &Path) -> Result<PathBuf, Error> {
    generate_at(source, PREVIEW_DIMENSION)
}

fn generate_at(source: &Path, dimension: u32) -> Result<PathBuf, Error> {
    let cache = cache_directory();
    std::fs::create_dir_all(&cache).map_err(|source| Error::Io {
        operation: "create thumbnail cache",
        path: cache.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let metadata = std::fs::symlink_metadata(&cache).map_err(|source| Error::Io {
            operation: "validate thumbnail cache",
            path: cache.clone(),
            source,
        })?;
        // SAFETY: `geteuid` takes no pointers and only returns process state.
        let effective_uid = unsafe { libc::geteuid() };
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != effective_uid
        {
            return Err(Error::Io {
                operation: "validate thumbnail cache",
                path: cache,
                source: io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "thumbnail cache is not a private user-owned directory",
                ),
            });
        }
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| Error::Io {
                operation: "secure thumbnail cache",
                path: cache.clone(),
                source,
            },
        )?;
    }
    let output = cached_path(source, &cache, dimension)?;
    if std::fs::symlink_metadata(&output).is_ok_and(|metadata| metadata.is_file()) {
        return Ok(output);
    }

    #[cfg(target_os = "macos")]
    let png = if source
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("heic"))
    {
        render_with_sips(source, &cache, dimension)?
    } else {
        render_portable(source, dimension)?
    };
    #[cfg(not(target_os = "macos"))]
    let png = render_portable(source, dimension)?;
    if cached_path(source, &cache, dimension)? != output {
        return Err(Error::SourceChanged {
            path: source.to_path_buf(),
        });
    }
    rmac_storage::atomic_write_private(&output, &png).map_err(|source| Error::Io {
        operation: "write thumbnail",
        path: output.clone(),
        source,
    })?;
    Ok(output)
}

pub fn is_current(source: &Path, thumbnail: &Path) -> bool {
    cached_path(source, &cache_directory(), THUMBNAIL_DIMENSION)
        .is_ok_and(|expected| expected == thumbnail)
        && std::fs::symlink_metadata(thumbnail).is_ok_and(|metadata| metadata.is_file())
}

fn cached_path(source: &Path, cache: &Path, dimension: u32) -> Result<PathBuf, Error> {
    let metadata = source
        .symlink_metadata()
        .map_err(|source_error| Error::Io {
            operation: "read image metadata",
            path: source.to_path_buf(),
            source: source_error,
        })?;
    if !metadata.is_file() {
        return Err(Error::Io {
            operation: "validate regular image",
            path: source.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "image source is not a regular file",
            ),
        });
    }
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        metadata.dev().hash(&mut hasher);
        metadata.ino().hash(&mut hasher);
        metadata.ctime().hash(&mut hasher);
        metadata.ctime_nsec().hash(&mut hasher);
    }
    dimension.hash(&mut hasher);
    Ok(cache.join(format!("{:016x}.png", hasher.finish())))
}

fn render_portable(source: &Path, dimension: u32) -> Result<Vec<u8>, Error> {
    let file = {
        #[cfg(unix)]
        {
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(source)
        }
        #[cfg(not(unix))]
        {
            std::fs::File::open(source)
        }
    }
    .map_err(|source_error| Error::Io {
        operation: "open image",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut reader = image::ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .map_err(|source_error| Error::Io {
            operation: "detect image format",
            path: source.to_path_buf(),
            source: source_error,
        })?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader.decode().map_err(|source_error| Error::Decode {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let thumbnail = image.thumbnail(dimension, dimension);
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
fn render_with_sips(source: &Path, cache: &Path, dimension: u32) -> Result<Vec<u8>, Error> {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = cache.join(format!(
        ".sips-{}-{sequence}-{dimension}-{}.png",
        std::process::id(),
        source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image")
    ));
    let mut child = Command::new("sips")
        .args(["-s", "format", "png", "-Z", &dimension.to_string()])
        .arg(source)
        .arg("--out")
        .arg(&temporary)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source_error| Error::Io {
            operation: "start sips",
            path: source.to_path_buf(),
            source: source_error,
        })?;
    let deadline = Instant::now() + CONVERTER_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&temporary);
                return Err(Error::Converter {
                    path: source.to_path_buf(),
                    message: "the image converter timed out".into(),
                });
            }
            Err(source_error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&temporary);
                return Err(Error::Io {
                    operation: "wait for sips",
                    path: source.to_path_buf(),
                    source: source_error,
                });
            }
        }
    };
    if !status.success() {
        let _ = std::fs::remove_file(&temporary);
        return Err(Error::Converter {
            path: source.to_path_buf(),
            message: "the image converter failed".into(),
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

        let bytes = render_portable(&source, THUMBNAIL_DIMENSION).unwrap();
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
        let first = cached_path(&source, &root, THUMBNAIL_DIMENSION).unwrap();
        let preview = cached_path(&source, &root, PREVIEW_DIMENSION).unwrap();
        assert_ne!(first, preview);
        std::fs::write(&source, b"a different length").unwrap();
        let second = cached_path(&source, &root, THUMBNAIL_DIMENSION).unwrap();

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
            render_portable(&source, THUMBNAIL_DIMENSION),
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
