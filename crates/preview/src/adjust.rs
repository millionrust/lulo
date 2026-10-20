//! Tools ▸ Adjust Color… (PREV-10): exposure, contrast, highlights, shadows,
//! saturation, temperature, tint, sepia and sharpness on a picture's pixels.
//! Pixels are Preview's own BGRA order (see `render`), so channel 0 is blue.

use image::RgbaImage;

/// Every slider of the Adjust Color sheet. Zero (sepia and sharpness: zero)
/// leaves the picture as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColourAdjust {
    /// Stops of exposure, -2.0‥2.0.
    pub exposure: f32,
    /// -1.0 (flat) ‥ 1.0 (punchy).
    pub contrast: f32,
    /// Positive values pull bright areas down.
    pub highlights: f32,
    /// Positive values lift dark areas.
    pub shadows: f32,
    /// -1.0 (grey) ‥ 1.0 (vivid).
    pub saturation: f32,
    /// -1.0 (cool) ‥ 1.0 (warm).
    pub temperature: f32,
    /// -1.0 (green) ‥ 1.0 (magenta).
    pub tint: f32,
    /// 0.0‥1.0.
    pub sepia: f32,
    /// 0.0‥1.0.
    pub sharpness: f32,
}

impl ColourAdjust {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    fn colour_is_identity(&self) -> bool {
        Self {
            sharpness: 0.0,
            ..*self
        } == Self::default()
    }
}

fn luma(red: f32, green: f32, blue: f32) -> f32 {
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

fn adjust_pixel(bgr: [u8; 3], p: &ColourAdjust, gain: f32) -> [u8; 3] {
    let mut blue = f32::from(bgr[0]) / 255.0 * gain;
    let mut green = f32::from(bgr[1]) / 255.0 * gain;
    let mut red = f32::from(bgr[2]) / 255.0 * gain;
    // White balance: warm raises red and lowers blue; tint moves green
    // against magenta.
    red *= 1.0 + 0.3 * p.temperature;
    blue *= 1.0 - 0.3 * p.temperature;
    green *= 1.0 - 0.3 * p.tint;
    red *= 1.0 + 0.15 * p.tint;
    blue *= 1.0 + 0.15 * p.tint;
    let l = luma(red, green, blue).clamp(0.0, 1.0);
    let lift = p.shadows * 0.5 * (1.0 - l).powi(2) - p.highlights * 0.5 * l.powi(2);
    red += lift;
    green += lift;
    blue += lift;
    let factor = 1.0 + p.contrast;
    red = (red - 0.5) * factor + 0.5;
    green = (green - 0.5) * factor + 0.5;
    blue = (blue - 0.5) * factor + 0.5;
    let l = luma(red, green, blue);
    let vivid = 1.0 + p.saturation;
    red = l + (red - l) * vivid;
    green = l + (green - l) * vivid;
    blue = l + (blue - l) * vivid;
    if p.sepia > 0.0 {
        let (sr, sg, sb) = (
            0.393 * red + 0.769 * green + 0.189 * blue,
            0.349 * red + 0.686 * green + 0.168 * blue,
            0.272 * red + 0.534 * green + 0.131 * blue,
        );
        red += (sr - red) * p.sepia;
        green += (sg - green) * p.sepia;
        blue += (sb - blue) * p.sepia;
    }
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [byte(blue), byte(green), byte(red)]
}

/// An unsharp mask: each colour channel plus `amount` times its difference
/// from a 3x3 average.
fn sharpen(source: &RgbaImage, amount: f32) -> RgbaImage {
    let (width, height) = source.dimensions();
    if width < 3 || height < 3 {
        return source.clone();
    }
    let mut out = source.clone();
    let raw = source.as_raw();
    let at =
        |x: u32, y: u32, channel: usize| f32::from(raw[((y * width + x) * 4) as usize + channel]);
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let target = out.get_pixel_mut(x, y);
            for channel in 0..3 {
                let mut sum = 0.0;
                for dy in 0..3 {
                    for dx in 0..3 {
                        sum += at(x + dx - 1, y + dy - 1, channel);
                    }
                }
                let centre = at(x, y, channel);
                let value = centre + amount * 2.0 * (centre - sum / 9.0);
                target[channel] = value.clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// `pixels` with `p` applied; alpha is never touched.
pub fn adjust_colour(pixels: &RgbaImage, p: &ColourAdjust) -> RgbaImage {
    if p.is_identity() {
        return pixels.clone();
    }
    let mut out = pixels.clone();
    if !p.colour_is_identity() {
        let gain = 2.0_f32.powf(p.exposure.clamp(-4.0, 4.0));
        for pixel in out.pixels_mut() {
            let [blue, green, red] = adjust_pixel([pixel[0], pixel[1], pixel[2]], p, gain);
            pixel[0] = blue;
            pixel[1] = green;
            pixel[2] = red;
        }
    }
    if p.sharpness > 0.0 {
        out = sharpen(&out, p.sharpness.clamp(0.0, 1.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn flat(bgr: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(4, 4, Rgba([bgr[0], bgr[1], bgr[2], 200]))
    }

    #[test]
    fn untouched_sliders_leave_every_pixel_alone() {
        let source = flat([10, 120, 240]);
        assert_eq!(adjust_colour(&source, &ColourAdjust::default()), source);
    }

    #[test]
    fn exposure_doubles_a_dark_tone_per_stop_and_keeps_alpha() {
        let adjusted = adjust_colour(
            &flat([20, 20, 20]),
            &ColourAdjust {
                exposure: 1.0,
                ..Default::default()
            },
        );
        assert_eq!(adjusted.get_pixel(0, 0).0, [40, 40, 40, 200]);
    }

    #[test]
    fn removing_all_saturation_makes_grey_and_adding_sepia_warms_it() {
        let grey = adjust_colour(
            &flat([0, 0, 255]),
            &ColourAdjust {
                saturation: -1.0,
                ..Default::default()
            },
        );
        let pixel = grey.get_pixel(0, 0).0;
        assert!(pixel[0] == pixel[1] && pixel[1] == pixel[2], "{pixel:?}");
        let warm = adjust_colour(
            &flat([128, 128, 128]),
            &ColourAdjust {
                sepia: 1.0,
                ..Default::default()
            },
        );
        let pixel = warm.get_pixel(0, 0).0;
        // BGRA: red (index 2) above blue (index 0).
        assert!(pixel[2] > pixel[0], "{pixel:?}");
    }

    #[test]
    fn warm_temperature_raises_red_over_blue() {
        let warm = adjust_colour(
            &flat([128, 128, 128]),
            &ColourAdjust {
                temperature: 1.0,
                ..Default::default()
            },
        );
        let pixel = warm.get_pixel(0, 0).0;
        assert!(pixel[2] > 128 && pixel[0] < 128);
    }

    #[test]
    fn contrast_pushes_tones_away_from_the_middle() {
        let punchy = ColourAdjust {
            contrast: 0.5,
            ..Default::default()
        };
        assert!(adjust_colour(&flat([200, 200, 200]), &punchy).get_pixel(0, 0)[0] > 200);
        assert!(adjust_colour(&flat([60, 60, 60]), &punchy).get_pixel(0, 0)[0] < 60);
    }

    #[test]
    fn shadows_lift_darks_more_than_brights_and_sharpness_sharpens_an_edge() {
        let lifted = ColourAdjust {
            shadows: 1.0,
            ..Default::default()
        };
        let dark = adjust_colour(&flat([20, 20, 20]), &lifted).get_pixel(0, 0)[0];
        let bright = adjust_colour(&flat([230, 230, 230]), &lifted).get_pixel(0, 0)[0];
        assert!(dark > 20 + 20 && bright < 230 + 10, "{dark} {bright}");
        let mut edge = RgbaImage::from_pixel(5, 3, Rgba([50, 50, 50, 255]));
        for y in 0..3 {
            for x in 3..5 {
                edge.put_pixel(x, y, Rgba([200, 200, 200, 255]));
            }
        }
        let sharp = adjust_colour(
            &edge,
            &ColourAdjust {
                sharpness: 1.0,
                ..Default::default()
            },
        );
        // The dark side of the edge gets darker, the bright side brighter.
        assert!(sharp.get_pixel(2, 1)[0] < 50);
        assert!(sharp.get_pixel(3, 1)[0] > 200);
    }
}
