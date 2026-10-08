//! Lulo's wallpaper for Lulo mode's desktop (ADR 0023 "Lulo mode").
//!
//! The picture is the one chosen in Lulo's System Settings ▸ Wallpaper,
//! from the same store and renderer Lulo OS uses (`rmac-shell-settings`,
//! `rmac-wallpaper`, `rmac-wallpaper-image`): the Lulo artwork, a gradient
//! or the user's own photo, in its light or dark form. It is decoded once
//! at the screen's size on a background thread and read again only when
//! the settings file changes (one parked thread on a change notification)
//! or the appearance or the display changes. Windows' own desktop picture
//! is never read or changed.

use std::path::PathBuf;

use rmac_shell_settings::WallpaperFit;
use windows::Win32::Storage::FileSystem::{
    FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE,
};
use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};

use super::trace;

/// A decoded wallpaper, as GPUI draws images: BGRA, straight alpha.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub fit: WallpaperFit,
    /// Mean relative luminance (0–1) of the strip the menu bar covers, so
    /// the bar's text can be light on a dark picture and dark on a light one.
    pub bar_luminance: f32,
    /// The wallpaper under the menu bar and under the Dock's strip, each
    /// the full width of the screen, shrunk so that drawn at full size it
    /// is the blurred picture the Mac's materials show (WIN-OS-47).
    pub bar_strip: Strip,
    pub dock_strip: Strip,
}

/// A small BGRA image of one band of the screen's wallpaper.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// How a picture covers the screen: screen pixel `x` shows picture pixel
/// `(x + offset_x) / scale_x` (and so for `y`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mapping {
    scale_x: f32,
    scale_y: f32,
    offset_x: f32,
    offset_y: f32,
}

fn mapping(fit: WallpaperFit, picture: (u32, u32), screen: (u32, u32)) -> Mapping {
    let (pw, ph) = (picture.0.max(1) as f32, picture.1.max(1) as f32);
    let (sw, sh) = (screen.0 as f32, screen.1 as f32);
    let centred = |scale: f32| Mapping {
        scale_x: scale,
        scale_y: scale,
        offset_x: (pw * scale - sw) / 2.0,
        offset_y: (ph * scale - sh) / 2.0,
    };
    match fit {
        WallpaperFit::Fill | WallpaperFit::Tile => centred((sw / pw).max(sh / ph)),
        WallpaperFit::Fit => centred((sw / pw).min(sh / ph)),
        WallpaperFit::Center => centred(1.0),
        WallpaperFit::Stretch => Mapping {
            scale_x: sw / pw,
            scale_y: sh / ph,
            offset_x: 0.0,
            offset_y: 0.0,
        },
    }
}

/// The band of screen rows `top..bottom` as an `out_width` × `out_height`
/// BGRA strip: each pixel the average of a small grid of picture samples
/// (black where the picture does not reach), so the strip is the band
/// blurred by about its cell size when it is drawn at full size.
#[allow(clippy::too_many_arguments)]
fn band(
    rgba: &[u8],
    picture: (u32, u32),
    fit: WallpaperFit,
    screen: (u32, u32),
    top: u32,
    bottom: u32,
    out_width: u32,
    out_height: u32,
) -> Strip {
    const SAMPLES: u32 = 4;
    let map = mapping(fit, picture, screen);
    let (pw, ph) = (picture.0 as i64, picture.1 as i64);
    let mut bgra = Vec::with_capacity((out_width * out_height * 4) as usize);
    let cell_width = screen.0 as f32 / out_width.max(1) as f32;
    let cell_height = bottom.saturating_sub(top) as f32 / out_height.max(1) as f32;
    for row in 0..out_height {
        for column in 0..out_width {
            let mut sum = [0u32; 3];
            for sample_y in 0..SAMPLES {
                for sample_x in 0..SAMPLES {
                    let x = (column as f32 + (sample_x as f32 + 0.5) / SAMPLES as f32) * cell_width;
                    let y = top as f32
                        + (row as f32 + (sample_y as f32 + 0.5) / SAMPLES as f32) * cell_height;
                    let source_x = ((x + map.offset_x) / map.scale_x).floor() as i64;
                    let source_y = ((y + map.offset_y) / map.scale_y).floor() as i64;
                    if (0..pw).contains(&source_x) && (0..ph).contains(&source_y) {
                        let at = ((source_y * pw + source_x) * 4) as usize;
                        if let Some(pixel) = rgba.get(at..at + 3) {
                            sum[0] += u32::from(pixel[0]);
                            sum[1] += u32::from(pixel[1]);
                            sum[2] += u32::from(pixel[2]);
                        }
                    }
                }
            }
            let count = SAMPLES * SAMPLES;
            bgra.extend_from_slice(&[
                (sum[2] / count) as u8,
                (sum[1] / count) as u8,
                (sum[0] / count) as u8,
                255,
            ]);
        }
    }
    Strip {
        width: out_width,
        height: out_height,
        bgra,
    }
}

/// Mean relative luminance of a BGRA strip.
fn bgra_luminance(strip: &Strip) -> f32 {
    let linear = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let pixels = strip.bgra.chunks_exact(4);
    let count = pixels.len().max(1) as f32;
    pixels
        .map(|pixel| {
            0.2126 * linear(pixel[2]) + 0.7152 * linear(pixel[1]) + 0.0722 * linear(pixel[0])
        })
        .sum::<f32>()
        / count
}

/// The selection saved in Lulo's shell settings, or Lulo's default.
fn selection() -> rmac_shell_settings::WallpaperSelection {
    rmac_shell_settings::ShellSettingsStore::from_environment()
        .and_then(|store| store.load())
        .map(|snapshot| snapshot.settings.wallpaper.default)
        .unwrap_or_default()
}

/// The settings file, for the change watch.
pub fn settings_path() -> Option<PathBuf> {
    rmac_shell_settings::ShellSettingsStore::from_environment()
        .ok()
        .map(|store| store.path().to_path_buf())
}

/// How much smaller than the screen the bar's and the Dock's strips are:
/// drawn at full size they are the band blurred over this many pixels.
const STRIP_SHRINK: u32 = 12;

/// Decode the wallpaper for a `width` × `height` physical-pixel screen
/// whose menu bar is `bar` pixels deep and whose Dock strip is `dock`.
pub fn load(width: u32, height: u32, bar: u32, dock: u32, dark: bool) -> Option<Picture> {
    let selection = selection();
    let target = rmac_compositor::PhysicalSize { width, height };
    let cache = rmac_wallpaper_image::Cache::new(0);
    let decode = |source: &rmac_wallpaper::Source| {
        let resolved = rmac_wallpaper_system::resolve(source).ok()?;
        cache.get_or_decode_for(resolved, target, dark).ok()
    };
    let source = rmac_wallpaper::parse_source(selection.source.as_deref()).unwrap_or(
        rmac_wallpaper::Source::BuiltIn(rmac_wallpaper::DEFAULT_BUILT_IN),
    );
    let decoded = decode(&source).or_else(|| {
        // The packaged artwork may be missing (a development build) or the
        // user's photo gone: the file-free fallback always draws.
        trace(|| "wallpaper: the chosen picture could not be read; using the fallback".into());
        decode(&rmac_wallpaper::Source::BuiltIn(
            rmac_wallpaper::FALLBACK_BUILT_IN,
        ))
    })?;
    let (picture_width, picture_height) = (decoded.width, decoded.height);
    let rgba = decoded.rgba.clone();
    drop(decoded);
    let picture = (picture_width, picture_height);
    let screen = (width, height);
    let strip_width = (width / STRIP_SHRINK).max(1);
    let bar_strip = band(
        &rgba,
        picture,
        selection.fit,
        screen,
        0,
        bar,
        strip_width,
        (bar / STRIP_SHRINK).max(2),
    );
    let dock_strip = band(
        &rgba,
        picture,
        selection.fit,
        screen,
        height.saturating_sub(dock),
        height,
        strip_width,
        (dock / STRIP_SHRINK).max(2),
    );
    let bar_luminance = bgra_luminance(&bar_strip);
    let mut bgra = rgba.to_vec();
    drop(rgba);
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    trace(|| {
        format!(
            "wallpaper {}x{} for {width}x{height}, bar luminance {bar_luminance:.2}",
            picture_width, picture_height
        )
    });
    Some(Picture {
        width: picture_width,
        height: picture_height,
        bgra,
        fit: selection.fit,
        bar_luminance,
        bar_strip,
        dock_strip,
    })
}

/// Mean relative luminance of the top `rows` rows of an RGBA picture,
/// sampled every few pixels (a bounded read whatever the picture's size).
pub fn strip_luminance(rgba: &[u8], width: u32, height: u32, rows: u32) -> f32 {
    let (width, height) = (width as usize, height as usize);
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return 0.0;
    }
    let rows = (rows as usize).clamp(1, height);
    let step = (width / 256).max(1);
    let row_step = (rows / 8).max(1);
    let linear = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let (mut total, mut count) = (0.0f32, 0u32);
    for row in (0..rows).step_by(row_step) {
        for column in (0..width).step_by(step) {
            let at = (row * width + column) * 4;
            total += 0.2126 * linear(rgba[at])
                + 0.7152 * linear(rgba[at + 1])
                + 0.0722 * linear(rgba[at + 2]);
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

/// Whether text over a backdrop of `luminance` should be dark, as the Mac's
/// menu bar picks: light text unless the wallpaper behind it is light.
pub fn dark_text_on(luminance: f32) -> bool {
    luminance > 0.45
}

/// Call `on_change` (from a parked thread) whenever Lulo's shell settings
/// file is written, so the desktop shows a newly chosen wallpaper.
pub fn watch(on_change: impl Fn() + Send + 'static) {
    let Some(folder) = settings_path().and_then(|path| path.parent().map(PathBuf::from)) else {
        return;
    };
    let _ = std::fs::create_dir_all(&folder);
    let spawned = std::thread::Builder::new()
        .name("lulo-wallpaper-watch".into())
        .spawn(move || {
            // SAFETY: watches one folder for writes and renames in it.
            let Ok(handle) = (unsafe {
                FindFirstChangeNotificationW(
                    &windows::core::HSTRING::from(folder.as_os_str()),
                    false,
                    FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
                )
            }) else {
                return;
            };
            loop {
                // SAFETY: waits on the handle opened above.
                unsafe { WaitForSingleObject(handle, INFINITE) };
                on_change();
                // SAFETY: re-arms the handle.
                if unsafe { FindNextChangeNotification(handle) }.is_err() {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("lulo-shell: the desktop will not follow wallpaper changes: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_strip_is_measured_from_the_top_rows() {
        // A picture dark on top and white below.
        let (width, height) = (16u32, 10u32);
        let mut rgba = vec![255u8; (width * height * 4) as usize];
        for pixel in rgba[..(width * 2 * 4) as usize].chunks_exact_mut(4) {
            pixel[..3].copy_from_slice(&[10, 10, 40]);
        }
        let top = strip_luminance(&rgba, width, height, 2);
        assert!(top < 0.05, "{top}");
        assert!(!dark_text_on(top));
        let all = strip_luminance(&rgba, width, height, height);
        assert!(all > 0.5);
        assert!(dark_text_on(all));
        assert_eq!(strip_luminance(&[], 0, 0, 1), 0.0);
    }

    #[test]
    fn a_covering_picture_maps_its_centre_to_the_screens() {
        // A 2:1 picture on a 4:3 screen is cropped at its sides.
        let map = mapping(WallpaperFit::Fill, (2000, 1000), (1024, 768));
        let centre_x = (512.0 + map.offset_x) / map.scale_x;
        let centre_y = (384.0 + map.offset_y) / map.scale_y;
        assert!((centre_x - 1000.0).abs() < 1.0 && (centre_y - 500.0).abs() < 1.0);
        let stretch = mapping(WallpaperFit::Stretch, (100, 50), (200, 200));
        assert_eq!((stretch.scale_x, stretch.scale_y), (2.0, 4.0));
    }

    #[test]
    fn strips_average_their_band_of_the_picture() {
        // Red on top, blue below, on a screen the picture's own size.
        let (width, height) = (64u32, 32u32);
        let mut rgba = Vec::new();
        for y in 0..height {
            for _ in 0..width {
                rgba.extend_from_slice(if y < 8 {
                    &[200, 0, 0, 255]
                } else {
                    &[0, 0, 200, 255]
                });
            }
        }
        let top = band(
            &rgba,
            (width, height),
            WallpaperFit::Fill,
            (width, height),
            0,
            8,
            4,
            2,
        );
        assert_eq!((top.width, top.height, top.bgra.len()), (4, 2, 32));
        // BGRA: red lands in the third byte.
        assert_eq!(&top.bgra[..4], &[0, 0, 200, 255]);
        let bottom = band(
            &rgba,
            (width, height),
            WallpaperFit::Fill,
            (width, height),
            24,
            32,
            4,
            2,
        );
        assert_eq!(&bottom.bgra[..4], &[200, 0, 0, 255]);
        assert!(bgra_luminance(&top) < 0.2);
        // Beyond a fitted picture's edge the strip is black.
        let letterbox = band(
            &rgba,
            (width, height),
            WallpaperFit::Fit,
            (width, height * 4),
            0,
            8,
            2,
            1,
        );
        assert_eq!(&letterbox.bgra[..4], &[0, 0, 0, 255]);
    }
}
