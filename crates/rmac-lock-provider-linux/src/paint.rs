//! Deterministic CPU painting for the first original rmac lock frame.
//!
//! Pixels use the native-endian `wl_shm` ARGB8888 representation. The supported
//! Ubuntu targets are little-endian, where each pixel is stored as BGRA bytes.

use std::io::{self, Write};

use crate::surface::BufferLayout;

const CHUNK_PIXELS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockPalette {
    pub top: Rgb,
    pub bottom: Rgb,
    pub glow: Rgb,
    pub panel: Rgb,
    pub avatar: Rgb,
    pub accent: Rgb,
}

impl LockPalette {
    /// Original rmac midnight colors; no Apple asset or color token is used.
    pub const MIDNIGHT: Self = Self {
        top: Rgb::new(16, 25, 48),
        bottom: Rgb::new(5, 10, 23),
        glow: Rgb::new(75, 105, 170),
        panel: Rgb::new(211, 220, 237),
        avatar: Rgb::new(221, 228, 241),
        accent: Rgb::new(105, 166, 255),
    };
}

impl Rgb {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    fn interpolate(self, other: Self, numerator: u32, denominator: u32) -> Self {
        let channel = |start: u8, end: u8| {
            let start = i64::from(start);
            let delta = i64::from(end) - start;
            (start + delta * i64::from(numerator) / i64::from(denominator)) as u8
        };
        Self::new(
            channel(self.red, other.red),
            channel(self.green, other.green),
            channel(self.blue, other.blue),
        )
    }

    fn blend(self, overlay: Self, alpha: u8) -> Self {
        let alpha = u16::from(alpha);
        let inverse = 255_u16 - alpha;
        let channel = |base: u8, top: u8| {
            ((u16::from(base) * inverse + u16::from(top) * alpha + 127) / 255) as u8
        };
        Self::new(
            channel(self.red, overlay.red),
            channel(self.green, overlay.green),
            channel(self.blue, overlay.blue),
        )
    }

    fn argb8888(self) -> [u8; 4] {
        u32::from_be_bytes([255, self.red, self.green, self.blue]).to_ne_bytes()
    }
}

/// Paint exactly one opaque buffer. Allocation is bounded to a 16 KiB chunk
/// regardless of output size; the caller owns the already-validated layout.
pub fn paint_lock_frame(
    writer: &mut impl Write,
    layout: BufferLayout,
    palette: LockPalette,
) -> io::Result<()> {
    let width = layout.width();
    let height = layout.height();
    let mut chunk = vec![0_u8; CHUNK_PIXELS * 4];

    for y in 0..height {
        let mut x = 0_u32;
        while x < width {
            let pixels = (width - x).min(CHUNK_PIXELS as u32) as usize;
            for index in 0..pixels {
                let pixel =
                    paint_pixel(x + index as u32, y, width, height, layout.scale(), palette);
                let offset = index * 4;
                chunk[offset..offset + 4].copy_from_slice(&pixel.argb8888());
            }
            writer.write_all(&chunk[..pixels * 4])?;
            x += pixels as u32;
        }
    }
    Ok(())
}

fn paint_pixel(x: u32, y: u32, width: u32, height: u32, scale: u32, palette: LockPalette) -> Rgb {
    let denominator = height.saturating_sub(1).max(1);
    let mut color = palette
        .top
        .interpolate(palette.bottom, y.min(denominator), denominator);

    let center_x = i64::from(width / 2);
    let glow_y = percent(height, 42);
    let dx = i64::from(x) - center_x;
    let dy = i64::from(y) - glow_y;
    let glow_radius = u64::from(width.min(height).max(1)) * 55 / 100;
    let distance_squared = (dx * dx + dy * dy) as u64;
    let radius_squared = glow_radius.saturating_mul(glow_radius).max(1);
    if distance_squared < radius_squared {
        let strength = ((radius_squared - distance_squared) * 58 / radius_squared) as u8;
        color = color.blend(palette.glow, strength);
    }

    let avatar_y = percent(height, 38);
    let avatar_radius = u64::from((width.min(height) / 13).clamp(28 * scale, 72 * scale));
    let avatar_dy = i64::from(y) - avatar_y;
    if (dx * dx + avatar_dy * avatar_dy) as u64 <= avatar_radius * avatar_radius {
        color = color.blend(palette.avatar, 224);
    }

    let panel_half_width = percent(width, 32).min(i64::from(210 * scale));
    let panel_half_height = i64::from(22 * scale);
    let panel_center_y = percent(height, 58);
    if inside_rounded_rect(
        i64::from(x),
        i64::from(y),
        center_x,
        panel_center_y,
        panel_half_width,
        panel_half_height,
        i64::from(12 * scale),
    ) {
        color = color.blend(palette.panel, 58);
    }

    let accent_x = center_x + panel_half_width - i64::from(18 * scale);
    let accent_radius = i64::from(4 * scale);
    let accent_dx = i64::from(x) - accent_x;
    let accent_dy = i64::from(y) - panel_center_y;
    if accent_dx * accent_dx + accent_dy * accent_dy <= accent_radius * accent_radius {
        color = color.blend(palette.accent, 238);
    }

    color
}

fn percent(value: u32, numerator: u64) -> i64 {
    (u64::from(value) * numerator / 100) as i64
}

#[allow(clippy::too_many_arguments)]
fn inside_rounded_rect(
    x: i64,
    y: i64,
    center_x: i64,
    center_y: i64,
    half_width: i64,
    half_height: i64,
    radius: i64,
) -> bool {
    let inner_x = (x - center_x).abs() - (half_width - radius).max(0);
    let inner_y = (y - center_y).abs() - (half_height - radius).max(0);
    let corner_x = inner_x.max(0);
    let corner_y = inner_y.max(0);
    inner_x <= radius
        && inner_y <= radius
        && corner_x * corner_x + corner_y * corner_y <= radius * radius
}

#[cfg(test)]
mod tests {
    use std::io;

    use rmac_lock_provider::OutputId;

    use super::*;
    use crate::surface::SurfaceSet;

    fn layout(width: u32, height: u32, scale: u32) -> BufferLayout {
        let output = OutputId::new(1).unwrap();
        let mut surfaces = SurfaceSet::new();
        surfaces.add_output(output).unwrap();
        surfaces.configure(output, 1, width, height).unwrap();
        surfaces.set_scale(output, scale).unwrap_or(false);
        surfaces.begin_render(output).unwrap().layout()
    }

    #[test]
    fn paints_exactly_one_opaque_deterministic_frame() {
        let layout = layout(320, 200, 1);
        let mut first = Vec::new();
        let mut second = Vec::new();
        paint_lock_frame(&mut first, layout, LockPalette::MIDNIGHT).unwrap();
        paint_lock_frame(&mut second, layout, LockPalette::MIDNIGHT).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.len() as u64, layout.byte_len());
        assert!(first.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert_ne!(&first[0..4], &first[first.len() / 2..first.len() / 2 + 4]);
    }

    #[test]
    fn painting_propagates_a_short_destination_failure() {
        let layout = layout(64, 64, 1);
        let mut writer = FailingWriter { remaining: 100 };
        assert_eq!(
            paint_lock_frame(&mut writer, layout, LockPalette::MIDNIGHT)
                .unwrap_err()
                .kind(),
            io::ErrorKind::WriteZero
        );
    }

    struct FailingWriter {
        remaining: usize,
    }

    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let written = bytes.len().min(self.remaining);
            self.remaining -= written;
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
