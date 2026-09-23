//! Quick Look geometry measured on the owner's Mac (macOS 26.2, dark,
//! 2026-09-23); see design-lab/quick-look.html for the captures. Values
//! marked S could not be measured and are stated as such there.

/// Title bar height; the content frame starts here.
pub const TITLE_BAR: f32 = 38.0;
/// Content inset from the left, right and bottom window edges.
pub const INSET: f32 = 5.0;
pub const WINDOW_RADIUS: f32 = 20.0;
pub const CONTENT_RADIUS: f32 = 15.0;

/// Close and full-screen buttons (14 pt glyphs).
pub const CLOSE_X: f32 = 11.0;
pub const FULL_SCREEN_X: f32 = 31.0;
pub const CIRCLE_Y: f32 = 12.0;
pub const CIRCLE_SIZE: f32 = 14.0;

/// Previous/next capsule shown for a multi-item selection.
pub const NAV_X: f32 = 51.0;
pub const NAV_Y: f32 = 6.0;
pub const NAV_WIDTH: f32 = 61.0;
pub const NAV_HEIGHT: f32 = 26.0;
pub const NAV_DIVIDER_X: f32 = 30.0;

pub const TITLE_X: f32 = 51.0;
pub const TITLE_X_AFTER_NAV: f32 = 118.0;
pub const TITLE_X_INDEX_SHEET: f32 = 31.0;
pub const TITLE_Y: f32 = 9.0;
pub const TITLE_HEIGHT: f32 = 18.0;
pub const TITLE_SIZE: f32 = 13.0;

/// Title-bar buttons (index sheet) and the "Open with …" capsule.
pub const BUTTON_WIDTH: f32 = 33.0;
pub const BUTTON_HEIGHT: f32 = 19.0;
pub const BUTTON_Y: f32 = 9.0;
pub const BUTTON_GAP: f32 = 6.0;
pub const RIGHT_MARGIN: f32 = 7.0;
pub const OPEN_PADDING: f32 = 12.0;
pub const UNCOMPRESS_WIDTH: f32 = 103.0;
pub const UNCOMPRESS_HEIGHT: f32 = 26.0;
pub const UNCOMPRESS_Y: f32 = 6.0;

/// Folders, archives and other files without a preview.
pub const SUMMARY_WINDOW: (f32, f32) = (592.0, 328.0);
pub const SUMMARY_ICON_X: f32 = 20.0;
pub const SUMMARY_ICON_Y: f32 = 48.0;
pub const SUMMARY_ICON_SIZE: f32 = 256.0;
pub const SUMMARY_TEXT_X: f32 = 298.0;
pub const SUMMARY_TEXT_WIDTH: f32 = 274.0;
pub const SUMMARY_NAME_Y: f32 = 128.0;
pub const SUMMARY_NAME_SIZE: f32 = 20.0;
pub const SUMMARY_LINE_2_Y: f32 = 163.0;
pub const SUMMARY_LINE_3_Y: f32 = 185.0;

/// Text previews: monospaced 11 on a 13 pt line (S: size from the pitch).
pub const TEXT_SIZE: f32 = 11.0;
pub const TEXT_LINE: f32 = 13.0;
pub const TEXT_PADDING_X: f32 = 8.0;
pub const TEXT_PADDING_TOP: f32 = 3.0;
/// A text file opened on its own (S: the Mac kept the previous panel size
/// when stepping onto one, 721 × 541).
pub const TEXT_CONTENT: (f32, f32) = (721.0, 541.0);

/// Longest content side as a share of the screen's visible height: 721 of
/// 923 on the 1470 × 956 Mac (S: the Mac's exact rule is private).
pub const MAX_CONTENT_FRACTION: f32 = 721.0 / 923.0;
/// Smallest content frame, so the title bar always fits (S).
pub const MIN_CONTENT: (f32, f32) = (320.0, 200.0);

/// Index sheet grid.
pub const INDEX_CELL: f32 = 215.0;
pub const INDEX_MARGIN: f32 = 37.0;
pub const INDEX_TOP: f32 = 46.0;
pub const INDEX_ROW_GAP: f32 = 60.0;
pub const INDEX_SELECTION: f32 = 4.0;

/// Pages of a PDF rendered for Quick Look (S: bounded for low-end PCs).
pub const MAX_PDF_PAGES: usize = 30;
pub const PDF_PAGE_GAP: f32 = 8.0;

/// The largest content frame on a screen whose visible area is `visible`
/// (width, height): 721 tall on the Mac's 923-tall visible area. The same
/// share is applied to the width (S).
pub fn content_limits(visible: (f32, f32)) -> (f32, f32) {
    (
        // The small bias keeps 923 × (721 ÷ 923) from flooring to 720.
        (visible.0 * MAX_CONTENT_FRACTION + 0.01)
            .floor()
            .max(MIN_CONTENT.0),
        (visible.1 * MAX_CONTENT_FRACTION + 0.01)
            .floor()
            .max(MIN_CONTENT.1),
    )
}

/// Natural size shown 1:1 when it fits `limits`, otherwise scaled down to
/// fit; never smaller than [`MIN_CONTENT`]. While stepping through a
/// selection the Mac keeps the first item's box, so later items fit a square
/// of that item's longest side.
pub fn fit(natural: (f32, f32), limits: (f32, f32)) -> (f32, f32) {
    let (width, height) = (natural.0.max(1.0), natural.1.max(1.0));
    let scale = (limits.0 / width).min(limits.1 / height).min(1.0);
    (
        (width * scale).round().max(MIN_CONTENT.0),
        (height * scale).round().max(MIN_CONTENT.1),
    )
}

/// Window size around a content frame.
pub fn window_for_content(content: (f32, f32)) -> (f32, f32) {
    (content.0 + 2.0 * INSET, content.1 + TITLE_BAR + INSET)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Index sheet cells for items whose height ÷ width are `aspects`, laid out
/// in a content frame `width` wide. Returns the cells and the grid height.
pub fn index_layout(aspects: &[f32], width: f32) -> (Vec<Cell>, f32) {
    let pitch = INDEX_CELL + INDEX_MARGIN;
    let columns = (((width - INDEX_MARGIN) / pitch).floor() as usize).max(1);
    let mut cells = Vec::with_capacity(aspects.len());
    let mut top = INDEX_TOP;
    for row in aspects.chunks(columns) {
        let heights: Vec<f32> = row
            .iter()
            .map(|aspect| (INDEX_CELL * aspect.clamp(0.1, 10.0)).round())
            .collect();
        let row_height = heights.iter().copied().fold(0.0, f32::max);
        for (column, height) in heights.iter().enumerate() {
            cells.push(Cell {
                x: INDEX_MARGIN + column as f32 * pitch,
                y: top + ((row_height - height) / 2.0).floor(),
                width: INDEX_CELL,
                height: *height,
            });
        }
        top += row_height + INDEX_ROW_GAP;
    }
    (cells, top - INDEX_ROW_GAP + INDEX_TOP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_panels_come_out_at_the_macs_sizes() {
        let limits = content_limits((1470.0, 923.0));
        assert_eq!(limits.1, 721.0);
        // 800 × 600 PNG alone: 1:1 in an 810 × 643 window.
        assert_eq!(
            window_for_content(fit((800.0, 600.0), limits)),
            (810.0, 643.0)
        );
        // 600 × 800 PNG: 541 × 721 in 551 × 764.
        assert_eq!(
            window_for_content(fit((600.0, 800.0), limits)),
            (551.0, 764.0)
        );
        // Letter PDF: 557 × 721 in 567 × 764.
        assert_eq!(
            window_for_content(fit((612.0, 792.0), limits)),
            (567.0, 764.0)
        );
        // Stepping onto 800 × 600 inside the 721 box of a selection.
        assert_eq!(fit((800.0, 600.0), (721.0, 721.0)), (721.0, 541.0));
        assert_eq!(window_for_content(TEXT_CONTENT), (731.0, 584.0));
    }

    #[test]
    fn index_sheet_matches_the_four_item_capture() {
        let aspects = [800.0 / 600.0, 600.0 / 800.0, 1.0, 792.0 / 612.0];
        let (cells, _) = index_layout(&aspects, 541.0);
        let origins: Vec<(f32, f32)> = cells.iter().map(|cell| (cell.x, cell.y)).collect();
        assert_eq!(
            origins,
            vec![(37.0, 46.0), (289.0, 109.0), (37.0, 424.0), (289.0, 393.0)]
        );
        assert_eq!(cells[0].height, 287.0);
        assert_eq!(cells[1].height, 161.0);
    }
}
