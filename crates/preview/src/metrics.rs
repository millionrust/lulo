//! Measured macOS 26 Preview geometry and colours (dark appearance), in
//! logical points from the window's top-left. Source: Accessibility frames
//! and Retina pixels ÷ 2 captured 2026-09-23; see design-lab/preview.html.

/// A PDF window opens 1121 × 789; images open at their size plus the
/// toolbar, fitted into the same box.
pub const DEFAULT_WINDOW: (f32, f32) = (1121.0, 789.0);
pub const TOOLBAR_HEIGHT: f32 = 52.0;
/// Close button centre; the lights sit on a 23 pt pitch.
pub const TRAFFIC_LIGHT_CENTER: (f32, f32) = (26.0, 26.0);

/// Toolbar controls: 36 pt tall capsules 8 pt from the top.
pub const CONTROL_HEIGHT: f32 = 36.0;
pub const CONTROL_TOP: f32 = 8.0;
pub const GLYPH_SIZE: f32 = 21.0;
pub const DIVIDER_HEIGHT: f32 = 20.0;

/// Sidebar toggle: a capsule while the sidebar is hidden, a bare glyph
/// inside the sidebar panel while it shows.
pub const SIDEBAR_TOGGLE_LEFT: f32 = 103.5;
pub const SIDEBAR_TOGGLE_WIDTH: f32 = 50.5;
pub const SIDEBAR_TOGGLE_SHOWN_SHIFT: f32 = 10.0;
pub const SIDEBAR_GLYPH_SIZE: f32 = 23.0;
pub const CHEVRON_SIZE: f32 = 8.0;

/// Title origin with the sidebar hidden / shown.
pub const TITLE_LEFT: f32 = 175.0;
pub const TITLE_LEFT_WITH_SIDEBAR: f32 = 188.0;
pub const TITLE_SINGLE_SIZE: f32 = 15.0;
pub const TITLE_SIZE: f32 = 13.0;
pub const TITLE_TOP: f32 = 11.0;
pub const TITLE_LINE: f32 = 16.0;
pub const SUBTITLE_SIZE: f32 = 11.0;
pub const SUBTITLE_TOP: f32 = 27.0;
pub const SUBTITLE_LINE: f32 = 14.0;

/// Right-hand toolbar group, laid out from the window's right edge.
pub const RIGHT_INSET: f32 = 8.0;
pub const SEARCH_WIDTH: f32 = 177.0;
pub const SEARCH_GLYPH_LEFT: f32 = 11.5;
pub const SEARCH_TEXT_LEFT: f32 = 39.0;
pub const GAP_SEARCH_INFO: f32 = 10.5;
pub const GAP_INFO_ROTATE: f32 = 18.5;
pub const GAP_ROTATE_ZOOM: f32 = 16.5;
pub const ZOOM_GROUP_WIDTH: f32 = 110.0;
pub const ZOOM_BUTTON_WIDTH: f32 = 36.0;

/// Floating sidebar panel.
pub const SIDEBAR_INSET: f32 = 8.0;
pub const SIDEBAR_WIDTH: f32 = 160.0;
pub const SIDEBAR_RADIUS: f32 = 19.0;
/// Thumbnails start at x 28 (selection at 22) inside the window.
pub const THUMB_LEFT: f32 = 28.0;
pub const THUMB_SELECTION_LEFT: f32 = 22.0;
pub const THUMB_SELECTION_WIDTH: f32 = 132.0;
pub const THUMB_SELECTION_RADIUS: f32 = 8.0;
pub const THUMB_LABEL_SIZE: f32 = 13.0;

/// The document column starts where the sidebar panel ends.
pub fn document_left(sidebar: bool) -> f32 {
    if sidebar {
        SIDEBAR_INSET + SIDEBAR_WIDTH
    } else {
        0.0
    }
}

/// Get Info inspector on the right.
pub const INSPECTOR_WIDTH: f32 = 279.0;
pub const INSPECTOR_CARD_LEFT: f32 = 19.0;
pub const INSPECTOR_CARD_RIGHT: f32 = 10.0;
pub const INSPECTOR_TOP: f32 = 8.0;
pub const INSPECTOR_ROW: f32 = 36.0;
pub const INSPECTOR_CARD_GAP: f32 = 10.0;
pub const INSPECTOR_CARD_RADIUS: f32 = 10.0;
pub const INSPECTOR_TEXT_INSET: f32 = 10.0;
pub const INSPECTOR_LABEL_SIZE: f32 = 12.0;

/// Colours (0xRRGGBB), dark appearance.
pub mod dark {
    pub const WINDOW: u32 = 0x20222C;
    pub const CONTROL_FILL: u32 = 0x262737;
    pub const CONTROL_EDGE: u32 = 0x434567;
    pub const DIVIDER: u32 = 0x383949;
    pub const GLYPH: u32 = 0xE9E9EB;
    pub const SUBTITLE: u32 = 0x8F9095;
    pub const PLACEHOLDER: u32 = 0x92939B;
    pub const DOCUMENT: u32 = 0x262734;
    pub const SIDEBAR: u32 = 0x20212B;
    pub const SIDEBAR_EDGE: u32 = 0x41435E;
    pub const SELECTION: u32 = 0x3478F6;
    pub const THUMB_LABEL: u32 = 0xC8C8CD;
    pub const INSPECTOR: u32 = 0x1E1F2B;
    pub const INSPECTOR_SEPARATOR: u32 = 0x4B4C55;
    pub const CARD: u32 = 0x292A34;
    pub const CARD_SEPARATOR: u32 = 0x3A3A44;
    pub const CARD_LABEL: u32 = 0xF5F5F6;
    pub const CARD_VALUE: u32 = 0x9D9DA4;
    /// Search hits: Preview's yellow find highlight (not measured on this
    /// capture; the system find-indicator yellow).
    pub const FIND_HIGHLIGHT: u32 = 0xFFD60A;
}

/// Light appearance (not captured on the Mac for Preview; derived from the
/// system light window and control colours).
pub mod light {
    pub const WINDOW: u32 = 0xF4F4F6;
    pub const CONTROL_FILL: u32 = 0xFFFFFF;
    pub const CONTROL_EDGE: u32 = 0xD6D6DC;
    pub const DIVIDER: u32 = 0xDCDCE0;
    pub const GLYPH: u32 = 0x3C3C43;
    pub const SUBTITLE: u32 = 0x86868B;
    pub const PLACEHOLDER: u32 = 0x8E8E93;
    pub const DOCUMENT: u32 = 0xE9E9ED;
    pub const SIDEBAR: u32 = 0xEDEDF0;
    pub const SIDEBAR_EDGE: u32 = 0xD6D6DC;
    pub const SELECTION: u32 = 0x0A84FF;
    pub const THUMB_LABEL: u32 = 0x3C3C43;
    pub const INSPECTOR: u32 = 0xF2F2F5;
    pub const INSPECTOR_SEPARATOR: u32 = 0xD6D6DC;
    pub const CARD: u32 = 0xFFFFFF;
    pub const CARD_SEPARATOR: u32 = 0xE5E5EA;
    pub const CARD_LABEL: u32 = 0x1D1D1F;
    pub const CARD_VALUE: u32 = 0x6E6E73;
    pub const FIND_HIGHLIGHT: u32 = 0xFFD60A;
}

/// Where each right-hand control starts (x of its left edge) for a window
/// `width` wide. Images have no search field (search is for PDF text), so
/// their controls move right into its place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RightGroup {
    pub search: f32,
    pub info: f32,
    pub rotate: f32,
    pub zoom: f32,
}

pub fn right_group(width: f32, search_field: bool) -> RightGroup {
    let search = width - RIGHT_INSET - SEARCH_WIDTH;
    let info = if search_field {
        search - GAP_SEARCH_INFO - CONTROL_HEIGHT
    } else {
        width - RIGHT_INSET - CONTROL_HEIGHT
    };
    let rotate = info - GAP_INFO_ROTATE - CONTROL_HEIGHT;
    let zoom = rotate - GAP_ROTATE_ZOOM - ZOOM_GROUP_WIDTH;
    RightGroup {
        search,
        info,
        rotate,
        zoom,
    }
}

/// Window size for an image of `pixels`: the image at actual size under the
/// toolbar, scaled down to fit the default window box.
pub fn image_window_size(pixels: (f32, f32)) -> (f32, f32) {
    let (max_width, max_height) = DEFAULT_WINDOW;
    let content_height = max_height - TOOLBAR_HEIGHT;
    let (width, height) = pixels;
    if width <= 0.0 || height <= 0.0 {
        return DEFAULT_WINDOW;
    }
    let scale = (max_width / width).min(content_height / height).min(1.0);
    // Keep the whole toolbar usable on tiny images.
    let min_width =
        TITLE_LEFT + (DEFAULT_WINDOW.0 - right_group(DEFAULT_WINDOW.0, true).zoom) + 60.0;
    (
        (width * scale).round().max(min_width.min(max_width)),
        (height * scale).round() + TOOLBAR_HEIGHT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_group_matches_the_measured_toolbar() {
        let group = right_group(1121.0, true);
        assert_eq!(group.search, 936.0);
        assert_eq!(group.info, 889.5);
        assert_eq!(group.rotate, 835.0);
        assert_eq!(group.zoom, 708.5);
        let images = right_group(800.0, false);
        assert_eq!(images.info, 756.0);
        assert_eq!(images.zoom, 756.0 - 18.5 - 36.0 - 16.5 - 110.0);
    }

    #[test]
    fn image_windows_size_to_the_image() {
        // Measured: an 800×600 PNG opens an 800 × 652 window.
        assert_eq!(image_window_size((800.0, 600.0)), (800.0, 652.0));
        let (width, height) = image_window_size((4000.0, 3000.0));
        assert!(width <= 1121.0 && height <= 789.0);
        assert!((width / (height - TOOLBAR_HEIGHT) - 4.0 / 3.0).abs() < 0.01);
        assert_eq!(image_window_size((0.0, 10.0)), DEFAULT_WINDOW);
        // Tiny images keep room for the toolbar.
        assert!(image_window_size((16.0, 16.0)).0 > 500.0);
    }

    #[test]
    fn document_column_starts_after_the_sidebar() {
        assert_eq!(document_left(true), 168.0);
        assert_eq!(document_left(false), 0.0);
    }
}
