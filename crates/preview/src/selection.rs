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
        any.then_some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
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
}
