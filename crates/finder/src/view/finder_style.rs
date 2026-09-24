//! Finder geometry and colours measured from macOS 26.2 (Tahoe, dark) on the
//! owner's Mac. `design-lab/finder.html` documents every number and is the
//! 1:1 mock these constants were checked against.
//!
//! Dark-mode colours are the measured values. Light mode has not been
//! measured yet, so it falls back to the shared theme tokens instead of
//! guessing.

use gpui::{hsla, rgb, Hsla};
use rmac_appearance::ResolvedColorScheme;

fn dark() -> bool {
    rmac_ui::theme::current().color_scheme == ResolvedColorScheme::Dark
}

fn white(alpha: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, alpha)
}

fn black(alpha: f32) -> Hsla {
    hsla(0.0, 0.0, 0.0, alpha)
}

// ---- window ----------------------------------------------------------------

/// Unified toolbar height; the content and the first sidebar row start here.
pub(super) const TOOLBAR_HEIGHT: f32 = 52.0;
/// A new Finder window (`open ~`) is 947 × 833.
pub(super) const WINDOW_WIDTH: f32 = 947.0;
pub(super) const WINDOW_HEIGHT: f32 = 833.0;

// ---- sidebar ---------------------------------------------------------------

/// The sidebar is a floating panel inset from the window's left, top and
/// bottom edges; its right edge meets the content.
pub(super) const SIDEBAR_INSET: f32 = 8.0;
pub(super) const SIDEBAR_BOTTOM_INSET: f32 = 8.5;
/// Window radius 27 minus the 8 pt inset keeps the corners concentric.
pub(super) const SIDEBAR_RADIUS: f32 = 19.0;
pub(super) const SIDEBAR_ROW_HEIGHT: f32 = 32.0;
/// Rows (and their selection) are inset 10 from both panel edges.
pub(super) const SIDEBAR_ROW_INSET: f32 = 10.0;
pub(super) const SIDEBAR_ROW_RADIUS: f32 = 8.0;
/// Glyph box centred 19 from the row's left edge; label starts at 35.
pub(super) const SIDEBAR_GLYPH: f32 = 22.0;
pub(super) const SIDEBAR_GLYPH_CENTRE: f32 = 19.0;
pub(super) const SIDEBAR_TEXT_X: f32 = 35.0;
pub(super) const SIDEBAR_TAG_DOT: f32 = 12.0;
/// A section header is a 13 pt gap then a 19 pt row, text 5 in.
pub(super) const SIDEBAR_SECTION_GAP: f32 = 13.0;
pub(super) const SIDEBAR_SECTION_HEIGHT: f32 = 19.0;
pub(super) const SIDEBAR_SECTION_TEXT_X: f32 = 5.0;
/// Traffic-light centres, window-relative: x 26 (then +23, +23), y 26.
pub(super) const TRAFFIC_LIGHT_FIRST_CENTRE: f32 = 26.0;

pub(super) fn sidebar_panel() -> Hsla {
    if dark() {
        // rgb(30,30,39) over the rgb(33,33,46) window.
        black(0.10)
    } else {
        rmac_ui::mac::material_sidebar()
    }
}

pub(super) fn sidebar_panel_edge() -> Hsla {
    if dark() {
        white(0.10)
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn sidebar_selection() -> Hsla {
    if dark() {
        // rgb(44,44,53) over the panel; Tahoe no longer tints this blue.
        white(0.062)
    } else {
        rmac_ui::mac::sidebar_selection()
    }
}

pub(super) fn sidebar_text() -> Hsla {
    if dark() {
        rgb(0xf4f4fd).into()
    } else {
        rmac_ui::mac::text()
    }
}

pub(super) fn sidebar_section_text() -> Hsla {
    if dark() {
        rgb(0x9a9aa5).into()
    } else {
        rmac_ui::mac::text_secondary()
    }
}

// ---- toolbar ---------------------------------------------------------------

/// Toolbar capsules: 36 tall, 8 from the window top, full radius.
pub(super) const CAPSULE_HEIGHT: f32 = 36.0;
pub(super) const CAPSULE_BUTTON: f32 = 36.0;
pub(super) const CAPSULE_PADDING: f32 = 0.5;
/// The selected view mode is a 34 × 28 pill inside its 36 pt button.
pub(super) const CAPSULE_PILL_WIDTH: f32 = 34.0;
pub(super) const CAPSULE_PILL_HEIGHT: f32 = 28.0;
pub(super) const CAPSULE_DIVIDER_HEIGHT: f32 = 20.0;
/// Back/forward capsule starts 8 right of the sidebar column.
pub(super) const TOOLBAR_LEADING_GAP: f32 = 8.0;
/// Title starts 12 after the back/forward capsule; 15 pt bold.
pub(super) const TITLE_GAP: f32 = 12.0;
pub(super) const TITLE_SIZE: f32 = 15.0;
/// Gaps between the trailing capsules, right to left.
pub(super) const TRAILING_MARGIN: f32 = 9.0;
pub(super) const TRAILING_GAP: f32 = 20.0;
pub(super) const VIEW_TO_GROUP_GAP: f32 = 17.5;
pub(super) const GROUP_CAPSULE_WIDTH: f32 = 47.5;
pub(super) const TOOLBAR_GLYPH: f32 = 20.0;
pub(super) const TOOLBAR_CHEVRON_GLYPH: f32 = 22.0;

pub(super) fn capsule_fill() -> Hsla {
    if dark() {
        white(0.02)
    } else {
        rmac_ui::mac::material_clear()
    }
}

pub(super) fn capsule_edge() -> Hsla {
    if dark() {
        white(0.14)
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn capsule_selected() -> Hsla {
    if dark() {
        white(0.16)
    } else {
        rmac_ui::mac::control_fill_hover()
    }
}

pub(super) fn capsule_divider() -> Hsla {
    if dark() {
        white(0.10)
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn toolbar_glyph() -> Hsla {
    if dark() {
        rgb(0xe8e8ea).into()
    } else {
        rmac_ui::mac::text()
    }
}

// ---- content ---------------------------------------------------------------

pub(super) fn primary_text() -> Hsla {
    if dark() {
        rgb(0xdfdfe1).into()
    } else {
        rmac_ui::mac::text()
    }
}

pub(super) fn secondary_text() -> Hsla {
    if dark() {
        rgb(0x9f9fa5).into()
    } else {
        rmac_ui::mac::text_secondary()
    }
}

/// Header, path-bar and status-bar text.
pub(super) fn chrome_text() -> Hsla {
    if dark() {
        rgb(0x9b9ba1).into()
    } else {
        rmac_ui::mac::text_secondary()
    }
}

pub(super) fn sorted_header_text() -> Hsla {
    if dark() {
        rgb(0xdcdcde).into()
    } else {
        rmac_ui::mac::text()
    }
}

/// Hairline under the list header and above the path bar.
pub(super) fn hairline() -> Hsla {
    if dark() {
        white(0.15)
    } else {
        rmac_ui::mac::separator()
    }
}

/// Hairline above the status bar and between browser columns.
pub(super) fn dark_rule() -> Hsla {
    if dark() {
        black(1.0)
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn header_divider() -> Hsla {
    if dark() {
        white(0.10)
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn stripe() -> Hsla {
    if dark() {
        // rgb(43,43,56) over rgb(33,33,46).
        white(0.045)
    } else {
        rmac_ui::mac::row_alternate()
    }
}

/// Selected rows while the window is key (Blue accent).
pub(super) fn selection_focused() -> Hsla {
    if dark() {
        rgb(0x2558c9).into()
    } else {
        rmac_ui::mac::accent()
    }
}

/// Selected rows while another window is key.
pub(super) fn selection_unfocused() -> Hsla {
    if dark() {
        rgb(0x464646).into()
    } else {
        rmac_ui::mac::sidebar_selection()
    }
}

pub(super) fn selection(active: bool) -> Hsla {
    if active {
        selection_focused()
    } else {
        selection_unfocused()
    }
}

/// Text on a selected row: white when key, the ordinary label otherwise.
pub(super) fn selected_text(active: bool) -> Hsla {
    if active {
        rmac_ui::mac::on_accent()
    } else {
        primary_text()
    }
}

/// The plate drawn behind a selected icon in icon view.
pub(super) fn icon_plate() -> Hsla {
    if dark() {
        white(0.10)
    } else {
        black(0.06)
    }
}

// ---- list view -------------------------------------------------------------

pub(super) const LIST_HEADER_HEIGHT: f32 = 28.0;
pub(super) const LIST_HEADER_TEXT: f32 = 11.0;
pub(super) const LIST_ROWS_TOP: f32 = 5.0;
pub(super) const LIST_ROW_HEIGHT: f32 = 20.0;
pub(super) const LIST_ROW_INSET: f32 = 10.0;
pub(super) const ROW_RADIUS: f32 = 6.0;
/// Within a row (after the 10 pt inset): disclosure 1…16, icon 16, name 36.
pub(super) const LIST_DISCLOSURE_WIDTH: f32 = 15.0;
pub(super) const LIST_DISCLOSURE_X: f32 = 1.0;
pub(super) const LIST_ICON: f32 = 16.0;
pub(super) const LIST_ICON_TO_NAME: f32 = 4.0;
pub(super) const LIST_CELL_TEXT_X: f32 = 6.0;
pub(super) const LIST_SIZE_TRAILING: f32 = 5.0;
pub(super) const LIST_HEADER_DIVIDER_HEIGHT: f32 = 16.0;

// ---- icon view -------------------------------------------------------------

pub(super) const ICON_GRID_LEFT: f32 = 11.0;
pub(super) const ICON_GRID_TOP: f32 = 24.0;
/// Cell pitch for the 64 pt default; it scales with the icon size.
pub(super) const ICON_CELL_EXTRA_WIDTH: f32 = 64.0;
pub(super) const ICON_CELL_EXTRA_HEIGHT: f32 = 52.0;
pub(super) const ICON_LABEL_GAP: f32 = 6.0;
pub(super) const ICON_LABEL_SIZE: f32 = 12.0;
pub(super) const ICON_LABEL_MAX_WIDTH: f32 = 112.0;
pub(super) const ICON_PLATE_GROW: f32 = 4.0;
pub(super) const ICON_PLATE_RADIUS: f32 = 8.0;
pub(super) const ICON_LABEL_RADIUS: f32 = 5.0;

// ---- column view -----------------------------------------------------------

pub(super) const COLUMN_WIDTH: f32 = 245.0;
pub(super) const COLUMN_ROWS_TOP: f32 = 5.0;
pub(super) const COLUMN_ROW_HEIGHT: f32 = 22.0;
pub(super) const COLUMN_ROW_INSET: f32 = 10.0;
pub(super) const COLUMN_ICON_X: f32 = 7.0;
pub(super) const COLUMN_TEXT_X: f32 = 26.0;
/// A file's preview column: content inset 10, artwork up to 128 on top,
/// the name 10 below it.
pub(super) const COLUMN_PREVIEW_INSET: f32 = 10.0;
pub(super) const COLUMN_PREVIEW_ARTWORK: f32 = 128.0;
pub(super) const COLUMN_PREVIEW_TEXT_GAP: f32 = 10.0;

// ---- gallery view ----------------------------------------------------------

pub(super) const GALLERY_INSPECTOR_WIDTH: f32 = 250.0;
pub(super) const GALLERY_THUMB: f32 = 48.0;
pub(super) const GALLERY_THUMB_PITCH: f32 = 59.0;
/// Inspector: text 9 in from its left edge (11 from the window's right),
/// the name 26 below the toolbar, "Information" 24 below the summary line,
/// then 23 pt rows split by hairlines.
pub(super) const GALLERY_INSPECTOR_INSET: f32 = 9.0;
pub(super) const GALLERY_INSPECTOR_TRAILING: f32 = 11.0;
pub(super) const GALLERY_INSPECTOR_TITLE_TOP: f32 = 26.0;
pub(super) const GALLERY_INSPECTOR_SECTION_GAP: f32 = 24.0;
pub(super) const GALLERY_INSPECTOR_ROW_HEIGHT: f32 = 23.0;

// ---- Get Info ------------------------------------------------------------------

/// Finder's info window is 265 wide; its close button is centred 16 in.
pub(super) const INFO_WIDTH: f32 = 265.0;
pub(super) const INFO_MAX_HEIGHT: f32 = 620.0;
pub(super) const INFO_TITLE_HEIGHT: f32 = 32.0;
pub(super) const INFO_TITLE_TEXT_INSET: f32 = 40.0;
pub(super) const INFO_CLOSE: f32 = 14.0;
pub(super) const INFO_CLOSE_CENTRE: f32 = 16.0;
/// Header icon 32; sections inset 10 with a 12 pt title row 25 tall.
pub(super) const INFO_HEADER_ICON: f32 = 32.0;
pub(super) const INFO_SECTION_INSET: f32 = 10.0;
pub(super) const INFO_SECTION_HEADER: f32 = 25.0;
pub(super) const INFO_SECTION_TEXT: f32 = 12.0;
/// Rows: 12 pt on a 14 pt pitch; labels end 67 in, values start 5.5 later.
pub(super) const INFO_ROW_TEXT: f32 = 12.0;
pub(super) const INFO_ROW_LINE: f32 = 14.0;
pub(super) const INFO_ROW_PITCH: f32 = 14.0;
pub(super) const INFO_LABEL_RIGHT: f32 = 67.0;
pub(super) const INFO_LABEL_GAP: f32 = 5.5;
pub(super) const INFO_PREVIEW_HEIGHT: f32 = 130.0;

// ---- path and status bars --------------------------------------------------

pub(super) const PATH_BAR_HEIGHT: f32 = 28.0;
pub(super) const PATH_BAR_LEADING: f32 = 14.0;
pub(super) const PATH_BAR_ICON: f32 = 13.0;
pub(super) const PATH_BAR_ICON_GAP: f32 = 5.0;
pub(super) const PATH_BAR_SEPARATOR_MARGIN: f32 = 6.0;
pub(super) const STATUS_BAR_HEIGHT: f32 = 28.0;
pub(super) const STATUS_TEXT: f32 = 11.0;
pub(super) const STATUS_SLIDER_WIDTH: f32 = 82.0;
pub(super) const STATUS_SLIDER_TRAILING: f32 = 14.0;

// ---- behaviour -------------------------------------------------------------

/// Type-to-select keeps extending its prefix while keys arrive within this
/// window (AppKit's type-select interval is about one second).
pub(super) const TYPE_SELECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);
/// A folder springs open after a dragged item hovers it this long.
pub(super) const SPRING_LOADING_DELAY: std::time::Duration = std::time::Duration::from_millis(700);

/// Striped filler rows drawn below the last list row (enough for a tall
/// display; the container clips the rest).
pub(super) const FILLER_STRIPES: usize = 80;

/// Icon-view marquee. Not yet measured on the Mac: a light wash with a
/// brighter edge, derived from the selection plate.
pub(super) fn marquee_fill() -> Hsla {
    icon_plate()
}

pub(super) fn marquee_edge() -> Hsla {
    if dark() {
        white(0.35)
    } else {
        black(0.25)
    }
}
