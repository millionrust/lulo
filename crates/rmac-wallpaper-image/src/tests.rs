use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

fn target(width: u32, height: u32) -> rmac_compositor::PhysicalSize {
    rmac_compositor::PhysicalSize { width, height }
}

#[test]
fn color_summary_is_an_eight_by_eight_srgb_average() {
    let mut rgba = Vec::with_capacity(8 * 8 * 4);
    for row in 0..8_u8 {
        for column in 0..8_u8 {
            rgba.extend_from_slice(&[column * 20, row * 10, 200, 255]);
        }
    }
    let image = Decoded {
        width: 8,
        height: 8,
        rgba: Arc::from(rgba),
    };
    let summary = summarize_color(&image).unwrap();
    assert_eq!(summary.dominant, [70, 35, 200]);
    assert!((summary.luminance - 0.067).abs() < 0.002);
}

#[test]
fn color_summary_rejects_an_invalid_pixel_buffer() {
    let image = Decoded {
        width: 8,
        height: 8,
        rgba: Arc::from([0_u8; 4]),
    };
    assert_eq!(summarize_color(&image), None);
}

#[test]
fn procedural_default_is_deterministic_bounded_and_cached_by_target() {
    let cache = Cache::new(1024 * 1024);
    let source = || {
        rmac_wallpaper_system::ResolvedSource::BuiltIn(rmac_wallpaper::BuiltInId::Aurora.metadata())
    };
    let first = cache.get_or_decode(source(), target(64, 32)).unwrap();
    let second = cache.get_or_decode(source(), target(64, 32)).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.rgba.len(), 64 * 32 * 4);
    assert_eq!(cache.decode_count(), 1);
    assert!(!Arc::ptr_eq(
        &first,
        &cache.get_or_decode(source(), target(32, 32)).unwrap()
    ));
    assert_eq!(cache.decode_count(), 2);
}

#[test]
fn user_file_decode_is_shared_across_outputs_and_explicitly_invalidated() {
    let root = temporary_directory("file-cache");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("wallpaper.png");
    image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255]))
        .save(&path)
        .unwrap();
    let cache = Cache::default();
    let source =
        || rmac_wallpaper_system::resolve(&rmac_wallpaper::Source::File(path.clone())).unwrap();
    let first = cache.get_or_decode(source(), target(1920, 1080)).unwrap();
    let second = cache.get_or_decode(source(), target(3840, 2160)).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.physical_size(), target(8, 4));
    assert_eq!(cache.decode_count(), 1);
    assert_eq!(cache.invalidate_path(&path.canonicalize().unwrap()), 1);
    let third = cache.get_or_decode(source(), target(1920, 1080)).unwrap();
    assert!(!Arc::ptr_eq(&first, &third));
    assert_eq!(cache.decode_count(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bounds_reject_zero_per_axis_and_decompression_sized_targets() {
    let cache = Cache::default();
    let source = || {
        rmac_wallpaper_system::ResolvedSource::BuiltIn(rmac_wallpaper::BuiltInId::Aurora.metadata())
    };
    assert_eq!(
        cache
            .get_or_decode(source(), target(0, 10))
            .unwrap_err()
            .kind,
        ErrorKind::InvalidDimensions
    );
    assert_eq!(
        cache
            .get_or_decode(source(), target(10_000, 10_000))
            .unwrap_err()
            .kind,
        ErrorKind::TooManyPixels
    );
}

#[test]
fn lru_budget_evicts_old_images_without_invalidating_live_arcs() {
    let cache = Cache::new(4 * 4 * 4);
    let source = || {
        rmac_wallpaper_system::ResolvedSource::BuiltIn(rmac_wallpaper::BuiltInId::Aurora.metadata())
    };
    let first = cache.get_or_decode(source(), target(4, 4)).unwrap();
    cache.get_or_decode(source(), target(2, 2)).unwrap();
    assert_eq!(cache.len(), 1);
    assert_eq!(first.rgba.len(), 64);
    assert_eq!(first.rgba[3], 255);
}

#[test]
fn empty_watch_set_needs_no_native_watcher_and_failures_are_redacted() {
    let (sender, _) = async_channel::bounded(1);
    assert!(watch_files(&[], sender).unwrap().is_none());
    let event = FileWatchEvent::Failed {
        detail: "/home/alex/private/wallpaper.png".into(),
    };
    assert!(!format!("{event:?}").contains("alex"));
}

#[test]
fn rasterization_shares_user_decode_across_outputs_and_falls_back_per_failure() {
    let root = temporary_directory("rasterize");
    std::fs::create_dir_all(&root).unwrap();
    let valid = root.join("valid.png");
    image::RgbaImage::from_pixel(8, 4, image::Rgba([1, 2, 3, 255]))
        .save(&valid)
        .unwrap();
    let corrupt = root.join("corrupt.png");
    std::fs::write(&corrupt, b"\x89PNG\r\n\x1a\ncorrupt").unwrap();
    let surface = |output: &str, source: std::path::PathBuf, scale: f64| rmac_wallpaper::Surface {
        output: output.into(),
        logical_size: rmac_compositor::LogicalSize {
            width: 16.0,
            height: 9.0,
        },
        scale,
        fit: rmac_shell_settings::WallpaperFit::Fill,
        source: rmac_wallpaper::Source::File(source),
    };
    let plan = rmac_wallpaper::Plan {
        surfaces: vec![
            surface("DP-1", valid.clone(), 1.0),
            surface("DP-2", valid, 2.0),
            surface("DP-3", corrupt, 1.0),
            surface("DP-4", root.join("missing.png"), 1.0),
        ],
        issues: Vec::new(),
    };
    let cache = Cache::default();
    let rasterized = rasterize(&plan, &cache);
    assert_eq!(rasterized.surfaces.len(), 4);
    assert!(Arc::ptr_eq(
        &rasterized.surfaces[0].image,
        &rasterized.surfaces[1].image
    ));
    assert!(rasterized.surfaces[..2]
        .iter()
        .all(|surface| { surface.source == RasterSource::UserFile && !surface.fallback }));
    assert_eq!(rasterized.issues.len(), 2);
    assert!(rasterized
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, RasterIssueKind::Decode(_))));
    assert!(rasterized
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, RasterIssueKind::Resolve(_))));
    assert_eq!(rasterized.surfaces[2].image.physical_size(), target(16, 9));
    assert_eq!(rasterized.surfaces[3].image.physical_size(), target(16, 9));
    assert!(rasterized.surfaces[2..].iter().all(|surface| {
        surface.source == RasterSource::BuiltIn(rmac_wallpaper::FALLBACK_BUILT_IN)
            && surface.fallback
    }));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn procedural_built_ins_follow_the_appearance() {
    let cache = Cache::new(1024 * 1024);
    let source = || {
        rmac_wallpaper_system::ResolvedSource::BuiltIn(rmac_wallpaper::BuiltInId::Aurora.metadata())
    };
    let dark = cache
        .get_or_decode_for(source(), target(8, 8), true)
        .unwrap();
    let light = cache
        .get_or_decode_for(source(), target(8, 8), false)
        .unwrap();
    assert_ne!(dark.rgba, light.rgba);
    // The appearance is part of the cache identity.
    assert_eq!(cache.len(), 2);
    assert_eq!(
        cache.get_or_decode(source(), target(8, 8)).unwrap().rgba,
        dark.rgba
    );
}

#[test]
fn packaged_artwork_follows_the_appearance_and_missing_artwork_falls_back_visibly() {
    // The only test that points the artwork directory somewhere else, so the
    // environment change cannot race another reader.
    let root = temporary_directory("artwork");
    std::fs::create_dir_all(&root).unwrap();
    // Content is detected by magic bytes, so a small PNG stands in for the JPEG.
    image::RgbaImage::from_pixel(16, 9, image::Rgba([10, 20, 30, 255]))
        .save_with_format(
            root.join("lulo-dark-1920x1080.jpg"),
            image::ImageFormat::Png,
        )
        .unwrap();
    std::env::set_var(rmac_wallpaper::PACKAGED_WALLPAPER_DIR_ENV, &root);
    let plan = rmac_wallpaper::Plan {
        surfaces: vec![rmac_wallpaper::Surface {
            output: "eDP-1".into(),
            logical_size: rmac_compositor::LogicalSize {
                width: 1536.0,
                height: 864.0,
            },
            scale: 1.25,
            fit: rmac_shell_settings::WallpaperFit::Fill,
            source: rmac_wallpaper::Source::BuiltIn(rmac_wallpaper::BuiltInId::Lulo),
        }],
        issues: Vec::new(),
    };
    let cache = Cache::default();

    let dark = rasterize_for(&plan, &cache, true);
    assert!(dark.issues.is_empty());
    assert_eq!(dark.surfaces.len(), 1);
    assert_eq!(
        dark.surfaces[0].source,
        RasterSource::BuiltIn(rmac_wallpaper::BuiltInId::Lulo)
    );
    assert!(!dark.surfaces[0].fallback);
    assert_eq!(dark.surfaces[0].image.physical_size(), target(16, 9));
    assert_eq!(&dark.surfaces[0].image.rgba[..4], &[10, 20, 30, 255]);

    // No light image is packaged here: the output shows the file-free
    // fallback and records why instead of going blank.
    let light = rasterize_for(&plan, &cache, false);
    std::env::remove_var(rmac_wallpaper::PACKAGED_WALLPAPER_DIR_ENV);
    assert_eq!(light.surfaces.len(), 1);
    assert_eq!(
        light.surfaces[0].source,
        RasterSource::BuiltIn(rmac_wallpaper::FALLBACK_BUILT_IN)
    );
    assert!(light.surfaces[0].fallback);
    assert_eq!(
        light.issues[0].kind,
        RasterIssueKind::Decode(ErrorKind::Artwork)
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn temporary_directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rmac-wallpaper-image-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
