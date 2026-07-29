//! Cross-platform, invalidation-safe image and bounded media preview generation.

use std::collections::hash_map::DefaultHasher;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::hash::{Hash, Hasher as _};
use std::io;
use std::io::BufReader;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, UNIX_EPOCH};
#[cfg(unix)]
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt as _};

const THUMBNAIL_DIMENSION: u32 = 96;
const PREVIEW_DIMENSION: u32 = 1024;
const MAX_SOURCE_DIMENSION: u32 = 32_768;
const MAX_DECODE_ALLOC: u64 = 128 * 1024 * 1024;
const CONVERTER_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_CONVERTER_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
const PDF_CACHE_KEY: u32 = 2_001;
const VIDEO_CACHE_KEY: u32 = 2_002;
const AUDIO_CACHE_KEY: u32 = 2_003;
#[cfg(target_os = "macos")]
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
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

pub fn media_kind(path: &Path) -> Option<MediaKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "pdf" => Some(MediaKind::Pdf),
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "mpeg" | "mpg" => Some(MediaKind::Video),
        "mp3" | "m4a" | "aac" | "flac" | "ogg" | "oga" | "opus" | "wav" => Some(MediaKind::Audio),
        _ => None,
    }
}

/// Generate a bounded PDF first-page, video-frame, or audio-waveform preview.
///
/// Linux uses the fixed `/usr/bin/pdftocairo` and `/usr/bin/ffmpeg` package
/// authorities. Other development hosts resolve the same converter names from
/// their controlled process path. Converter output is never trusted directly:
/// it is size-checked, decoded through the portable image limits, resized, and
/// atomically copied into the private thumbnail cache.
pub fn generate_media_preview(source: &Path, cancel: &AtomicBool) -> Result<MediaPreview, Error> {
    check_cancelled(cancel)?;
    let kind = media_kind(source).ok_or_else(|| Error::Io {
        operation: "classify media",
        path: source.to_path_buf(),
        source: io::Error::new(io::ErrorKind::Unsupported, "unsupported preview type"),
    })?;
    let cache = prepare_cache()?;
    let cache_key = match kind {
        MediaKind::Pdf => PDF_CACHE_KEY,
        MediaKind::Video => VIDEO_CACHE_KEY,
        MediaKind::Audio => AUDIO_CACHE_KEY,
    };
    let output = cached_path(source, &cache, cache_key)?;
    if std::fs::symlink_metadata(&output).is_ok_and(|metadata| metadata.is_file())
        && cached_path(source, &cache, cache_key)? == output
    {
        return Ok(MediaPreview {
            kind,
            preview: output,
        });
    }

    let input = open_media_source(source)?;
    let plan = converter_plan(kind, converter_input_path());
    let converted = run_converter(source, kind, &plan, input, cancel)?;
    check_cancelled(cancel)?;
    validate_converter_output(&converted, source)?;
    let png = render_converted_png(&converted, source, PREVIEW_DIMENSION)?;
    check_cancelled(cancel)?;
    if cached_path(source, &cache, cache_key)? != output {
        return Err(Error::SourceChanged {
            path: source.to_path_buf(),
        });
    }
    rmac_storage::atomic_write_private(&output, &png).map_err(|source_error| Error::Io {
        operation: "write media preview",
        path: output.clone(),
        source: source_error,
    })?;
    Ok(MediaPreview {
        kind,
        preview: output,
    })
}

struct ConverterPlan {
    program: &'static str,
    arguments: Vec<OsString>,
}

fn converter_plan(kind: MediaKind, input: &OsStr) -> ConverterPlan {
    match kind {
        MediaKind::Pdf => ConverterPlan {
            program: pdftocairo_program(),
            arguments: vec![
                "-png".into(),
                "-f".into(),
                "1".into(),
                "-l".into(),
                "1".into(),
                "-singlefile".into(),
                "-scale-to".into(),
                PREVIEW_DIMENSION.to_string().into(),
                input.to_owned(),
                "-".into(),
            ],
        },
        MediaKind::Video => ConverterPlan {
            program: ffmpeg_program(),
            arguments: vec![
                "-nostdin".into(),
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-threads".into(),
                "1".into(),
                "-protocol_whitelist".into(),
                "file,pipe".into(),
                "-ss".into(),
                "0".into(),
                "-i".into(),
                input.to_owned(),
                "-map".into(),
                "0:v:0".into(),
                "-frames:v".into(),
                "1".into(),
                "-an".into(),
                "-sn".into(),
                "-dn".into(),
                "-vf".into(),
                format!(
                    "scale={PREVIEW_DIMENSION}:{PREVIEW_DIMENSION}:force_original_aspect_ratio=decrease"
                )
                .into(),
                "-f".into(),
                "image2pipe".into(),
                "-vcodec".into(),
                "png".into(),
                "pipe:1".into(),
            ],
        },
        MediaKind::Audio => ConverterPlan {
            program: ffmpeg_program(),
            arguments: vec![
                "-nostdin".into(),
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-threads".into(),
                "1".into(),
                "-protocol_whitelist".into(),
                "file,pipe".into(),
                "-i".into(),
                input.to_owned(),
                "-t".into(),
                "30".into(),
                "-filter_complex".into(),
                format!(
                    "aformat=channel_layouts=mono,showwavespic=s={PREVIEW_DIMENSION}x360:colors=0x0a84ff"
                )
                .into(),
                "-frames:v".into(),
                "1".into(),
                "-f".into(),
                "image2pipe".into(),
                "-vcodec".into(),
                "png".into(),
                "pipe:1".into(),
            ],
        },
    }
}

fn run_converter(
    source: &Path,
    kind: MediaKind,
    plan: &ConverterPlan,
    input: std::fs::File,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, Error> {
    check_cancelled(cancel)?;
    let mut child = Command::new(plan.program)
        .args(&plan.arguments)
        .stdin(Stdio::from(input))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source_error| {
            if source_error.kind() == io::ErrorKind::NotFound {
                Error::ConverterUnavailable {
                    kind,
                    program: match kind {
                        MediaKind::Pdf => "Poppler",
                        MediaKind::Video | MediaKind::Audio => "FFmpeg",
                    },
                }
            } else {
                Error::Io {
                    operation: "start preview converter",
                    path: source.to_path_buf(),
                    source: source_error,
                }
            }
        })?;
    let Some(stdout) = child.stdout.take() else {
        terminate(&mut child);
        return Err(Error::Io {
            operation: "capture preview converter",
            path: source.to_path_buf(),
            source: io::Error::other("converter stdout was unavailable"),
        });
    };
    let mut reader = Some(std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .take(MAX_CONVERTER_OUTPUT_BYTES.saturating_add(1))
            .read_to_end(&mut output)?;
        Ok::<_, io::Error>(output)
    }));
    let mut converted = None;
    let deadline = Instant::now() + CONVERTER_TIMEOUT;
    loop {
        if reader
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            converted = match join_converter_reader(
                reader.take().expect("finished converter reader"),
                source,
            ) {
                Ok(output) => Some(output),
                Err(error) => {
                    terminate(&mut child);
                    return Err(error);
                }
            };
            if converted
                .as_ref()
                .is_some_and(|bytes| bytes.len() as u64 > MAX_CONVERTER_OUTPUT_BYTES)
            {
                terminate(&mut child);
                return Err(Error::Converter {
                    path: source.to_path_buf(),
                    message: "converter output exceeded the preview limit".into(),
                });
            }
        }
        if cancel.load(Ordering::Acquire) {
            terminate(&mut child);
            discard_converter_reader(reader.take());
            return Err(Error::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let output = match converted {
                    Some(output) => output,
                    None => join_converter_reader(
                        reader.take().expect("running converter reader"),
                        source,
                    )?,
                };
                if output.len() as u64 > MAX_CONVERTER_OUTPUT_BYTES {
                    return Err(Error::Converter {
                        path: source.to_path_buf(),
                        message: "converter output exceeded the preview limit".into(),
                    });
                }
                return Ok(output);
            }
            Ok(Some(_)) => {
                discard_converter_reader(reader.take());
                return Err(Error::Converter {
                    path: source.to_path_buf(),
                    message: "the preview converter rejected this file".into(),
                });
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                terminate(&mut child);
                discard_converter_reader(reader.take());
                return Err(Error::Converter {
                    path: source.to_path_buf(),
                    message: "the preview converter timed out".into(),
                });
            }
            Err(source_error) => {
                terminate(&mut child);
                discard_converter_reader(reader.take());
                return Err(Error::Io {
                    operation: "wait for preview converter",
                    path: source.to_path_buf(),
                    source: source_error,
                });
            }
        }
    }
}

fn join_converter_reader(
    reader: std::thread::JoinHandle<io::Result<Vec<u8>>>,
    source: &Path,
) -> Result<Vec<u8>, Error> {
    reader
        .join()
        .map_err(|_| Error::Io {
            operation: "join preview converter",
            path: source.to_path_buf(),
            source: io::Error::other("converter output reader stopped unexpectedly"),
        })?
        .map_err(|source_error| Error::Io {
            operation: "read preview converter",
            path: source.to_path_buf(),
            source: source_error,
        })
}

fn discard_converter_reader(reader: Option<std::thread::JoinHandle<io::Result<Vec<u8>>>>) {
    if let Some(reader) = reader {
        let _ = reader.join();
    }
}

fn validate_converter_output(bytes: &[u8], source: &Path) -> Result<(), Error> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() as u64 > MAX_CONVERTER_OUTPUT_BYTES {
        return Err(Error::Io {
            operation: "validate converted preview",
            path: source.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                "converter output was not a bounded PNG",
            ),
        });
    }
    Ok(())
}

fn terminate(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

fn open_media_source(source: &Path) -> Result<std::fs::File, Error> {
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
        operation: "open media preview source",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(Error::Io {
            operation: "validate media preview source",
            path: source.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "media preview source is not a regular file",
            ),
        });
    }
    Ok(file)
}

#[cfg(unix)]
fn converter_input_path() -> &'static OsStr {
    OsStr::new("/dev/fd/0")
}

#[cfg(not(unix))]
fn converter_input_path() -> &'static OsStr {
    OsStr::new("-")
}

#[cfg(target_os = "linux")]
fn pdftocairo_program() -> &'static str {
    "/usr/bin/pdftocairo"
}

#[cfg(not(target_os = "linux"))]
fn pdftocairo_program() -> &'static str {
    "pdftocairo"
}

#[cfg(target_os = "linux")]
fn ffmpeg_program() -> &'static str {
    "/usr/bin/ffmpeg"
}

#[cfg(not(target_os = "linux"))]
fn ffmpeg_program() -> &'static str {
    "ffmpeg"
}

fn generate_at(source: &Path, dimension: u32) -> Result<PathBuf, Error> {
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

fn prepare_cache() -> Result<PathBuf, Error> {
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

fn render_converted_png(bytes: &[u8], source: &Path, dimension: u32) -> Result<Vec<u8>, Error> {
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
    fn media_classification_and_converter_plans_are_fixed_and_shell_free() {
        assert_eq!(media_kind(Path::new("document.PDF")), Some(MediaKind::Pdf));
        assert_eq!(media_kind(Path::new("clip.webm")), Some(MediaKind::Video));
        assert_eq!(
            media_kind(Path::new("recording.FLAC")),
            Some(MediaKind::Audio)
        );
        assert_eq!(media_kind(Path::new("archive.zip")), None);

        let input = OsStr::new("/dev/fd/0");
        let pdf = converter_plan(MediaKind::Pdf, input);
        assert!(pdf.program.ends_with("pdftocairo"));
        assert_eq!(pdf.arguments.last(), Some(&OsString::from("-")));
        assert!(pdf.arguments.iter().any(|argument| argument == input));

        let video = converter_plan(MediaKind::Video, input);
        assert!(video.program.ends_with("ffmpeg"));
        assert!(video
            .arguments
            .windows(2)
            .any(|pair| pair == ["-protocol_whitelist", "file,pipe"]));
        assert_eq!(video.arguments.last(), Some(&OsString::from("pipe:1")));
        assert!(!video
            .arguments
            .iter()
            .any(|argument| argument == "-c" || argument == "sh"));
    }

    #[cfg(unix)]
    #[test]
    fn converter_runner_captures_and_revalidates_png_stdout() {
        let root = temporary_directory("converter-stdout");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("frame.png");
        image::RgbaImage::new(32, 16).save(&source).unwrap();
        let plan = ConverterPlan {
            program: "/bin/cat",
            arguments: Vec::new(),
        };
        let cancel = AtomicBool::new(false);
        let input = open_media_source(&source).unwrap();

        let converted = run_converter(&source, MediaKind::Video, &plan, input, &cancel).unwrap();
        validate_converter_output(&converted, &source).unwrap();
        let bounded = render_converted_png(&converted, &source, PREVIEW_DIMENSION).unwrap();
        let image = image::load_from_memory_with_format(&bounded, image::ImageFormat::Png).unwrap();

        assert_eq!((image.width(), image.height()), (1024, 512));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn media_preview_honors_cancellation_before_converter_or_cache_work() {
        let cancel = AtomicBool::new(true);

        assert!(matches!(
            generate_media_preview(Path::new("/missing/document.pdf"), &cancel),
            Err(Error::Cancelled)
        ));
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
