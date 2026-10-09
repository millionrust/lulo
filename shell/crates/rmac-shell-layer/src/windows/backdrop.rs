//! The blurred material behind the Dock's shelf, open menus and Control
//! Centre on Windows (ADR 0023, "Phase 3 revised: shared shell views").
//!
//! On Lulo OS niri blurs what lies under these surfaces (`blur { passes 3
//! offset 3 saturation 1.5 }` in `shell.kdl`) and the view draws its tint
//! over it. DWM's blur-behind is not that: on Windows Server it paints black,
//! acrylic adds its own grey and noise, and neither follows a window region
//! once GPUI draws through DirectComposition (WIN-OS-49). So the view draws
//! the blur itself: the desktop's wallpaper, which the shared desktop view
//! already decodes at the screen's size, is shrunk, blurred and saturated
//! once per wallpaper change ([`set_wallpaper`]), and each blurred surface
//! shows the part of it under its own rectangle ([`element`]). App windows
//! under a menu are not sampled, as niri's cheaper wallpaper-only "xray"
//! backdrop does not sample them either.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use gpui::{img, px, AnyElement, App, IntoElement, RenderImage, Styled, StyledImage, Window};
use uuid::Uuid;

/// The blurred copy is kept at a quarter of the screen's pixels; the blur
/// hides the scaling and the copy stays small (≈ 0.2 MB at 1024 × 768).
const SHRINK: u32 = 4;
/// Box-blur passes and radius (in the shrunk copy's pixels): three passes
/// approximate a Gaussian of about niri's dual-Kawase spread.
const PASSES: usize = 3;
const RADIUS: usize = 3;
/// What the Lulo OS scene shows under the Dock's shelf matches the
/// wallpaper's own colours (niri's `saturation 1.5` does not show at the
/// shelf's tint), so the copy keeps them (`Shell scenes`, WIN-OS-59).
const SATURATION: f32 = 1.0;

struct Blurred {
    output: Uuid,
    /// BGRA, as GPUI draws images, a quarter of the screen's size.
    pixels: image::RgbaImage,
}

static WALLPAPERS: RwLock<Vec<Arc<Blurred>>> = RwLock::new(Vec::new());
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The desktop view decoded `output`'s wallpaper: `bgra` is `width` ×
/// `height` physical pixels. Called off the UI thread; the views pick the
/// new copy up on their next frame.
pub fn set_wallpaper(output: Uuid, width: u32, height: u32, bgra: &[u8]) {
    let Some(pixels) = blur(width, height, bgra) else {
        return;
    };
    let blurred = Arc::new(Blurred { output, pixels });
    if let Ok(mut wallpapers) = WALLPAPERS.write() {
        wallpapers.retain(|wallpaper| wallpaper.output != output);
        wallpapers.push(blurred);
    }
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

fn blur(width: u32, height: u32, bgra: &[u8]) -> Option<image::RgbaImage> {
    if width == 0 || height == 0 || bgra.len() != (width as usize) * (height as usize) * 4 {
        return None;
    }
    let small_width = width.div_ceil(SHRINK);
    let small_height = height.div_ceil(SHRINK);
    let (w, h) = (small_width as usize, small_height as usize);
    // Average each SHRINK × SHRINK block.
    let mut channels = vec![0f32; w * h * 4];
    let mut counts = vec![0f32; w * h];
    for y in 0..height as usize {
        let row = &bgra[y * width as usize * 4..(y + 1) * width as usize * 4];
        let small_y = y / SHRINK as usize;
        for (x, pixel) in row.chunks_exact(4).enumerate() {
            let index = small_y * w + x / SHRINK as usize;
            for channel in 0..4 {
                channels[index * 4 + channel] += f32::from(pixel[channel]);
            }
            counts[index] += 1.0;
        }
    }
    for (index, count) in counts.iter().enumerate() {
        for channel in 0..4 {
            channels[index * 4 + channel] /= count.max(1.0);
        }
    }
    let mut scratch = vec![0f32; channels.len()];
    for _ in 0..PASSES {
        box_pass(&channels, &mut scratch, w, h, true);
        box_pass(&scratch, &mut channels, w, h, false);
    }
    let mut out = image::RgbaImage::new(small_width, small_height);
    for (index, pixel) in out.pixels_mut().enumerate() {
        let (b, g, r) = (
            channels[index * 4],
            channels[index * 4 + 1],
            channels[index * 4 + 2],
        );
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let saturate = |value: f32| (luma + (value - luma) * SATURATION).clamp(0.0, 255.0) as u8;
        pixel.0 = [saturate(b), saturate(g), saturate(r), 255];
    }
    Some(out)
}

/// One box-blur pass along rows (`horizontal`) or columns, clamped at the
/// edges as a blur of the screen is.
fn box_pass(source: &[f32], target: &mut [f32], w: usize, h: usize, horizontal: bool) {
    let (lines, length) = if horizontal { (h, w) } else { (w, h) };
    let at = |line: usize, position: usize| {
        if horizontal {
            line * w + position
        } else {
            position * w + line
        }
    };
    let span = (2 * RADIUS + 1) as f32;
    for line in 0..lines {
        for channel in 0..4 {
            let value = |position: isize| {
                let clamped = position.clamp(0, length as isize - 1) as usize;
                source[at(line, clamped) * 4 + channel]
            };
            let mut sum: f32 = (-(RADIUS as isize)..=RADIUS as isize).map(value).sum();
            for position in 0..length {
                target[at(line, position) * 4 + channel] = sum / span;
                sum += value(position as isize + RADIUS as isize + 1);
                sum -= value(position as isize - RADIUS as isize);
            }
        }
    }
}

/// A crop of the blurred copy: its output, its rectangle in the shrunk
/// copy, and the copy's generation.
type CropKey = (Uuid, [u32; 4], u64);

thread_local! {
    /// Crops already made. Few surfaces are blurred at once.
    static CROPS: RefCell<Vec<(CropKey, Arc<RenderImage>)>> = const { RefCell::new(Vec::new()) };
}

/// The blurred wallpaper under `window`, cut to `radius`, sized to the
/// window; `None` before the desktop has a wallpaper.
pub fn element(window: &Window, cx: &App, radius: f32) -> Option<AnyElement> {
    let display = window.display(cx)?;
    let output = crate::display_uuid(&*display)?;
    let wallpaper = WALLPAPERS
        .read()
        .ok()?
        .iter()
        .find(|wallpaper| wallpaper.output == output)?
        .clone();
    let generation = GENERATION.load(Ordering::Acquire);
    let scale = window.scale_factor();
    let screen = display.bounds();
    // Where Windows has the window (physical pixels): GPUI's own bounds of
    // a layer window placed after it opened can still read its opening
    // place, which sampled the wrong part of the wallpaper.
    let mut placed = windows::Win32::Foundation::RECT::default();
    let hwnd = super::surface::hwnd(window)?;
    // SAFETY: reads the rectangle of a window this process owns.
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut placed) }.ok()?;
    let origin_x = screen.origin.x.as_f32() * scale;
    let origin_y = screen.origin.y.as_f32() * scale;
    // The window's rectangle on its screen, in the shrunk copy's pixels.
    let (small_width, small_height) = wallpaper.pixels.dimensions();
    let to_small = |physical: f32, origin: f32, small: u32| {
        ((physical - origin) / SHRINK as f32).clamp(0.0, small as f32)
    };
    let left = to_small(placed.left as f32, origin_x, small_width);
    let top = to_small(placed.top as f32, origin_y, small_height);
    let right = to_small(placed.right as f32, origin_x, small_width);
    let bottom = to_small(placed.bottom as f32, origin_y, small_height);
    let rect = [
        left.floor() as u32,
        top.floor() as u32,
        (right.ceil() as u32).max(left.floor() as u32 + 1),
        (bottom.ceil() as u32).max(top.floor() as u32 + 1),
    ];
    let key = (output, rect, generation);
    let image = CROPS.with(|crops| {
        let mut crops = crops.borrow_mut();
        if let Some((_, image)) = crops.iter().find(|(found, _)| *found == key) {
            return Some(image.clone());
        }
        let width = rect[2].min(small_width).saturating_sub(rect[0]).max(1);
        let height = rect[3].min(small_height).saturating_sub(rect[1]).max(1);
        let crop = image::imageops::crop_imm(&wallpaper.pixels, rect[0], rect[1], width, height)
            .to_image();
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(crop)]));
        crops.retain(|((found_output, _, found_generation), _)| {
            *found_generation == generation && *found_output == output
        });
        crops.truncate(7);
        crops.push((key, image.clone()));
        Some(image)
    })?;
    Some(
        img(image)
            .absolute()
            .inset_0()
            .size_full()
            .rounded(px(radius))
            .object_fit(gpui::ObjectFit::Fill)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_wallpaper_stays_flat_and_a_quarter_of_its_size() {
        let (width, height) = (64u32, 32u32);
        let bgra: Vec<u8> = std::iter::repeat_n([40u8, 80, 120, 255], (width * height) as usize)
            .flatten()
            .collect();
        let blurred = blur(width, height, &bgra).expect("blurred");
        assert_eq!(blurred.dimensions(), (16, 8));
        let pixel = blurred.get_pixel(5, 5).0;
        // A flat colour stays that colour.
        assert!(
            pixel[0].abs_diff(40) <= 1 && pixel[1].abs_diff(80) <= 1 && pixel[2].abs_diff(120) <= 1,
            "{pixel:?}"
        );
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn the_wrong_buffer_size_is_refused() {
        assert!(blur(4, 4, &[0; 10]).is_none());
    }
}
