use std::io::{Read as _, Seek as _};
use std::path::{Component, Path};

use crate::{Error, ErrorKind, FileAsset, ImageFormat, MAX_WALLPAPER_BYTES};

pub fn open_file(path: &Path) -> Result<FileAsset, Error> {
    validate_path(path)?;
    let canonical_path = path
        .canonicalize()
        .map_err(|error| io_error(error, "resolve wallpaper path"))?;
    let mut file =
        std::fs::File::open(path).map_err(|error| io_error(error, "open wallpaper file"))?;
    let metadata = file
        .metadata()
        .map_err(|error| io_error(error, "read wallpaper metadata"))?;
    if !metadata.is_file() {
        return Err(failure(
            ErrorKind::NotRegularFile,
            "wallpaper source is not a regular file",
        ));
    }
    if metadata.len() == 0 {
        return Err(failure(ErrorKind::Empty, "wallpaper file is empty"));
    }
    if metadata.len() > MAX_WALLPAPER_BYTES {
        return Err(failure(
            ErrorKind::TooLarge,
            format!(
                "wallpaper file is {} bytes; limit is {MAX_WALLPAPER_BYTES}",
                metadata.len()
            ),
        ));
    }
    let mut header = [0u8; 12];
    let read = file
        .read(&mut header)
        .map_err(|error| io_error(error, "read wallpaper header"))?;
    let format = detect_format(&header[..read]).ok_or_else(|| {
        failure(
            ErrorKind::UnsupportedFormat,
            "wallpaper must be PNG, JPEG, or WebP",
        )
    })?;
    file.rewind()
        .map_err(|error| io_error(error, "rewind wallpaper file"))?;
    Ok(FileAsset::new(
        file,
        canonical_path,
        metadata.len(),
        metadata.modified().ok(),
        format,
    ))
}

fn validate_path(path: &Path) -> Result<(), Error> {
    if !path.is_absolute() {
        return Err(failure(
            ErrorKind::RelativePath,
            "wallpaper path is not absolute",
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(failure(
            ErrorKind::UnnormalizedPath,
            "wallpaper path is not normalized",
        ));
    }
    Ok(())
}

pub(crate) fn detect_format(header: &[u8]) -> Option<ImageFormat> {
    if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if header.starts_with(b"\xff\xd8\xff") {
        Some(ImageFormat::Jpeg)
    } else if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

fn io_error(error: std::io::Error, operation: &str) -> Error {
    failure(ErrorKind::Io(error.kind()), format!("{operation}: {error}"))
}

fn failure(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error {
        kind,
        detail: detail.into(),
    }
}
