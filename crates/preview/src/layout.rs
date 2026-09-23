//! Page geometry: continuous PDF layout, visibility, the current page, image
//! centring, rotation and the thumbnail sidebar. All values are logical
//! points measured on macOS 26 Preview (see design-lab/preview.html).

use std::ops::Range;

/// Space above the first page and between pages in continuous scroll.
pub const PAGE_MARGIN: f32 = 16.0;

/// Sidebar thumbnails: 120 pt wide, a 6 pt selection pad, a 19 pt label strip
/// and 10 pt between items.
pub const THUMB_WIDTH: f32 = 120.0;
pub const THUMB_PAD: f32 = 6.0;
pub const THUMB_LABEL: f32 = 19.0;
pub const THUMB_GAP: f32 = 10.0;
/// Tallest thumbnail rmac draws (not measured: very tall images shrink to
/// this height instead of making one item fill the sidebar).
pub const THUMB_MAX_HEIGHT: f32 = 240.0;

/// Clockwise quarter turns applied by Rotate Right (⌘R) / Rotate Left (⌘L).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rotation(u8);

impl Rotation {
    pub fn quarter_turns(self) -> u8 {
        self.0
    }

    pub fn from_degrees(degrees: i32) -> Self {
        Self((degrees.rem_euclid(360) / 90) as u8)
    }

    pub fn right(self) -> Self {
        Self((self.0 + 1) % 4)
    }

    pub fn left(self) -> Self {
        Self((self.0 + 3) % 4)
    }

    pub fn plus(self, other: Rotation) -> Self {
        Self((self.0 + other.0) % 4)
    }

    pub fn swaps_axes(self) -> bool {
        self.0 % 2 == 1
    }

    /// Size after rotation.
    pub fn apply(self, size: (f32, f32)) -> (f32, f32) {
        if self.swaps_axes() {
            (size.1, size.0)
        } else {
            size
        }
    }

    /// Rotate a rectangle given in unit coordinates (0‥1, origin top-left)
    /// of the unrotated page into the rotated page's unit coordinates.
    pub fn apply_unit_rect(self, rect: UnitRect) -> UnitRect {
        let UnitRect { x0, y0, x1, y1 } = rect;
        match self.0 {
            1 => UnitRect {
                x0: 1.0 - y1,
                y0: x0,
                x1: 1.0 - y0,
                y1: x1,
            },
            2 => UnitRect {
                x0: 1.0 - x1,
                y0: 1.0 - y1,
                x1: 1.0 - x0,
                y1: 1.0 - y0,
            },
            3 => UnitRect {
                x0: y0,
                y0: 1.0 - x1,
                x1: y1,
                y1: 1.0 - x0,
            },
            _ => rect,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

/// Continuous-scroll layout of every page at `scale` in a column
/// `column_width` wide. Pages are centred; the content grows wider than the
/// column when zoomed pages (plus the fit margin) do not fit.
#[derive(Clone, Debug, PartialEq)]
pub struct PageLayout {
    pub pages: Vec<Rect>,
    pub width: f32,
    pub height: f32,
}

pub fn continuous(page_sizes: &[(f32, f32)], scale: f32, column_width: f32) -> PageLayout {
    let widest = page_sizes
        .iter()
        .map(|(width, _)| width * scale)
        .fold(0.0_f32, f32::max);
    let width = column_width.max(widest + crate::zoom::PDF_FIT_MARGIN);
    let mut y = PAGE_MARGIN;
    let mut pages = Vec::with_capacity(page_sizes.len());
    for (page_width, page_height) in page_sizes {
        let (w, h) = (page_width * scale, page_height * scale);
        pages.push(Rect {
            x: ((width - w) / 2.0).max(0.0),
            y,
            width: w,
            height: h,
        });
        y += h + PAGE_MARGIN;
    }
    PageLayout {
        pages,
        width,
        height: y,
    }
}

/// Indices of pages intersecting the viewport, widened by `overscan` pages
/// on each side so neighbours render before they scroll in.
pub fn visible_pages(
    pages: &[Rect],
    scroll_top: f32,
    viewport_height: f32,
    overscan: usize,
) -> Range<usize> {
    let bottom = scroll_top + viewport_height;
    let first = pages.partition_point(|page| page.bottom() < scroll_top);
    let end = pages.partition_point(|page| page.y <= bottom);
    let end = end.max(first);
    first.saturating_sub(overscan)..(end + overscan).min(pages.len())
}

/// The page shown as "Page N of M": the one under the viewport's vertical
/// midpoint, or the nearest page above it when the midpoint is in a gap.
pub fn current_page(pages: &[Rect], scroll_top: f32, viewport_height: f32) -> usize {
    if pages.is_empty() {
        return 0;
    }
    let middle = scroll_top + viewport_height / 2.0;
    let after = pages.partition_point(|page| page.y <= middle);
    after.saturating_sub(1).min(pages.len() - 1)
}

/// Scroll offset that brings `index` to the top with the standard margin.
pub fn scroll_to_page(pages: &[Rect], index: usize) -> f32 {
    pages
        .get(index)
        .map(|page| (page.y - PAGE_MARGIN).max(0.0))
        .unwrap_or(0.0)
}

/// Origin that centres content in the viewport on each axis where it is
/// smaller, and pins it to the start where it overflows (and scrolls).
pub fn centred_origin(content: (f32, f32), viewport: (f32, f32)) -> (f32, f32) {
    (
        ((viewport.0 - content.0) / 2.0).max(0.0),
        ((viewport.1 - content.1) / 2.0).max(0.0),
    )
}

/// Largest scroll offset for content in a viewport.
pub fn max_scroll(content: f32, viewport: f32) -> f32 {
    (content - viewport).max(0.0)
}

/// Thumbnail size for a page or image of `size` points.
pub fn thumbnail_size(size: (f32, f32)) -> (f32, f32) {
    let (width, height) = size;
    if width <= 0.0 || height <= 0.0 {
        return (THUMB_WIDTH, THUMB_WIDTH);
    }
    let scale = (THUMB_WIDTH / width).min(THUMB_MAX_HEIGHT / height);
    (width * scale, height * scale)
}

/// One sidebar item: the selection highlight spans `top‥top+height`; the
/// thumbnail sits `THUMB_PAD` inside it and the label strip below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThumbItem {
    pub top: f32,
    pub thumb: (f32, f32),
    pub height: f32,
}

pub fn thumbnail_items(sizes: &[(f32, f32)]) -> (Vec<ThumbItem>, f32) {
    let mut top = 0.0;
    let mut items = Vec::with_capacity(sizes.len());
    for size in sizes {
        let thumb = thumbnail_size(*size);
        let height = THUMB_PAD + thumb.1 + THUMB_LABEL;
        items.push(ThumbItem { top, thumb, height });
        top += height + THUMB_GAP;
    }
    (items, top)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: (f32, f32) = (612.0, 792.0);

    #[test]
    fn continuous_layout_matches_the_measured_pdf() {
        let scale = crate::zoom::fit_width(612.0, 953.0);
        let layout = continuous(&[LETTER; 3], scale, 953.0);
        let first = layout.pages[0];
        assert!((first.width - 928.0).abs() < 0.01);
        assert!((first.height - 1200.9).abs() < 0.1);
        assert!((first.x - 12.5).abs() < 0.01);
        assert_eq!(first.y, 16.0);
        // 105 → 1322: the second page starts one page + 16 pt lower.
        assert!((layout.pages[1].y - (16.0 + first.height + 16.0)).abs() < 0.01);
        assert_eq!(layout.width, 953.0);
        assert!((layout.height - (16.0 + 3.0 * (first.height + 16.0))).abs() < 0.01);
    }

    #[test]
    fn zoomed_pages_widen_the_content_and_stay_centred() {
        let layout = continuous(&[LETTER, (400.0, 400.0)], 2.0, 953.0);
        assert_eq!(layout.width, 1224.0 + 25.0);
        assert_eq!(layout.pages[0].x, 12.5);
        assert_eq!(layout.pages[1].x, (1249.0 - 800.0) / 2.0);
    }

    #[test]
    fn visible_pages_cover_the_viewport_with_overscan() {
        let layout = continuous(&[(100.0, 100.0); 10], 1.0, 200.0);
        // Pages at 16, 132, 248, 364 … (116 pitch).
        assert_eq!(visible_pages(&layout.pages, 0.0, 200.0, 0), 0..2);
        assert_eq!(visible_pages(&layout.pages, 250.0, 100.0, 0), 2..3);
        assert_eq!(visible_pages(&layout.pages, 250.0, 100.0, 1), 1..4);
        assert_eq!(visible_pages(&layout.pages, 5000.0, 100.0, 0), 10..10);
        assert_eq!(visible_pages(&[], 0.0, 100.0, 1), 0..0);
    }

    #[test]
    fn current_page_follows_the_viewport_middle() {
        let layout = continuous(&[(100.0, 100.0); 3], 1.0, 200.0);
        assert_eq!(current_page(&layout.pages, 0.0, 100.0), 0);
        assert_eq!(current_page(&layout.pages, 100.0, 100.0), 1);
        // Middle in the gap after page 2 still reports page 2.
        assert_eq!(current_page(&layout.pages, 5000.0, 100.0), 2);
        assert_eq!(current_page(&[], 0.0, 100.0), 0);
    }

    #[test]
    fn scroll_to_page_leaves_the_top_margin() {
        let layout = continuous(&[(100.0, 100.0); 3], 1.0, 200.0);
        assert_eq!(scroll_to_page(&layout.pages, 0), 0.0);
        assert_eq!(scroll_to_page(&layout.pages, 1), 116.0);
        assert_eq!(scroll_to_page(&layout.pages, 9), 0.0);
    }

    #[test]
    fn rotation_cycles_and_swaps_axes() {
        let rotation = Rotation::default();
        assert_eq!(rotation.right().quarter_turns(), 1);
        assert_eq!(rotation.left().quarter_turns(), 3);
        assert_eq!(rotation.left().right(), rotation);
        assert_eq!(rotation.right().apply((800.0, 600.0)), (600.0, 800.0));
        assert_eq!(Rotation::from_degrees(-90), Rotation::from_degrees(270));
        assert_eq!(Rotation::from_degrees(450).quarter_turns(), 1);
        assert_eq!(
            Rotation::from_degrees(90).plus(Rotation::from_degrees(270)),
            Rotation::default()
        );
    }

    #[test]
    fn unit_rects_rotate_clockwise() {
        let top_left = UnitRect {
            x0: 0.0,
            y0: 0.0,
            x1: 0.25,
            y1: 0.5,
        };
        // A quarter turn clockwise moves the top-left corner to the top-right.
        assert_eq!(
            Rotation::from_degrees(90).apply_unit_rect(top_left),
            UnitRect {
                x0: 0.5,
                y0: 0.0,
                x1: 1.0,
                y1: 0.25
            }
        );
        assert_eq!(
            Rotation::from_degrees(180).apply_unit_rect(top_left),
            UnitRect {
                x0: 0.75,
                y0: 0.5,
                x1: 1.0,
                y1: 1.0
            }
        );
        assert_eq!(
            Rotation::from_degrees(270).apply_unit_rect(top_left),
            UnitRect {
                x0: 0.0,
                y0: 0.75,
                x1: 0.5,
                y1: 1.0
            }
        );
    }

    #[test]
    fn images_centre_until_they_overflow() {
        assert_eq!(
            centred_origin((600.0, 400.0), (800.0, 600.0)),
            (100.0, 100.0)
        );
        assert_eq!(
            centred_origin((1000.0, 400.0), (800.0, 600.0)),
            (0.0, 100.0)
        );
        assert_eq!(max_scroll(1000.0, 800.0), 200.0);
        assert_eq!(max_scroll(100.0, 800.0), 0.0);
    }

    #[test]
    fn thumbnails_match_the_measured_sidebar() {
        let (items, total) = thumbnail_items(&[LETTER, LETTER, (800.0, 600.0)]);
        // Letter: 120 × 155.3; selection 52 → 233 and the next page 191 lower.
        assert!((items[0].thumb.1 - 155.29).abs() < 0.01);
        assert!((items[0].height - 180.29).abs() < 0.01);
        assert!((items[1].top - 190.29).abs() < 0.01);
        assert!((items[2].thumb.0 - 120.0).abs() < 1e-3 && (items[2].thumb.1 - 90.0).abs() < 1e-3);
        assert!((total - (items[2].top + items[2].height + THUMB_GAP)).abs() < 0.01);
        assert_eq!(thumbnail_size((100.0, 1000.0)), (24.0, 240.0));
        assert_eq!(thumbnail_size((0.0, 10.0)), (120.0, 120.0));
    }
}
