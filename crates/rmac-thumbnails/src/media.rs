use std::ffi::{OsStr, OsString};
use std::io;
use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
#[cfg(unix)]
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt as _};

use crate::renderer::{cached_path, prepare_cache, render_converted_png};
use crate::{
    Error, MediaKind, MediaPreview, AUDIO_CACHE_KEY, CONVERTER_TIMEOUT, MAX_CONVERTER_OUTPUT_BYTES,
    PDF_CACHE_KEY, PREVIEW_DIMENSION, VIDEO_CACHE_KEY,
};

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

pub(crate) struct ConverterPlan {
    pub(crate) program: &'static str,
    pub(crate) arguments: Vec<OsString>,
}

pub(crate) fn converter_plan(kind: MediaKind, input: &OsStr) -> ConverterPlan {
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

pub(crate) fn run_converter(
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

pub(crate) fn validate_converter_output(bytes: &[u8], source: &Path) -> Result<(), Error> {
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

pub(crate) fn open_media_source(source: &Path) -> Result<std::fs::File, Error> {
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
