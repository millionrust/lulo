//! Deterministic CPU painting for the first original rmac lock frame.
//!
//! Pixels use the native-endian `wl_shm` ARGB8888 representation. The supported
//! Ubuntu targets are little-endian, where each pixel is stored as BGRA bytes.

use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use crate::surface::BufferLayout;

const CHUNK_PIXELS: usize = 4096;
const MAX_TEXT_RASTER_BYTES: usize = 2 * 1024 * 1024;
const WHITE: Rgb = Rgb::new(255, 255, 255);
const BLACK: Rgb = Rgb::new(0, 0, 0);

/// The macOS 26 lock screen in logical points (`design-lab/lock.html`).
/// Clock rows hang from the top edge and the identity block from the bottom
/// edge, like the Mac, instead of scaling with the output. Nothing here could
/// be measured (the owner's Mac cannot be locked for a capture), so every
/// value is `S` until a photo replaces it.
pub(crate) mod layout {
    // The text rows are only rasterized by the Linux renderer.
    #![cfg_attr(not(target_os = "linux"), allow(dead_code))]

    /// Date line centre, from the top edge. S
    pub(crate) const DATE_CENTER_FROM_TOP: u32 = 70;
    /// Clock line centre, from the top edge. S
    pub(crate) const CLOCK_CENTER_FROM_TOP: u32 = 146;
    /// Monogram avatar diameter. S
    pub(crate) const AVATAR_DIAMETER: u32 = 56;
    /// Avatar centre, from the bottom edge. S
    pub(crate) const AVATAR_CENTER_FROM_BOTTOM: u32 = 196;
    /// Account name centre, from the bottom edge. S
    pub(crate) const ACCOUNT_CENTER_FROM_BOTTOM: u32 = 148;
    /// Password pill. S
    pub(crate) const FIELD_WIDTH: u32 = 180;
    pub(crate) const FIELD_HEIGHT: u32 = 28;
    pub(crate) const FIELD_CENTER_FROM_BOTTOM: u32 = 110;
    /// Gap between the pill and the guidance text under it. S
    pub(crate) const GUIDANCE_GAP: u32 = 8;
    /// Submit circle: diameter 20, centre 14 in from the pill's right end. S
    pub(crate) const SUBMIT_RADIUS: u32 = 10;
    pub(crate) const SUBMIT_INSET: u32 = 14;
    /// Password bullets: 6 pt dots on a 10 pt pitch, first centre 14 in. S
    pub(crate) const DOT_RADIUS: u32 = 3;
    pub(crate) const DOT_PITCH: u32 = 10;
    pub(crate) const DOT_INSET: u32 = 14;
    /// Wrong-password shake: ±8 pt, three cycles in 400 ms. S
    pub(crate) const SHAKE_AMPLITUDE: f64 = 8.0;
    pub(crate) const SHAKE_CYCLES: f64 = 3.0;
    pub(crate) const SHAKE_MILLIS: u64 = 400;

    pub(crate) fn from_bottom(height: u32, scale: u32, points: u32) -> i64 {
        i64::from(height) - i64::from(points.saturating_mul(scale.max(1)))
    }

    pub(crate) fn from_top(scale: u32, points: u32) -> i64 {
        i64::from(points.saturating_mul(scale.max(1)))
    }
}

/// Horizontal field offset in logical points for a shake that started
/// `elapsed` ago, or `None` once the shake has finished.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn shake_offset(elapsed: Duration) -> Option<i8> {
    let total = Duration::from_millis(layout::SHAKE_MILLIS);
    if elapsed >= total {
        return None;
    }
    let progress = elapsed.as_secs_f64() / total.as_secs_f64();
    let phase = progress * layout::SHAKE_CYCLES * std::f64::consts::TAU;
    Some((layout::SHAKE_AMPLITUDE * phase.sin()).round() as i8)
}

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
    pub secondary_glow: Rgb,
    pub panel: Rgb,
    pub avatar: Rgb,
    pub accent: Rgb,
    pub error: Rgb,
}

impl LockPalette {
    /// Original rmac Aurora colors, shared with the default desktop wallpaper.
    pub const MIDNIGHT: Self = Self {
        top: Rgb::new(16, 22, 47),
        bottom: Rgb::new(33, 43, 92),
        glow: Rgb::new(34, 166, 161),
        secondary_glow: Rgb::new(217, 108, 157),
        panel: Rgb::new(244, 246, 252),
        avatar: Rgb::new(33, 51, 79),
        accent: Rgb::new(118, 181, 255),
        error: Rgb::new(255, 105, 120),
    };
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LockVisualState {
    prompt: PromptVisual,
    authentication_failed: bool,
    caps_lock_active: bool,
    keyboard_focused: bool,
    shake_offset: i8,
}

impl Default for LockVisualState {
    fn default() -> Self {
        Self {
            prompt: PromptVisual::Hidden,
            authentication_failed: false,
            caps_lock_active: false,
            keyboard_focused: false,
            shake_offset: 0,
        }
    }
}

impl LockVisualState {
    pub const fn new(prompt: PromptVisual, authentication_failed: bool) -> Self {
        Self {
            prompt,
            authentication_failed,
            caps_lock_active: false,
            keyboard_focused: false,
            shake_offset: 0,
        }
    }

    /// Displace the password pill horizontally by `points` for one frame of
    /// the wrong-password shake.
    pub const fn with_shake_offset(mut self, points: i8) -> Self {
        self.shake_offset = points;
        self
    }

    pub const fn shake_offset(self) -> i8 {
        self.shake_offset
    }

    pub const fn with_caps_lock(mut self, active: bool) -> Self {
        self.caps_lock_active = active;
        self
    }

    pub const fn with_keyboard_focus(mut self, focused: bool) -> Self {
        self.keyboard_focused = focused;
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

    pub const fn keyboard_focused(self) -> bool {
        self.keyboard_focused
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

    /// Position of row `y` inside the raster box, 0 at the top and 255 at the
    /// bottom, for vertical gradients clipped to the glyphs.
    fn vertical_fraction(&self, y: i64) -> u32 {
        let local = (y - self.origin_y).clamp(0, i64::from(self.height));
        (local * 255 / i64::from(self.height.max(1))) as u32
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

/// A small, fixed-size RGB8 raster this process only ever reads as raw,
/// length-checked bytes (LOCK-01; `crate::picture`) — never as a decoded
/// image format. Used for the blurred wallpaper background and the real
/// account picture, in place of the Aurora gradient and the monogram disc.
#[derive(Clone)]
pub struct PictureRaster {
    width: u32,
    height: u32,
    rgb: Arc<[u8]>,
}

impl PictureRaster {
    /// `rgb` must be exactly `width * height * 3` bytes (row-major, no
    /// padding, no alpha). Anything else is rejected rather than truncated
    /// or reinterpreted.
    pub fn new(width: u32, height: u32, rgb: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let expected = u64::from(width)
            .checked_mul(u64::from(height))?
            .checked_mul(3)?;
        if rgb.len() as u64 != expected {
            return None;
        }
        Some(Self {
            width,
            height,
            rgb: rgb.into(),
        })
    }

    /// Nearest-neighbour sample at a fractional position, `u` and `v` each
    /// meant to range over `[0, 1)`; out-of-range values clamp to the edge.
    fn sample(&self, u: f32, v: f32) -> Rgb {
        let clamp01 = |value: f32| value.clamp(0.0, 0.999_999);
        let x = (clamp01(u) * self.width as f32) as u32;
        let y = (clamp01(v) * self.height as f32) as u32;
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let offset = ((y * self.width + x) * 3) as usize;
        match self.rgb.get(offset..offset + 3) {
            Some([red, green, blue]) => Rgb::new(*red, *green, *blue),
            _ => BLACK,
        }
    }
}

impl fmt::Debug for PictureRaster {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PictureRaster(<redacted>)")
    }
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
    visual: LockVisualState,
    account_text: Option<&TextRaster>,
    prompt_text: Option<&TextRaster>,
) -> io::Result<()> {
    paint_lock_frame_with_text(
        writer,
        layout,
        palette,
        visual,
        LockTexts {
            account: account_text,
            prompt: prompt_text,
            ..LockTexts::default()
        },
    )
}

/// Every text raster one lock frame can carry. Each is optional so a frame
/// still paints while fonts are unavailable.
#[derive(Clone, Copy, Default)]
pub struct LockTexts<'a> {
    pub clock: Option<&'a TextRaster>,
    pub date: Option<&'a TextRaster>,
    pub avatar: Option<&'a TextRaster>,
    pub account: Option<&'a TextRaster>,
    /// "Enter Password", drawn inside the empty pill and moved by the shake.
    pub placeholder: Option<&'a TextRaster>,
    pub prompt: Option<&'a TextRaster>,
    /// The user's current wallpaper, blurred (LOCK-01). `None` paints the
    /// Aurora gradient, exactly as before — the documented fallback for a
    /// missing cache file or a `Reduce Transparency`-equivalent restricted
    /// read (crate::picture).
    pub background: Option<&'a PictureRaster>,
    /// The account picture (LOCK-01). `None` keeps the monogram disc and
    /// letter (`avatar` above).
    pub picture: Option<&'a PictureRaster>,
}

impl fmt::Debug for LockTexts<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LockTexts(<redacted>)")
    }
}

/// Paint one lock frame with the complete modern lock-screen text hierarchy.
pub fn paint_lock_frame_with_text(
    writer: &mut impl Write,
    layout: BufferLayout,
    palette: LockPalette,
    visual: LockVisualState,
    texts: LockTexts<'_>,
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
                    texts,
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
    texts: LockTexts<'_>,
) -> Rgb {
    let center_x = i64::from(width / 2);
    let mut color = match texts.background {
        // LOCK-01: the real wallpaper, already blurred by the small
        // fixed-size thumbnail it was sampled down to (crate::picture,
        // rmac_wallpaper_image::lock_thumbnail). No synthetic glow: a real
        // photo already has its own colour, unlike the Aurora artwork below.
        Some(background) => background.sample(
            x as f32 / width.max(1) as f32,
            y as f32 / height.max(1) as f32,
        ),
        None => {
            let denominator = height.saturating_sub(1).max(1);
            let mut color =
                palette
                    .top
                    .interpolate(palette.bottom, y.min(denominator), denominator);

            let glow_x = percent(width, 70);
            let glow_y = percent(height, 32);
            let dx = i64::from(x) - glow_x;
            let dy = i64::from(y) - glow_y;
            let glow_radius = u64::from(width.min(height).max(1)) * 55 / 100;
            let distance_squared = (dx * dx + dy * dy) as u64;
            let radius_squared = glow_radius.saturating_mul(glow_radius).max(1);
            if distance_squared < radius_squared {
                let strength = ((radius_squared - distance_squared) * 58 / radius_squared) as u8;
                color = color.blend(palette.glow, strength);
            }

            let secondary_x = percent(width, 20);
            let secondary_y = percent(height, 72);
            let secondary_dx = i64::from(x) - secondary_x;
            let secondary_dy = i64::from(y) - secondary_y;
            let secondary_radius = u64::from(width.min(height).max(1)) * 62 / 100;
            let secondary_distance =
                (secondary_dx * secondary_dx + secondary_dy * secondary_dy) as u64;
            let secondary_radius_squared = secondary_radius.saturating_mul(secondary_radius).max(1);
            if secondary_distance < secondary_radius_squared {
                let strength = ((secondary_radius_squared - secondary_distance) * 52
                    / secondary_radius_squared) as u8;
                color = color.blend(palette.secondary_glow, strength);
            }
            color
        }
    };

    // The Aurora artwork is already made of soft gradients, so it reads as the
    // Mac's blurred wallpaper; the Mac then lays a black 20 % veil over it (S).
    // The same veil applies over a real wallpaper thumbnail above, matching
    // how the Mac darkens its own blurred wallpaper behind the lock UI.
    color = color.blend(BLACK, 51);

    let px = i64::from(x);
    let py = i64::from(y);
    let unit = i64::from(scale.max(1));
    let shake = i64::from(visual.shake_offset) * unit;

    // The account picture (LOCK-01) when one is cached, else the monogram
    // fallback: a grey vertical gradient disc, like a Contacts monogram
    // without a picture (S).
    let avatar_center_y = layout::from_bottom(height, scale, layout::AVATAR_CENTER_FROM_BOTTOM);
    let avatar_radius = i64::from(layout::AVATAR_DIAMETER / 2) * unit;
    if inside_circle(px, py, center_x, avatar_center_y, avatar_radius) {
        let top = avatar_center_y - avatar_radius;
        let span = (2 * avatar_radius).max(1);
        match texts.picture {
            Some(picture) => {
                let left = center_x - avatar_radius;
                let u = (px - left) as f32 / span as f32;
                let v = (py - top) as f32 / span as f32;
                color = picture.sample(u, v);
            }
            None => {
                let position = ((py - top).clamp(0, span) * 255 / span) as u32;
                color = Rgb::new(165, 171, 184).interpolate(Rgb::new(132, 137, 147), position, 255);
            }
        }
    }

    // The glass password pill, displaced by the wrong-password shake.
    let geometry = PromptGeometry::new(width, height, scale);
    let field_x = center_x + shake;
    let field_y = geometry.center_y;
    let half_width = geometry.half_width;
    let half_height = geometry.half_height;
    if inside_rounded_rect(
        px,
        py,
        field_x,
        field_y,
        half_width,
        half_height,
        half_height,
    ) {
        color = color.blend(WHITE, 51);
        let inner = inside_rounded_rect(
            px,
            py,
            field_x,
            field_y,
            half_width - unit,
            half_height - unit,
            half_height - unit,
        );
        if !inner {
            let interactive = matches!(
                visual.prompt,
                PromptVisual::Secret { .. }
                    | PromptVisual::Text { .. }
                    | PromptVisual::Notice
                    | PromptVisual::Radio { .. }
            );
            // A brighter rim is the only focus cue; the Mac draws no accent
            // ring on the lock screen (S).
            let rim = if visual.keyboard_focused && interactive {
                110
            } else {
                41
            };
            color = color.blend(WHITE, rim);
        }
    }

    let submit_visible = prompt_can_submit(visual.prompt);
    if submit_visible {
        let submit_x = geometry.submit_x + shake;
        if inside_circle(px, py, submit_x, field_y, geometry.submit_radius) {
            color = color.blend(WHITE, 64);
            let arrow_dx = px - submit_x;
            let arrow_dy = py - field_y;
            // A 1.5 pt stroke: 1 px at 1x, 3 px at 2x.
            let on_stroke = |distance: i64| distance.abs() * 4 <= 3 * unit;
            let shaft = (-4 * unit..=3 * unit).contains(&arrow_dx) && on_stroke(arrow_dy);
            let head_x = arrow_dx + unit;
            let head =
                (0..=4 * unit).contains(&head_x) && on_stroke(arrow_dy.abs() - (4 * unit - head_x));
            if shaft || head {
                color = color.blend(WHITE, 240);
            }
        }
    }

    if visual.caps_lock_active
        && matches!(
            visual.prompt,
            PromptVisual::Secret { .. } | PromptVisual::Text { .. }
        )
    {
        // ⇪ at the pill's right end, left of the submit arrow when it shows.
        let indicator_x = field_x + half_width
            - i64::from(layout::SUBMIT_INSET) * unit
            - if submit_visible { 24 * unit } else { 0 };
        let local_x = (px - indicator_x).abs();
        let local_y = py - (field_y - 6 * unit);
        let arrow_head = (0..=5 * unit).contains(&local_y) && local_x <= local_y;
        let arrow_stem = local_x <= unit && (field_y - unit..=field_y + 3 * unit).contains(&py);
        let base = local_x <= 3 * unit && (field_y + 5 * unit..=field_y + 6 * unit).contains(&py);
        if arrow_head || arrow_stem || base {
            color = color.blend(WHITE, 178);
        }
    }

    color = paint_prompt(
        color,
        px,
        py,
        field_x,
        field_y,
        half_width,
        scale,
        palette,
        visual.prompt,
    );

    // Clock: a glass fill (white 92 % → 62 % down the line box) over a 1 pt
    // black 18 % shadow (S).
    if let Some(clock) = texts.clock {
        if let Some(alpha) = clock.alpha_at(px, py - unit) {
            color = color.blend(BLACK, scale_alpha(alpha, 46));
        }
        if let Some(alpha) = clock.alpha_at(px, py) {
            let fraction = clock.vertical_fraction(py);
            let strength = 235 - (235 - 158) * fraction / 255;
            color = color.blend(WHITE, scale_alpha(alpha, strength));
        }
    }
    let placeholder_visible = matches!(
        visual.prompt,
        PromptVisual::Hidden | PromptVisual::Secret { dots: 0 }
    );
    let placeholder = texts.placeholder.filter(|_| placeholder_visible);
    for (text, strength, sample_x) in [
        (texts.date, 230, px),
        // The monogram letter only makes sense over the grey disc fallback.
        (texts.avatar.filter(|_| texts.picture.is_none()), 255, px),
        (texts.account, 255, px),
        (placeholder, 153, px - shake),
        (texts.prompt, 204, px),
    ] {
        let Some(text) = text else {
            continue;
        };
        if let Some(alpha) = text.alpha_at(sample_x, py) {
            color = color.blend(WHITE, scale_alpha(alpha, strength));
        }
    }

    color
}

fn scale_alpha(alpha: u8, strength: u32) -> u8 {
    (u32::from(alpha) * strength.min(255) / 255) as u8
}

/// Whether the submit arrow is shown and clickable. Like the Mac, an empty
/// password or text field has no arrow; Return still submits it.
pub(crate) fn prompt_can_submit(prompt: PromptVisual) -> bool {
    match prompt {
        PromptVisual::Secret { dots } | PromptVisual::Text { dots } => dots > 0,
        PromptVisual::Notice | PromptVisual::Radio { .. } => true,
        PromptVisual::Hidden | PromptVisual::Authenticating | PromptVisual::Binary => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromptGeometry {
    pub(crate) center_y: i64,
    pub(crate) half_width: i64,
    pub(crate) half_height: i64,
    pub(crate) submit_x: i64,
    pub(crate) submit_radius: i64,
}

impl PromptGeometry {
    pub(crate) fn new(width: u32, height: u32, scale: u32) -> Self {
        let scale = scale.max(1);
        let unit = i64::from(scale);
        let half_width = percent(width, 45).min(i64::from(layout::FIELD_WIDTH / 2) * unit);
        Self {
            center_y: layout::from_bottom(height, scale, layout::FIELD_CENTER_FROM_BOTTOM),
            half_width,
            half_height: i64::from(layout::FIELD_HEIGHT / 2) * unit,
            submit_x: i64::from(width / 2) + half_width - i64::from(layout::SUBMIT_INSET) * unit,
            submit_radius: i64::from(layout::SUBMIT_RADIUS) * unit,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_prompt(
    mut color: Rgb,
    x: i64,
    y: i64,
    center_x: i64,
    center_y: i64,
    half_width: i64,
    scale: u32,
    palette: LockPalette,
    prompt: PromptVisual,
) -> Rgb {
    let scale = i64::from(scale.max(1));
    match prompt {
        PromptVisual::Authenticating => {
            for offset in [-10_i64, 0, 10] {
                if inside_circle(x, y, center_x + offset * scale, center_y, 2 * scale) {
                    color = color.blend(WHITE, 176);
                }
            }
        }
        PromptVisual::Secret { dots } | PromptVisual::Text { dots } => {
            // Bullets start at the left like a macOS secure text field (S).
            let first = center_x - half_width + i64::from(layout::DOT_INSET) * scale;
            let pitch = i64::from(layout::DOT_PITCH) * scale;
            let radius = i64::from(layout::DOT_RADIUS) * scale;
            for index in 0..i64::from(dots) {
                if inside_circle(x, y, first + index * pitch, center_y, radius) {
                    color = color.blend(WHITE, 224);
                    break;
                }
            }
        }
        PromptVisual::Notice => {
            for (offset, line_half_width) in [(-7, 34), (0, 42), (7, 28)] {
                if (y - (center_y + i64::from(offset) * scale)).abs() <= scale
                    && (x - center_x).abs() <= i64::from(line_half_width) * scale
                {
                    color = color.blend(WHITE, 188);
                }
            }
        }
        PromptVisual::Radio { selected } => {
            let selection_x = center_x + if selected { 9 * scale } else { -9 * scale };
            if inside_rounded_rect(x, y, center_x, center_y, 19 * scale, 10 * scale, 10 * scale) {
                color = color.blend(WHITE, 92);
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
                    color = color.blend(WHITE, 196);
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
        let mut focused = Vec::new();
        let mut focused_authenticating = Vec::new();
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
        paint_lock_frame(
            &mut focused,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::new(PromptVisual::secret(9), false).with_keyboard_focus(true),
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut focused_authenticating,
            layout,
            LockPalette::MIDNIGHT,
            LockVisualState::new(PromptVisual::Authenticating, false).with_keyboard_focus(true),
            None,
            None,
        )
        .unwrap();
        assert_ne!(secret, failed);
        assert_ne!(secret, caps_lock);
        assert_ne!(hidden, authenticating);
        assert_ne!(secret, focused);
        assert_eq!(authenticating, focused_authenticating);
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

    #[test]
    fn wrong_password_shake_moves_the_field_and_settles() {
        assert_eq!(shake_offset(Duration::ZERO), Some(0));
        let peak = (0..400)
            .filter_map(|millis| shake_offset(Duration::from_millis(millis)))
            .map(i8::unsigned_abs)
            .max();
        assert_eq!(peak, Some(8));
        assert_eq!(shake_offset(Duration::from_millis(400)), None);

        let layout = layout(320, 200, 1);
        let mut still = Vec::new();
        let mut shaken = Vec::new();
        let visual = LockVisualState::new(PromptVisual::Hidden, true);
        paint_lock_frame(
            &mut still,
            layout,
            LockPalette::MIDNIGHT,
            visual,
            None,
            None,
        )
        .unwrap();
        paint_lock_frame(
            &mut shaken,
            layout,
            LockPalette::MIDNIGHT,
            visual.with_shake_offset(8),
            None,
            None,
        )
        .unwrap();
        assert_ne!(still, shaken);
    }

    #[test]
    fn submit_arrow_needs_input_like_the_mac() {
        assert!(!prompt_can_submit(PromptVisual::secret(0)));
        assert!(prompt_can_submit(PromptVisual::secret(1)));
        assert!(!prompt_can_submit(PromptVisual::text(0)));
        assert!(prompt_can_submit(PromptVisual::Notice));
        assert!(!prompt_can_submit(PromptVisual::Binary));
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
