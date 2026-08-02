use super::*;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::linux_cache_directory;
use crate::media::{
    converter_plan, open_media_source, run_converter, validate_converter_output, ConverterPlan,
};
use crate::renderer::{cached_path, render_converted_png, render_portable};

#[test]
fn portable_thumbnail_fits_bounds_and_is_png() {
    let root = temporary_directory("render");
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("wide.png");
    image::RgbaImage::new(240, 120).save(&source).unwrap();

    let bytes = render_portable(&source, THUMBNAIL_DIMENSION).unwrap();
    let thumbnail = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
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
