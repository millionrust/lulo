use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher as _};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
#[cfg(unix)]
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt as _};

#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use crate::CONVERTER_TIMEOUT;
use crate::{cache_directory, Error, MAX_DECODE_ALLOC, MAX_SOURCE_DIMENSION, THUMBNAIL_DIMENSION};

#[cfg(target_os = "macos")]
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn generate_at(source: &Path, dimension: u32) -> Result<PathBuf, Error> {
    let cache = prepare_cache()?;
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

pub(crate) fn prepare_cache() -> Result<PathBuf, Error> {
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
    Ok(cache)
}

pub fn is_current(source: &Path, thumbnail: &Path) -> bool {
    cached_path(source, &cache_directory(), THUMBNAIL_DIMENSION)
        .is_ok_and(|expected| expected == thumbnail)
        && std::fs::symlink_metadata(thumbnail).is_ok_and(|metadata| metadata.is_file())
}

pub(crate) fn cached_path(source: &Path, cache: &Path, dimension: u32) -> Result<PathBuf, Error> {
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

pub(crate) fn render_portable(source: &Path, dimension: u32) -> Result<Vec<u8>, Error> {
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

pub(crate) fn render_converted_png(
    bytes: &[u8],
    source: &Path,
    dimension: u32,
) -> Result<Vec<u8>, Error> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|source_error| Error::Io {
            operation: "detect converted preview format",
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
    let mut output = std::io::Cursor::new(Vec::new());
    thumbnail
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|source_error| Error::Encode {
            path: source.to_path_buf(),
            source: source_error,
        })?;
    Ok(output.into_inner())
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
