//! Safe local-file authority for wallpaper decoding adapters.

use std::fmt;
use std::io::{Read as _, Seek as _};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

pub const MAX_WALLPAPER_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    detail: String,
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

/// Resolve every planned output independently. One invalid custom file falls
/// back to the original built-in on that output and never removes peers.
pub fn resolve_plan(plan: &rmac_wallpaper::Plan) -> Resolution {
    let mut resolution = Resolution::default();
    for surface in &plan.surfaces {
        let source = match resolve(&surface.source) {
            Ok(source) => source,
            Err(error) => {
                resolution.issues.push(ResolutionIssue {
                    output: surface.output.clone(),
                    kind: error.kind,
                });
                ResolvedSource::BuiltIn(rmac_wallpaper::DEFAULT_BUILT_IN.metadata())
            }
        };
        resolution.surfaces.push(ResolvedSurface {
            output: surface.output.clone(),
            logical_size: surface.logical_size,
            scale: surface.scale,
            fit: surface.fit,
            source,
        });
    }
    resolution
}

pub fn resolve(source: &rmac_wallpaper::Source) -> Result<ResolvedSource, Error> {
    match source {
        rmac_wallpaper::Source::BuiltIn(id) => Ok(ResolvedSource::BuiltIn(id.metadata())),
        rmac_wallpaper::Source::File(path) => open_file(path).map(ResolvedSource::File),
    }
}

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
    Ok(FileAsset {
        file,
        canonical_path,
        byte_len: metadata.len(),
        modified: metadata.modified().ok(),
        format,
    })
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

fn detect_format(header: &[u8]) -> Option<ImageFormat> {
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn validates_magic_keeps_an_open_handle_and_redacts_default_output() {
        let root = temporary_directory("valid");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("custom.data");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nrest").unwrap();
        let asset = open_file(&path).expect("valid PNG opens");
        assert_eq!(asset.format, ImageFormat::Png);
        assert_eq!(asset.byte_len, 12);
        assert_eq!(asset.canonical_path(), path.canonicalize().unwrap());
        assert!(!format!("{asset:?}").contains(path.to_string_lossy().as_ref()));
        let mut clone = asset.into_file();
        let mut signature = [0; 8];
        clone.read_exact(&mut signature).unwrap();
        assert_eq!(&signature, b"\x89PNG\r\n\x1a\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_relative_directory_empty_oversized_and_unknown_sources() {
        let root = temporary_directory("invalid");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            open_file(Path::new("relative.png")).unwrap_err().kind,
            ErrorKind::RelativePath
        );
        assert_eq!(
            open_file(&root).unwrap_err().kind,
            ErrorKind::NotRegularFile
        );

        let empty = root.join("empty.png");
        std::fs::write(&empty, []).unwrap();
        assert_eq!(open_file(&empty).unwrap_err().kind, ErrorKind::Empty);
        let unknown = root.join("unknown.bin");
        std::fs::write(&unknown, b"not an image").unwrap();
        let error = open_file(&unknown).unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnsupportedFormat);
        assert_eq!(error.to_string(), "Could not load the wallpaper file");
        assert!(!error.to_string().contains(root.to_string_lossy().as_ref()));

        let oversized = root.join("oversized.png");
        let file = std::fs::File::create(&oversized).unwrap();
        file.set_len(MAX_WALLPAPER_BYTES + 1).unwrap();
        assert_eq!(open_file(&oversized).unwrap_err().kind, ErrorKind::TooLarge);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn built_in_resolution_uses_only_original_metadata() {
        let resolved = resolve(&rmac_wallpaper::Source::BuiltIn(
            rmac_wallpaper::BuiltInId::Aurora,
        ))
        .unwrap();
        let ResolvedSource::BuiltIn(metadata) = resolved else {
            panic!("built-in resolves without filesystem access");
        };
        assert_eq!(metadata.id.id(), "rmac-aurora");
        assert_eq!(metadata.attribution, "Original procedural artwork by rmac");
    }

    #[test]
    fn format_detection_covers_the_bounded_decoder_allowlist() {
        assert_eq!(detect_format(b"\xff\xd8\xffrest"), Some(ImageFormat::Jpeg));
        assert_eq!(
            detect_format(b"RIFF\x04\x00\x00\x00WEBP"),
            Some(ImageFormat::WebP)
        );
        assert_eq!(detect_format(b"GIF89a"), None);
    }

    #[test]
    fn plan_resolution_falls_back_per_output_without_blanking_peers() {
        let plan = rmac_wallpaper::Plan {
            surfaces: vec![
                rmac_wallpaper::Surface {
                    output: "DP-1".into(),
                    logical_size: rmac_compositor::LogicalSize {
                        width: 1280.0,
                        height: 720.0,
                    },
                    scale: 1.0,
                    fit: rmac_shell_settings::WallpaperFit::Fill,
                    source: rmac_wallpaper::Source::BuiltIn(rmac_wallpaper::BuiltInId::Aurora),
                },
                rmac_wallpaper::Surface {
                    output: "DP-2".into(),
                    logical_size: rmac_compositor::LogicalSize {
                        width: 1920.0,
                        height: 1080.0,
                    },
                    scale: 2.0,
                    fit: rmac_shell_settings::WallpaperFit::Fit,
                    source: rmac_wallpaper::Source::File(
                        "/definitely/missing/rmac-wallpaper.png".into(),
                    ),
                },
            ],
            issues: Vec::new(),
        };
        let resolved = resolve_plan(&plan);
        assert_eq!(resolved.surfaces.len(), 2);
        assert_eq!(resolved.issues.len(), 1);
        assert_eq!(resolved.issues[0].output.0, "DP-2");
        assert!(matches!(
            resolved.surfaces[0].source,
            ResolvedSource::BuiltIn(_)
        ));
        assert!(matches!(
            resolved.surfaces[1].source,
            ResolvedSource::BuiltIn(_)
        ));
    }

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rmac-wallpaper-system-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
