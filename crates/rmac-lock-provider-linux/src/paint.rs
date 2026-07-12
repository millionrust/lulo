//! Deterministic CPU painting for the first original rmac lock frame.
//!
//! Pixels use the native-endian `wl_shm` ARGB8888 representation. The supported
//! Ubuntu targets are little-endian, where each pixel is stored as BGRA bytes.

use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;

use crate::surface::BufferLayout;

const CHUNK_PIXELS: usize = 4096;
const MAX_TEXT_RASTER_BYTES: usize = 2 * 1024 * 1024;

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
    pub error: Rgb,
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
        error: Rgb::new(255, 105, 120),
    };
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LockVisualState {
    prompt: PromptVisual,
    authentication_failed: bool,
    caps_lock_active: bool,
}

impl Default for LockVisualState {
    fn default() -> Self {
        Self {
            prompt: PromptVisual::Hidden,
            authentication_failed: false,
            caps_lock_active: false,
        }
    }
}

impl LockVisualState {
    pub const fn new(prompt: PromptVisual, authentication_failed: bool) -> Self {
        Self {
            prompt,
            authentication_failed,
            caps_lock_active: false,
        }
    }

    pub const fn with_caps_lock(mut self, active: bool) -> Self {
        self.caps_lock_active = active;
        self
    }

    pub const fn prompt(self) -> PromptVisual {
        self.prompt
    }

    pub const fn authentication_failed(self) -> bool {
        self.authentication_failed
    }

    pub const fn caps_lock_active(self) -> bool {
        self.caps_lock_active
    }
}

impl fmt::Debug for LockVisualState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LockVisualState(<redacted>)")
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum PromptVisual {
    Hidden,
    Authenticating,
    Secret { dots: u8 },
    Text { dots: u8 },
    Notice,
    Radio { selected: bool },
    Binary,
}

impl PromptVisual {
    pub fn secret(character_count: usize) -> Self {
        Self::Secret {
            dots: capped_dots(character_count),
        }
    }

    pub fn text(character_count: usize) -> Self {
        Self::Text {
            dots: capped_dots(character_count),
        }
    }
}

impl fmt::Debug for PromptVisual {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Hidden => "PromptVisual::Hidden",
            Self::Authenticating => "PromptVisual::Authenticating",
            Self::Secret { .. } => "PromptVisual::Secret(<redacted>)",
            Self::Text { .. } => "PromptVisual::Text(<redacted>)",
            Self::Notice => "PromptVisual::Notice",
            Self::Radio { .. } => "PromptVisual::Radio(<redacted>)",
            Self::Binary => "PromptVisual::Binary",
        })
    }
}

fn capped_dots(character_count: usize) -> u8 {
    character_count.min(12) as u8
}

#[derive(Clone, Eq, PartialEq)]
pub struct TextRaster {
    origin_x: i64,
    origin_y: i64,
    width: u32,
    height: u32,
    alpha: Arc<[u8]>,
}

impl TextRaster {
    pub fn new(
        origin_x: i64,
        origin_y: i64,
        width: u32,
        height: u32,
        alpha: Vec<u8>,
    ) -> Result<Self, TextRasterError> {
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(TextRasterError::InvalidDimensions)?;
        if expected == 0 || expected > MAX_TEXT_RASTER_BYTES || alpha.len() != expected {
            return Err(TextRasterError::InvalidDimensions);
        }
        Ok(Self {
            origin_x,
            origin_y,
            width,
            height,
            alpha: alpha.into(),
        })
    }

    fn alpha_at(&self, x: i64, y: i64) -> Option<u8> {
        let local_x = x.checked_sub(self.origin_x)?;
        let local_y = y.checked_sub(self.origin_y)?;
        let local_x = u32::try_from(local_x).ok()?;
        let local_y = u32::try_from(local_y).ok()?;
        if local_x >= self.width || local_y >= self.height {
            return None;
        }
        let index = usize::try_from(local_y)
            .ok()?
            .checked_mul(usize::try_from(self.width).ok()?)?
            .checked_add(usize::try_from(local_x).ok()?)?;
        self.alpha.get(index).copied()
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn origin_y(&self) -> i64 {
        self.origin_y
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn bottom(&self) -> i64 {
        self.origin_y.saturating_add(i64::from(self.height))
    }
}

impl fmt::Debug for TextRaster {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TextRaster(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextRasterError {
    InvalidDimensions,
}

impl fmt::Display for TextRasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("lock prompt raster dimensions are invalid")
    }
}

impl std::error::Error for TextRasterError {}

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
    visual: LockVisualState,
    account_text: Option<&TextRaster>,
    prompt_text: Option<&TextRaster>,
) -> io::Result<()> {
    let width = layout.width();
    let height = layout.height();
    let mut chunk = vec![0_u8; CHUNK_PIXELS * 4];

    for y in 0..height {
        let mut x = 0_u32;
        while x < width {
            let pixels = (width - x).min(CHUNK_PIXELS as u32) as usize;
            for index in 0..pixels {
                let pixel = paint_pixel(
                    x + index as u32,
                    y,
                    width,
                    height,
                    layout.scale(),
                    palette,
                    visual,
                    account_text,
                    prompt_text,
                );
                let offset = index * 4;
                chunk[offset..offset + 4].copy_from_slice(&pixel.argb8888());
            }
            writer.write_all(&chunk[..pixels * 4])?;
            x += pixels as u32;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn paint_pixel(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    scale: u32,
    palette: LockPalette,
    visual: LockVisualState,
    account_text: Option<&TextRaster>,
    prompt_text: Option<&TextRaster>,
) -> Rgb {
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
        let panel = if visual.authentication_failed {
            palette.error
        } else {
            palette.panel
        };
        color = color.blend(panel, if visual.authentication_failed { 76 } else { 58 });
    }

    let accent_x = center_x + panel_half_width - i64::from(18 * scale);
    let accent_radius = i64::from(4 * scale);
    let accent_dx = i64::from(x) - accent_x;
    let accent_dy = i64::from(y) - panel_center_y;
    if accent_dx * accent_dx + accent_dy * accent_dy <= accent_radius * accent_radius {
        color = color.blend(
            if visual.authentication_failed {
                palette.error
            } else {
                palette.accent
            },
            238,
        );
    }

    if visual.caps_lock_active
        && matches!(
            visual.prompt,
            PromptVisual::Secret { .. } | PromptVisual::Text { .. }
        )
    {
        let scale = i64::from(scale);
        let indicator_x = center_x - panel_half_width + 18 * scale;
        let indicator_y = panel_center_y;
        let local_x = (i64::from(x) - indicator_x).abs();
        let local_y = i64::from(y) - (indicator_y - 7 * scale);
        let arrow_head = (0..=6 * scale).contains(&local_y) && local_x <= local_y;
        let arrow_stem = local_x <= scale
            && (indicator_y - scale..=indicator_y + 6 * scale).contains(&i64::from(y));
        if arrow_head || arrow_stem {
            color = color.blend(palette.error, 232);
        }
    }

    color = paint_prompt(
        color,
        i64::from(x),
        i64::from(y),
        center_x,
        panel_center_y,
        scale,
        palette,
        visual.prompt,
    );

    for text in [account_text, prompt_text].into_iter().flatten() {
        if let Some(alpha) = text.alpha_at(i64::from(x), i64::from(y)) {
            color = color.blend(palette.panel, alpha);
        }
    }

    color
}

#[allow(clippy::too_many_arguments)]
fn paint_prompt(
    mut color: Rgb,
    x: i64,
    y: i64,
    center_x: i64,
    center_y: i64,
    scale: u32,
    palette: LockPalette,
    prompt: PromptVisual,
) -> Rgb {
    let scale = i64::from(scale);
    match prompt {
        PromptVisual::Authenticating => {
            for offset in [-10_i64, 0, 10] {
                if inside_circle(x, y, center_x + offset * scale, center_y, 2 * scale) {
                    color = color.blend(palette.panel, 176);
                }
            }
        }
        PromptVisual::Secret { dots } | PromptVisual::Text { dots } => {
            let count = i64::from(dots);
            let spacing = 12 * scale;
            let first = center_x - (count.saturating_sub(1) * spacing / 2);
            for index in 0..count {
                if inside_circle(x, y, first + index * spacing, center_y, 3 * scale) {
                    color = color.blend(palette.panel, 224);
                    break;
                }
            }
        }
        PromptVisual::Notice => {
            for (offset, half_width) in [(-7, 34), (0, 42), (7, 28)] {
                if (y - (center_y + i64::from(offset) * scale)).abs() <= scale
                    && (x - center_x).abs() <= i64::from(half_width) * scale
                {
                    color = color.blend(palette.panel, 188);
                }
            }
        }
        PromptVisual::Radio { selected } => {
            let selection_x = center_x + if selected { 9 * scale } else { -9 * scale };
            if inside_rounded_rect(x, y, center_x, center_y, 19 * scale, 10 * scale, 10 * scale) {
                color = color.blend(palette.panel, 92);
            }
            if inside_circle(x, y, selection_x, center_y, 7 * scale) {
                color = color.blend(palette.accent, 230);
            }
        }
        PromptVisual::Binary => {
            for offset in [-9_i64, 0, 9] {
                if (x - (center_x + offset * scale)).abs() <= 2 * scale
                    && (y - center_y).abs() <= 7 * scale
                {
                    color = color.blend(palette.panel, 196);
                }
            }
        }
        PromptVisual::Hidden => {}
    }
    color
}

fn inside_circle(x: i64, y: i64, center_x: i64, center_y: i64, radius: i64) -> bool {
    let dx = x - center_x;
    let dy = y - center_y;
    dx * dx + dy * dy <= radius * radius
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
        paint_lock_frame(
            &mut first,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::default(),
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut second,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::default(),
            None,
            None,
        )
        .unwrap();

        assert_eq!(first, second);
        assert_eq!(first.len() as u64, layout.byte_len());
        assert!(first.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert_ne!(&first[0..4], &first[first.len() / 2..first.len() / 2 + 4]);
    }

    #[test]
    fn prompt_and_failure_states_change_pixels_but_redact_diagnostics() {
        let layout = layout(320, 200, 1);
        let mut secret = Vec::new();
        let mut failed = Vec::new();
        let mut caps_lock = Vec::new();
        let mut hidden = Vec::new();
        let mut hidden_caps_lock = Vec::new();
        let mut authenticating = Vec::new();
        paint_lock_frame(
            &mut secret,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::new(PromptVisual::secret(9), false),
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut failed,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::new(PromptVisual::Hidden, true),
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut caps_lock,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::new(PromptVisual::secret(9), false).with_caps_lock(true),
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut hidden,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::default(),
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut hidden_caps_lock,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::default().with_caps_lock(true),
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut authenticating,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::new(PromptVisual::Authenticating, false),
            None,
            None,
        )
        .unwrap();
        assert_ne!(secret, failed);
        assert_ne!(secret, caps_lock);
        assert_ne!(hidden, authenticating);
        assert_eq!(hidden, hidden_caps_lock);
        let debug = format!(
            "{:?} {:?}",
            LockVisualState::new(PromptVisual::secret(9), false).with_caps_lock(true),
            PromptVisual::Radio { selected: true }
        );
        assert!(!debug.contains('9'));
        assert!(!debug.contains("true"));
    }

    #[test]
    fn painting_propagates_a_short_destination_failure() {
        let layout = layout(64, 64, 1);
        let mut writer = FailingWriter { remaining: 100 };
        assert_eq!(
            paint_lock_frame(
                &mut writer,
                layout,
                LockPalette::MIDNIGHT,
                LockVisualState::default(),
                None,
                None,
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::WriteZero
        );
    }

    #[test]
    fn bounded_text_raster_blends_without_exposing_pixels_to_debug() {
        let layout = layout(64, 64, 1);
        let mut plain = Vec::new();
        let mut account_only = Vec::new();
        let mut both = Vec::new();
        let account = TextRaster::new(30, 30, 2, 2, vec![255; 4]).unwrap();
        let prompt = TextRaster::new(40, 40, 2, 2, vec![255; 4]).unwrap();
        paint_lock_frame(
            &mut plain,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::default(),
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut account_only,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::default(),
            Some(&account),
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut both,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::default(),
            Some(&account),
            Some(&prompt),
        )
        .unwrap();
        assert_ne!(plain, account_only);
        assert_ne!(account_only, both);
        assert_eq!(format!("{account:?}"), "TextRaster(<redacted>)");
        assert_eq!(format!("{prompt:?}"), "TextRaster(<redacted>)");
        assert_eq!(
            TextRaster::new(0, 0, 2, 2, vec![0; 3]),
            Err(TextRasterError::InvalidDimensions)
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
