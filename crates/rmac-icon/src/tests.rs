use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

use image::ImageEncoder as _;

use super::*;

fn request(edge: u32) -> DecodeRequest {
    DecodeRequest::new(edge).unwrap()
}

fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}

fn jpeg(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 90)
        .write_image(rgb, width, height, image::ExtendedColorType::Rgb8)
        .unwrap();
    bytes
}

fn root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("rmac-icon-{label}-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn request_and_input_limits_fail_before_allocation() {
    assert_eq!(
        DecodeRequest::new(0).unwrap_err().kind(),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        DecodeRequest::new(MAX_ICON_EDGE + 1).unwrap_err().kind(),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        decode_bytes(&vec![b'x'; MAX_ICON_FILE_BYTES + 1], request(64))
            .unwrap_err()
            .kind(),
        ErrorKind::TooLarge
    );
}

#[test]
fn png_is_detected_from_bytes_and_fitted_into_exact_square_rgba() {
    let bytes = png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]);
    let decoded = decode_bytes(&bytes, request(8)).unwrap();
    assert_eq!(decoded.edge(), 8);
    assert_eq!(decoded.format(), SourceFormat::Png);
    assert_eq!(decoded.rgba().len(), 8 * 8 * 4);
    assert!(decoded.rgba()[..8 * 2 * 4]
        .chunks_exact(4)
        .all(|pixel| pixel[3] == 0));
    assert!(decoded.rgba()[8 * 2 * 4..8 * 6 * 4]
        .chunks_exact(4)
        .any(|pixel| pixel[3] == 255));
}

#[test]
fn png_decoder_enforces_source_dimensions() {
    let width = MAX_ICON_SOURCE_DIMENSION + 1;
    let bytes = png(width, 1, &vec![255; width as usize * 4]);
    assert_eq!(
        decode_bytes(&bytes, request(32)).unwrap_err().kind(),
        ErrorKind::TooLarge
    );
}

#[test]
fn jpeg_is_detected_from_bytes_and_decoded_to_opaque_rgba() {
    let bytes = jpeg(2, 1, &[255, 0, 0, 0, 255, 0]);
    let decoded = decode_bytes(&bytes, request(8)).unwrap();
    assert_eq!(decoded.edge(), 8);
    assert_eq!(decoded.format(), SourceFormat::Jpeg);
    assert_eq!(decoded.rgba().len(), 8 * 8 * 4);
    assert!(decoded
        .rgba()
        .chunks_exact(4)
        .filter(|pixel| pixel[3] != 0)
        .all(|pixel| pixel[3] == 255));
}

#[test]
fn svg_renders_centered_straight_rgba_without_external_resources() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000" fill-opacity="0.5"/></svg>"##;
    let decoded = decode_bytes(svg, request(20)).unwrap();
    assert_eq!(decoded.format(), SourceFormat::Svg);
    assert_eq!(decoded.rgba().len(), 20 * 20 * 4);
    let top = &decoded.rgba()[0..20 * 5 * 4];
    assert!(top.chunks_exact(4).all(|pixel| pixel[3] == 0));
    let center = &decoded.rgba()[(10 * 20 + 10) * 4..][..4];
    assert!(center[0] >= 254);
    assert_eq!(center[1], 0);
    assert!((127..=128).contains(&center[3]));
}

#[test]
fn svg_rejects_active_external_and_expansion_features() {
    let cases: [&[u8]; 6] = [
        br#"<!DOCTYPE svg><svg xmlns="http://www.w3.org/2000/svg"/>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="/private/file"/></svg>"#,
        br##"<svg xmlns="http://www.w3.org/2000/svg"><use href="#x"/></svg>"##,
        br#"<svg xmlns="http://www.w3.org/2000/svg"><filter/></svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg"><rect onclick="x()"/></svg>"#,
    ];
    for bytes in cases {
        assert_eq!(
            decode_bytes(bytes, request(32)).unwrap_err().kind(),
            ErrorKind::UnsafeSvg
        );
    }
}

#[test]
fn malformed_unsupported_and_invisible_inputs_are_distinct() {
    assert_eq!(
        decode_bytes(b"not an icon", request(32))
            .unwrap_err()
            .kind(),
        ErrorKind::Malformed
    );
    assert_eq!(
        decode_bytes(&[0xff, 0xfe], request(32)).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        decode_bytes(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#,
            request(32)
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Empty
    );
}

#[test]
fn file_boundary_is_bounded_and_diagnostics_do_not_expose_path_or_pixels() {
    let root = root("private");
    let path = root.join("secret-app-icon.png");
    let bytes = png(1, 1, &[12, 34, 56, 255]);
    File::create(&path).unwrap().write_all(&bytes).unwrap();

    let decoded = decode_file(&path, request(4)).unwrap();
    let debug = format!("{decoded:?}");
    assert!(!debug.contains("secret-app-icon"));
    assert!(!debug.contains("12, 34, 56"));
    assert!(debug.contains("rgba_bytes: 64"));

    let missing = root.join("private-missing-icon.svg");
    let error = decode_file(&missing, request(4)).unwrap_err();
    let diagnostics = format!("{error:?} {error}");
    assert!(!diagnostics.contains("private-missing-icon"));
    assert_eq!(error.kind(), ErrorKind::Io(std::io::ErrorKind::NotFound));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_hits_then_invalidates_an_atomically_replaced_file() {
    let root = root("cache-change");
    let path = root.join("private-theme-icon.png");
    std::fs::write(&path, png(1, 1, &[10, 20, 30, 255])).unwrap();
    let cache = Cache::default();

    let first = cache.get_or_decode(&path, request(8)).unwrap();
    let again = cache.get_or_decode(&path, request(8)).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(cache.stats().decodes, 1);

    let replacement = root.join("replacement.png");
    std::fs::write(&replacement, png(2, 1, &[90, 80, 70, 255, 60, 50, 40, 255])).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    let changed = cache.get_or_decode(&path, request(8)).unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(cache.stats().decodes, 2);
    assert_eq!(cache.stats().entries, 1);

    cache.invalidate_path(&path);
    assert_eq!(cache.stats().entries, 0);
    assert!(!format!("{cache:?}").contains("private-theme-icon"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_enforces_byte_budget_with_lru_eviction() {
    let root = root("cache-budget");
    let first_path = root.join("first.png");
    let second_path = root.join("second.png");
    std::fs::write(&first_path, png(1, 1, &[1, 2, 3, 255])).unwrap();
    std::fs::write(&second_path, png(1, 1, &[4, 5, 6, 255])).unwrap();
    let cache = Cache::new(4 * 4 * 4);

    cache.get_or_decode(&first_path, request(4)).unwrap();
    cache.get_or_decode(&second_path, request(4)).unwrap();
    assert_eq!(cache.stats().entries, 1);
    assert_eq!(cache.stats().bytes, 64);
    assert_eq!(cache.stats().decodes, 2);
    cache.get_or_decode(&first_path, request(4)).unwrap();
    assert_eq!(cache.stats().decodes, 3);
    assert_eq!(cache.stats().entries, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_misses_are_coalesced_into_one_bounded_decode() {
    let root = root("cache-coalesce");
    let path = root.join("shared.png");
    std::fs::write(&path, png(1, 1, &[7, 8, 9, 255])).unwrap();
    let cache = Arc::new(Cache::default());
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for _ in 0..4 {
        let cache = cache.clone();
        let barrier = barrier.clone();
        let path = path.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            cache.get_or_decode(&path, request(16)).unwrap()
        }));
    }
    barrier.wait();
    let icons = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert!(icons
        .iter()
        .skip(1)
        .all(|icon| Arc::ptr_eq(&icons[0], icon)));
    assert_eq!(cache.stats().decodes, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn third_party_plate_only_wraps_non_app_icons() {
    assert_eq!(third_party_plate(IconShape::RoundedSquare), None);
    assert_eq!(third_party_plate(IconShape::Other), Some(PLATE_ICON_SCALE));
    assert!(PLATE_ICON_SCALE > 0.0 && PLATE_ICON_SCALE < 1.0);
}
