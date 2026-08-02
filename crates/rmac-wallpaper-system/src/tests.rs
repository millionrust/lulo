use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::file::detect_format;

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
