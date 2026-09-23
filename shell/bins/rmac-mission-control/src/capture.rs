//! Window pictures for Mission Control (docs/decisions/0014-mission-control.md).
//!
//! niri 26.04 can copy an output (wlr-screencopy and ext-image-copy-capture
//! with output sources) but not a single window, and its screenshot-window
//! action always writes the clipboard. So the service reads the focused
//! output once with `grim`, before its overlay maps, and cuts each visible
//! window out of that picture. The raw PPM stream avoids an encode and a
//! decode; the pixels never touch the disk.

use std::io;
use std::process::{Command, Stdio};

use crate::model::Rect;

/// An RGB picture of one output.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// `grim -t ppm -o OUTPUT -`.
pub fn grab_output(output: &str) -> io::Result<Picture> {
    let result = Command::new("grim")
        .args(["-t", "ppm", "-o", output, "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()?;
    if !result.status.success() {
        return Err(io::Error::other(format!("grim failed: {}", result.status)));
    }
    parse_ppm(&result.stdout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "grim wrote an unreadable PPM"))
}

/// The next header token, skipping whitespace and `#` comments.
fn next_token<'a>(bytes: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    loop {
        while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
            *cursor += 1;
        }
        if *cursor < bytes.len() && bytes[*cursor] == b'#' {
            while *cursor < bytes.len() && bytes[*cursor] != b'\n' {
                *cursor += 1;
            }
            continue;
        }
        break;
    }
    let start = *cursor;
    while *cursor < bytes.len() && !bytes[*cursor].is_ascii_whitespace() {
        *cursor += 1;
    }
    (*cursor > start).then(|| &bytes[start..*cursor])
}

fn next_number(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    std::str::from_utf8(next_token(bytes, cursor)?)
        .ok()?
        .parse()
        .ok()
}

/// Parse a binary (P6), 8-bit PPM.
pub fn parse_ppm(bytes: &[u8]) -> Option<Picture> {
    let mut cursor = 0usize;
    if next_token(bytes, &mut cursor)? != b"P6" {
        return None;
    }
    let width = next_number(bytes, &mut cursor)?;
    let height = next_number(bytes, &mut cursor)?;
    let max = next_number(bytes, &mut cursor)?;
    if max != 255 || width == 0 || height == 0 {
        return None;
    }
    // Exactly one whitespace byte separates the header from the pixels.
    let start = cursor + 1;
    let length = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(3)?;
    let rgb = bytes.get(start..start.checked_add(length)?)?.to_vec();
    Some(Picture { width, height, rgb })
}

/// Cut `rect` (output-local logical points) out of `picture`, whose pixels
/// cover a `logical_width` wide output, as BGRA — GPUI's image layout.
pub fn crop_bgra(picture: &Picture, logical_width: f32, rect: Rect) -> Option<(u32, u32, Vec<u8>)> {
    if logical_width <= 0.0 {
        return None;
    }
    let scale = picture.width as f32 / logical_width;
    let left = ((rect.x * scale).floor().max(0.0) as u32).min(picture.width);
    let top = ((rect.y * scale).floor().max(0.0) as u32).min(picture.height);
    let right = ((rect.right() * scale).ceil().max(0.0) as u32).min(picture.width);
    let bottom = ((rect.bottom() * scale).ceil().max(0.0) as u32).min(picture.height);
    if right <= left || bottom <= top {
        return None;
    }
    let (width, height) = (right - left, bottom - top);
    let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
    for y in top..bottom {
        let row = (y as usize * picture.width as usize + left as usize) * 3;
        for pixel in picture.rgb[row..row + width as usize * 3].chunks_exact(3) {
            bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 0xFF]);
        }
    }
    Some((width, height, bgra))
}

/// A whole-output picture scaled to `target_width` pixels (nearest pixel),
/// as BGRA, for the current Space's thumbnail.
pub fn thumbnail_bgra(picture: &Picture, target_width: u32) -> Option<(u32, u32, Vec<u8>)> {
    if picture.width == 0 || picture.height == 0 || target_width == 0 {
        return None;
    }
    let width = target_width.min(picture.width);
    let height =
        ((u64::from(picture.height) * u64::from(width)) / u64::from(picture.width)).max(1) as u32;
    let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        let source_y = (u64::from(y) * u64::from(picture.height) / u64::from(height)) as usize;
        for x in 0..width {
            let source_x = (u64::from(x) * u64::from(picture.width) / u64::from(width)) as usize;
            let at = (source_y * picture.width as usize + source_x) * 3;
            let pixel = &picture.rgb[at..at + 3];
            bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 0xFF]);
        }
    }
    Some((width, height, bgra))
}

/// BGRA pixels ready to become a GPUI image.
pub type Pixels = (u32, u32, Vec<u8>);

/// What one capture yields: the Space's thumbnail and a picture of every
/// window nothing covers.
#[derive(Default)]
pub struct Captured {
    pub desktop: Option<Pixels>,
    pub windows: Vec<(rmac_compositor::WindowId, Pixels)>,
}

/// Width of the Space thumbnail picture: 138 pt at up to 4× density.
const THUMBNAIL_PIXELS: u32 = 552;

/// Read the scene's output once and cut out its uncovered windows. A
/// failed capture leaves every window to its icon card.
pub fn capture_scene(scene: &crate::model::Scene) -> Captured {
    let picture = match grab_output(&scene.output.0) {
        Ok(picture) => picture,
        Err(error) => {
            eprintln!("Mission Control could not read the screen: {error}");
            return Captured::default();
        }
    };
    let windows = scene
        .windows
        .iter()
        .filter(|window| !window.occluded)
        .filter_map(|window| {
            crop_bgra(&picture, scene.width, window.frame).map(|pixels| (window.id, pixels))
        })
        .collect();
    Captured {
        desktop: thumbnail_bgra(&picture, THUMBNAIL_PIXELS),
        windows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppm(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = format!("P6\n# grim\n{width} {height}\n255\n").into_bytes();
        for i in 0..width * height {
            bytes.extend_from_slice(&[i as u8, 100, 200]);
        }
        bytes
    }

    #[test]
    fn reads_grim_ppm_and_rejects_anything_else() {
        let picture = parse_ppm(&ppm(4, 2)).unwrap();
        assert_eq!(
            (picture.width, picture.height, picture.rgb.len()),
            (4, 2, 24)
        );
        assert!(parse_ppm(b"P3\n1 1\n255\n0 0 0").is_none());
        assert!(parse_ppm(b"P6\n2 2\n255\n\x00\x00").is_none());
        assert!(parse_ppm(b"P6\n1 1\n65535\n\x00\x00\x00\x00\x00\x00").is_none());
    }

    #[test]
    fn crops_logical_rects_at_the_output_scale_as_bgra() {
        // A 4 × 2 pixel picture of a 2 × 1 point output (scale 2).
        let picture = parse_ppm(&ppm(4, 2)).unwrap();
        let (width, height, bgra) =
            crop_bgra(&picture, 2.0, Rect::new(1.0, 0.0, 1.0, 1.0)).unwrap();
        assert_eq!((width, height), (2, 2));
        // Pixel (2, 0) has index 2: RGB (2, 100, 200) → BGRA (200, 100, 2, 255).
        assert_eq!(&bgra[..4], &[200, 100, 2, 255]);
        assert_eq!(&bgra[8..12], &[200, 100, 6, 255]);
        assert!(crop_bgra(&picture, 2.0, Rect::new(5.0, 5.0, 1.0, 1.0)).is_none());
    }

    #[test]
    fn thumbnails_keep_the_aspect_and_sample_whole_pixels() {
        let picture = parse_ppm(&ppm(4, 2)).unwrap();
        let (width, height, bgra) = thumbnail_bgra(&picture, 2).unwrap();
        assert_eq!((width, height, bgra.len()), (2, 1, 8));
        assert_eq!(&bgra[4..8], &[200, 100, 2, 255]);
        let (width, _, _) = thumbnail_bgra(&picture, 99).unwrap();
        assert_eq!(width, 4);
    }
}
