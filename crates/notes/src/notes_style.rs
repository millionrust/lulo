//! Notes geometry and colours measured from macOS 26.2 Notes (Tahoe, dark)
//! on the owner's Mac. `design-lab/apps.html` documents every number and is
//! the 1:1 mock these constants were checked against. Column widths are the
//! owner's split, measured in a 1470 pt window.
//!
//! Dark-mode colours are the measured values. Light mode has not been
//! measured, so it falls back to the shared theme tokens instead of guessing.

use gpui::{hsla, rgb, Hsla};
use rmac_ui::mac;

fn dark() -> bool {
    mac::window().l < 0.5
}

fn white(alpha: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, alpha)
}

/// A colour measured on the Mac in dark mode, as 0xRRGGBB.
fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn measured(dark_value: u32, light: Hsla) -> Hsla {
    if dark() {
        hex(dark_value)
    } else {
        light
    }
}

// ---- window ----------------------------------------------------------------

/// Unified toolbar height; it has no base line and takes each column's fill.
pub(super) const TOOLBAR_HEIGHT: f32 = 52.0;
/// The Mac's toolbar controls: 36 tall, 8 from the window top.
pub(super) const CAPSULE_HEIGHT: f32 = 36.0;
pub(super) const CAPSULE_TOP: f32 = 8.0;
/// A glyph button inside a capsule.
pub(super) const CAPSULE_BUTTON_WIDTH: f32 = 38.0;
pub(super) const TOOLBAR_GLYPH: f32 = 18.0;

// ---- sidebar ---------------------------------------------------------------

/// The folder sidebar is a floating panel inset 8 from the window's left,
/// top and bottom; its right edge meets the note list at x 221.
pub(super) const SIDEBAR_WIDTH: f32 = 221.0;
pub(super) const SIDEBAR_INSET: f32 = 8.0;
pub(super) const SIDEBAR_BOTTOM_INSET: f32 = 8.5;
/// Window radius 27 minus the 8 pt inset (Finder's measured panel).
pub(super) const SIDEBAR_RADIUS: f32 = 19.0;
pub(super) const SIDEBAR_ROW_HEIGHT: f32 = 32.0;
/// Row selection inset from the panel's edges.
pub(super) const SIDEBAR_ROW_INSET: f32 = 11.0;
pub(super) const SIDEBAR_ROW_RADIUS: f32 = 8.0;
/// Folder glyph centred 14 from the row's left edge; label at 29.5.
pub(super) const SIDEBAR_GLYPH: f32 = 18.0;
pub(super) const SIDEBAR_GLYPH_CENTRE: f32 = 14.0;
pub(super) const SIDEBAR_TEXT_X: f32 = 29.5;
pub(super) const SIDEBAR_COUNT_RIGHT: f32 = 8.0;
/// Section header: 19 tall, text 16.5 from the panel's left edge.
pub(super) const SIDEBAR_SECTION_HEIGHT: f32 = 19.0;
pub(super) const SIDEBAR_SECTION_TEXT_X: f32 = 16.5;
/// Traffic-light centres, window-relative: x 26 (then +23, +23), y 26.
pub(super) const TRAFFIC_LIGHT_FIRST_CENTRE: f32 = 26.0;

pub(super) fn window_frame() -> Hsla {
    measured(0x1f202c, mac::window())
}

pub(super) fn sidebar_panel() -> Hsla {
    measured(0x1d1e2a, mac::material_sidebar())
}

pub(super) fn sidebar_panel_edge() -> Hsla {
    measured(0x404665, mac::separator())
}

pub(super) fn sidebar_selection() -> Hsla {
    if dark() {
        white(0.06)
    } else {
        mac::sidebar_selection()
    }
}

pub(super) fn sidebar_text() -> Hsla {
    mac::text()
}

pub(super) fn sidebar_selected_text() -> Hsla {
    measured(0xf2bb4b, mac::notes_accent())
}

pub(super) fn folder_glyph() -> Hsla {
    measured(0xffce40, mac::notes_accent())
}

pub(super) fn sidebar_count() -> Hsla {
    mac::text_tertiary()
}

pub(super) fn sidebar_selected_count() -> Hsla {
    measured(0x5f5f65, mac::text_tertiary())
}

pub(super) fn sidebar_section_text() -> Hsla {
    measured(0x5d5e67, mac::text_secondary())
}

// ---- note list -------------------------------------------------------------

pub(super) const LIST_WIDTH: f32 = 346.0;
/// Folder name and count in the toolbar above the list.
pub(super) const LIST_TITLE_X: f32 = 21.0;
/// View Options: a 36 circle whose right edge is 8 from the column rule.
pub(super) const LIST_MORE_RIGHT: f32 = 8.0;
/// The list starts 10 below the toolbar; a date section is a 40 pt row
/// ("Today" 15 bold at x 16.5) with a full-width rule 29 below its top.
pub(super) const LIST_TOP_PADDING: f32 = 10.0;
pub(super) const SECTION_HEIGHT: f32 = 40.0;
pub(super) const SECTION_TEXT_X: f32 = 16.5;
pub(super) const SECTION_RULE_Y: f32 = 29.0;
/// Note rows: 56 pitch, selection 55 tall inset 10 from both sides,
/// radius 8, text 26 in from the selection's left edge.
pub(super) const NOTE_ROW_HEIGHT: f32 = 56.0;
pub(super) const NOTE_ROW_INSET: f32 = 10.0;
pub(super) const NOTE_ROW_RADIUS: f32 = 8.0;
pub(super) const NOTE_TEXT_X: f32 = 26.0;
/// Time and preview on the second line are 9 apart.
pub(super) const NOTE_TIME_GAP: f32 = 9.0;

pub(super) fn list_fill() -> Hsla {
    measured(0x21222d, mac::list())
}

pub(super) fn column_rule() -> Hsla {
    measured(0x000000, mac::separator())
}

pub(super) fn list_title() -> Hsla {
    measured(0xe9e9ea, mac::text())
}

pub(super) fn list_subtitle() -> Hsla {
    measured(0x909096, mac::text_secondary())
}

pub(super) fn section_text() -> Hsla {
    measured(0xdddddf, mac::text())
}

pub(super) fn section_rule() -> Hsla {
    if dark() {
        white(0.10)
    } else {
        mac::separator()
    }
}

pub(super) fn row_rule() -> Hsla {
    if dark() {
        white(0.08)
    } else {
        mac::separator()
    }
}

/// The selected note: yellow while the list has focus, grey otherwise.
pub(super) fn selection_fill(list_focused: bool) -> Hsla {
    match (dark(), list_focused) {
        (true, true) => hex(0x99833f),
        (true, false) => hex(0x464646),
        (false, true) => mac::notes_selection(),
        (false, false) => mac::control_fill_hover(),
    }
}

pub(super) fn selection_text(list_focused: bool) -> Hsla {
    match (dark(), list_focused) {
        (true, true) => hex(0xefece2),
        (true, false) => hex(0xe3e3e3),
        (false, _) => mac::text(),
    }
}

pub(super) fn selection_preview(list_focused: bool) -> Hsla {
    match (dark(), list_focused) {
        (true, true) => hex(0xd1c7a8),
        (true, false) => hex(0xacacac),
        (false, _) => mac::text_secondary(),
    }
}

// ---- editor ----------------------------------------------------------------

/// Text starts 22 in from the editor's left edge (ink at 23).
pub(super) const EDITOR_INSET: f32 = 22.0;
/// Toolbar: compose 8.5 from the column rule; the format capsule floats
/// centred between compose and the ⋯ capsule; search ends 8 from the edge,
/// 15.5 after the ⋯ capsule.
pub(super) const COMPOSE_LEFT: f32 = 8.5;
pub(super) const SEARCH_GAP: f32 = 15.5;
pub(super) const TRAILING_MARGIN: f32 = 8.0;
pub(super) const SEARCH_MAX_WIDTH: f32 = 326.0;
pub(super) const SEARCH_MIN_WIDTH: f32 = 150.0;
/// Date 12 pt, baseline 20 below the toolbar; title 20 bold, baseline 51.5;
/// body 13 on a 17 pitch.
pub(super) const DATE_SIZE: f32 = 12.0;
pub(super) const DATE_TOP: f32 = 8.0;
pub(super) const DATE_LINE: f32 = 15.0;
pub(super) const TITLE_SIZE: f32 = 20.0;
pub(super) const TITLE_LINE: f32 = 24.0;
pub(super) const TITLE_TOP: f32 = 9.0;
pub(super) const BODY_SIZE: f32 = 13.0;
pub(super) const BODY_LINE: f32 = 17.0;

pub(super) fn editor_fill() -> Hsla {
    measured(0x1e1e1e, mac::window())
}

pub(super) fn editor_date() -> Hsla {
    measured(0x808080, mac::text_secondary())
}

pub(super) fn editor_text() -> Hsla {
    measured(0xdcdcdc, mac::text())
}

pub(super) fn capsule_fill() -> Hsla {
    if dark() {
        white(0.02)
    } else {
        mac::material_clear()
    }
}

pub(super) fn capsule_edge() -> Hsla {
    if dark() {
        white(0.11)
    } else {
        mac::separator()
    }
}

pub(super) fn toolbar_glyph() -> Hsla {
    measured(0xe9e9e9, mac::text())
}

pub(super) fn search_placeholder() -> Hsla {
    mac::text_secondary()
}
