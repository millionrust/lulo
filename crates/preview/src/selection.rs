//! Tools ▸ Automatic Selection and Edit ▸ Invert Selection (PRV-MENU-013,
//! PRV-27). Before this module, Preview's only selection shape was
//! Tools ▸ Rectangular Selection's single drag rectangle
//! (`view::PreviewView::selection_drag`/`Slot::selection`), which has no
//! way to represent "everything outside this rectangle" or an irregular,
//! flood-filled region. [`Mask`] is a per-pixel boolean selection in the
//! image's own pixel space that both tools (and Invert) produce and
//! share; Crop (PRV-10) and Redact (PRV-MENU-015) consume its bounding
//! box, matching how macOS Preview's own Crop always yields a
//! rectangular result regardless of the selection's shape.

use image::RgbaImage;

/// A per-pixel boolean selection the size of one image, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    width: u32,
    height: u32,
    bits: Vec<bool>,
}

impl Mask {
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bits: vec![false; (width as usize) * (height as usize)],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    fn index(&self, x: u32, y: u32) -> usize {
        y as usize * self.width as usize + x as usize
    }

    pub fn get(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.bits[self.index(x, y)]
    }

    fn set(&mut self, x: u32, y: u32, value: bool) {
        let index = self.index(x, y);
        self.bits[index] = value;
    }

    /// A rectangular mask (Tools ▸ Rectangular Selection's shape, and the
    /// basis Invert Selection negates), in pixel coordinates, clamped to
    /// the image.
    pub fn from_rect(width: u32, height: u32, x: u32, y: u32, w: u32, h: u32) -> Self {
        let mut mask = Self::empty(width, height);
        let x1 = x.saturating_add(w).min(width);
        let y1 = y.saturating_add(h).min(height);
        for py in y.min(height)..y1 {
            for px in x.min(width)..x1 {
                mask.set(px, py, true);
            }
        }
        mask
    }

    /// Edit ▸ Invert Selection (PRV-27): everything that was not selected,
    /// and nothing that was — the mask model this needed, which a single
    /// rectangle alone cannot represent.
    pub fn invert(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            bits: self.bits.iter().map(|bit| !bit).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.bits.iter().any(|bit| *bit)
    }

    pub fn selected_count(&self) -> usize {
        self.bits.iter().filter(|bit| **bit).count()
    }

    /// The smallest rectangle containing every selected pixel, in pixel
    /// coordinates `(x, y, width, height)` — what Crop and Redact act on,
    /// matching macOS Preview's own Crop, which always produces a
    /// rectangular image regardless of the selection's shape.
    pub fn bounding_box(&self) -> Option<(u32, u32, u32, u32)> {
        let mut min_x = u32::MAX;
        let mut min_y = u32::MAX;
        let mut max_x = 0u32;
        let mut max_y = 0u32;
        let mut any = false;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y) {
                    any = true;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        if !any {
            // `min_x`/`min_y` are still `u32::MAX` here; computing the
            // width/height below would underflow, so this must return
            // before that arithmetic runs (`then_some`'s argument is
            // evaluated eagerly, unlike `then`, so it cannot gate this).
            return None;
        }
        Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
    }

    /// A translucent colour overlay (same size as the mask) a caller can
    /// composite over the document to show exactly which pixels are
    /// selected, not just the bounding box — e.g. a flood-filled region's
    /// real silhouette. `rgb` is drawn at `alpha` over selected pixels and
    /// fully transparent elsewhere.
    pub fn overlay(&self, rgb: [u8; 3], alpha: u8) -> RgbaImage {
        let mut overlay = RgbaImage::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let value = if self.get(x, y) {
                    image::Rgba([rgb[0], rgb[1], rgb[2], alpha])
                } else {
                    image::Rgba([0, 0, 0, 0])
                };
                overlay.put_pixel(x, y, value);
            }
        }
        overlay
    }
}

/// Colour distance Automatic Selection treats as "similar enough" to keep
/// flooding, in simple per-channel Manhattan terms over 0..=255 channels
/// (cheap, and more than adequate for flat-colour backgrounds/objects —
/// the common case this tool targets).
fn channel_distance(a: image::Rgba<u8>, b: image::Rgba<u8>) -> u32 {
    a.0.iter()
        .zip(b.0.iter())
        .map(|(x, y)| u32::from(x.abs_diff(*y)))
        .sum()
}

/// Tools ▸ Automatic Selection (PRV-MENU-013): a 4-connected flood fill
/// from `seed`, growing while each new pixel's colour distance from the
/// seed pixel is within `tolerance` (0 = exact match only, 255*4 = select
/// everything). Returns an empty mask for a seed outside the image.
pub fn flood_fill(pixels: &RgbaImage, seed: (u32, u32), tolerance: u32) -> Mask {
    let (width, height) = pixels.dimensions();
    let mut mask = Mask::empty(width, height);
    if seed.0 >= width || seed.1 >= height {
        return mask;
    }
    let seed_color = *pixels.get_pixel(seed.0, seed.1);
    let mut stack = vec![seed];
    mask.set(seed.0, seed.1, true);
    while let Some((x, y)) = stack.pop() {
        let visit = |nx: i64, ny: i64, mask: &mut Mask, stack: &mut Vec<(u32, u32)>| {
            if nx < 0 || ny < 0 || nx >= i64::from(width) || ny >= i64::from(height) {
                return;
            }
            let (nx, ny) = (nx as u32, ny as u32);
            if mask.get(nx, ny) {
                return;
            }
            if channel_distance(*pixels.get_pixel(nx, ny), seed_color) <= tolerance {
                mask.set(nx, ny, true);
                stack.push((nx, ny));
            }
        };
        visit(i64::from(x) - 1, i64::from(y), &mut mask, &mut stack);
        visit(i64::from(x) + 1, i64::from(y), &mut mask, &mut stack);
        visit(i64::from(x), i64::from(y) - 1, &mut mask, &mut stack);
        visit(i64::from(x), i64::from(y) + 1, &mut mask, &mut stack);
    }
    mask
}

/// Colour distance (sum of channel differences, 0..=1020) within which a
/// pixel counts as the same background colour for [`remove_background`].
pub const BACKGROUND_TOLERANCE: u32 = 60;

/// Tools ▸ Remove Background (PRV-MENU-009): makes the image's background
/// transparent. The background colour is the most common colour on the
/// image's border (bucketed to 5 bits a channel, so noise and JPEG
/// artefacts don't split it); every pixel connected to the border through
/// pixels within [`BACKGROUND_TOLERANCE`] of that colour loses its alpha.
/// Interior regions of the same colour that the border can't reach (the
/// hole in a "0") are kept, as a subject's own colours should be. This is
/// colour-keying from the edges, not subject recognition: a photo whose
/// border isn't one flat colour changes little. Returns the pixels
/// unchanged if nothing matched.
pub fn remove_background(pixels: &RgbaImage) -> RgbaImage {
    let (width, height) = pixels.dimensions();
    if width == 0 || height == 0 {
        return pixels.clone();
    }
    let border: Vec<(u32, u32)> = (0..width)
        .flat_map(|x| [(x, 0), (x, height - 1)])
        .chain((0..height).flat_map(|y| [(0, y), (width - 1, y)]))
        .collect();
    let mut buckets: std::collections::HashMap<[u8; 3], (usize, image::Rgba<u8>)> =
        std::collections::HashMap::new();
    for &(x, y) in &border {
        let pixel = *pixels.get_pixel(x, y);
        let key = [pixel.0[0] >> 3, pixel.0[1] >> 3, pixel.0[2] >> 3];
        buckets.entry(key).or_insert((0, pixel)).0 += 1;
    }
    let Some((_, reference)) = buckets
        .values()
        .max_by_key(|(count, pixel)| (*count, pixel.0))
        .copied()
    else {
        return pixels.clone();
    };
    let mut removed = Mask::empty(width, height);
    let mut stack = Vec::new();
    for (x, y) in border {
        if !removed.get(x, y)
            && channel_distance(*pixels.get_pixel(x, y), reference) <= BACKGROUND_TOLERANCE
        {
            removed.set(x, y, true);
            stack.push((x, y));
        }
    }
    while let Some((x, y)) = stack.pop() {
        let neighbours = [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ];
        for (nx, ny) in neighbours {
            if nx >= width || ny >= height || removed.get(nx, ny) {
                continue;
            }
            if channel_distance(*pixels.get_pixel(nx, ny), reference) <= BACKGROUND_TOLERANCE {
                removed.set(nx, ny, true);
                stack.push((nx, ny));
            }
        }
    }
    let mut out = pixels.clone();
    for y in 0..height {
        for x in 0..width {
            if removed.get(x, y) {
                out.get_pixel_mut(x, y).0[3] = 0;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn checker(width: u32, height: u32, split_x: u32) -> RgbaImage {
        let mut pixels = RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let color = if x < split_x {
                    Rgba([255, 0, 0, 255])
                } else {
                    Rgba([0, 0, 255, 255])
                };
                pixels.put_pixel(x, y, color);
            }
        }
        pixels
    }

    #[test]
    fn flood_fill_stops_at_a_colour_boundary_with_zero_tolerance() {
        let pixels = checker(6, 4, 3);
        let mask = flood_fill(&pixels, (0, 0), 0);
        assert_eq!(mask.selected_count(), 3 * 4);
        for y in 0..4 {
            for x in 0..3 {
                assert!(mask.get(x, y), "expected ({x},{y}) selected");
            }
            for x in 3..6 {
                assert!(!mask.get(x, y), "expected ({x},{y}) not selected");
            }
        }
    }

    #[test]
    fn flood_fill_with_full_tolerance_selects_the_whole_image() {
        let pixels = checker(6, 4, 3);
        let mask = flood_fill(&pixels, (0, 0), 255 * 4);
        assert_eq!(mask.selected_count(), 6 * 4);
    }

    #[test]
    fn flood_fill_out_of_bounds_seed_selects_nothing() {
        let pixels = checker(4, 4, 2);
        let mask = flood_fill(&pixels, (10, 10), 0);
        assert!(mask.is_empty());
    }

    #[test]
    fn invert_is_the_exact_complement() {
        let mask = Mask::from_rect(4, 4, 1, 1, 2, 2);
        let inverted = mask.invert();
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(mask.get(x, y), !inverted.get(x, y));
            }
        }
        assert_eq!(
            mask.selected_count() + inverted.selected_count(),
            (4 * 4) as usize
        );
    }

    #[test]
    fn invert_of_invert_is_the_original() {
        let pixels = checker(5, 5, 2);
        let mask = flood_fill(&pixels, (0, 0), 0);
        assert_eq!(mask, mask.invert().invert());
    }

    #[test]
    fn bounding_box_of_an_l_shape_is_not_the_same_as_a_single_rect_assumption() {
        // An L-shape: full first column plus one extra cell on the last
        // row — its bounding box must cover the whole thing, not just
        // the column, proving the box comes from real min/max scanning.
        let mut mask = Mask::empty(5, 5);
        for y in 0..5 {
            mask.set(0, y, true);
        }
        mask.set(4, 4, true);
        let (x, y, w, h) = mask.bounding_box().unwrap();
        assert_eq!((x, y, w, h), (0, 0, 5, 5));
    }

    #[test]
    fn bounding_box_of_empty_mask_is_none() {
        let mask = Mask::empty(3, 3);
        assert!(mask.bounding_box().is_none());
    }

    #[test]
    fn from_rect_clamps_to_image_bounds() {
        let mask = Mask::from_rect(4, 4, 2, 2, 10, 10);
        assert_eq!(mask.bounding_box(), Some((2, 2, 2, 2)));
    }

    #[test]
    fn overlay_is_transparent_outside_the_mask_and_tinted_inside() {
        let mask = Mask::from_rect(3, 3, 1, 1, 1, 1);
        let overlay = mask.overlay([10, 20, 30], 128);
        assert_eq!(*overlay.get_pixel(0, 0), Rgba([0, 0, 0, 0]));
        assert_eq!(*overlay.get_pixel(1, 1), Rgba([10, 20, 30, 128]));
    }

    #[test]
    fn remove_background_clears_the_border_connected_colour_and_keeps_the_subject() {
        // White 7x7 with a red 3x3 subject and a white hole inside it.
        let mut pixels = RgbaImage::from_pixel(7, 7, Rgba([255, 255, 255, 255]));
        for y in 2..5 {
            for x in 2..5 {
                pixels.put_pixel(x, y, Rgba([200, 0, 0, 255]));
            }
        }
        pixels.put_pixel(3, 3, Rgba([255, 255, 255, 255]));
        let out = remove_background(&pixels);
        assert_eq!(out.get_pixel(0, 0).0[3], 0);
        assert_eq!(out.get_pixel(6, 6).0[3], 0);
        assert_eq!(out.get_pixel(2, 2).0, [200, 0, 0, 255]);
        // The enclosed white pixel isn't reachable from the border.
        assert_eq!(out.get_pixel(3, 3).0, [255, 255, 255, 255]);
    }

    #[test]
    fn remove_background_tolerates_slight_noise_in_the_background() {
        let mut pixels = RgbaImage::from_pixel(5, 5, Rgba([250, 250, 250, 255]));
        pixels.put_pixel(0, 2, Rgba([245, 252, 248, 255]));
        pixels.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
        let out = remove_background(&pixels);
        assert_eq!(out.get_pixel(0, 2).0[3], 0);
        assert_eq!(out.get_pixel(2, 2).0[3], 255);
    }

    #[test]
    fn remove_background_leaves_an_empty_image_alone() {
        let pixels = RgbaImage::new(0, 0);
        assert_eq!(remove_background(&pixels).dimensions(), (0, 0));
    }
}
