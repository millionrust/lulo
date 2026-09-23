//! System Settings geometry and colours measured from macOS 26.2 (Tahoe,
//! dark) on the owner's Mac. `design-lab/settings.html` documents every
//! number and is the 1:1 mock these constants were checked against.
//!
//! Dark-mode colours are the measured values. Light mode has not been
//! measured yet, so it falls back to the shared theme tokens instead of
//! guessing.

use gpui::{hsla, rgb, Hsla};
use rmac_appearance::ResolvedColorScheme;

pub(super) fn dark() -> bool {
    rmac_ui::theme::current().color_scheme == ResolvedColorScheme::Dark
}

fn white(alpha: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, alpha)
}

// ---- window ----------------------------------------------------------------

/// A new Settings window on the Mac is 723 × 832.
pub(super) const WINDOW_WIDTH: f32 = 723.0;
pub(super) const WINDOW_HEIGHT: f32 = 832.0;
pub(super) const TOOLBAR_HEIGHT: f32 = 52.0;

// ---- sidebar ---------------------------------------------------------------

/// The sidebar is a floating panel 8 in from the window's left, top and
/// bottom edges; the detail column starts 8 after it (x 223).
pub(super) const SIDEBAR_INSET: f32 = 8.0;
pub(super) const SIDEBAR_PANEL_WIDTH: f32 = 215.0;
pub(super) const SIDEBAR_COLUMN_WIDTH: f32 = SIDEBAR_INSET + SIDEBAR_PANEL_WIDTH;
/// Window radius 27 minus the inset keeps the corners concentric.
pub(super) const SIDEBAR_RADIUS: f32 = 19.0;
/// Search field: x 18, y 61 (window), 195 × 28, fully round.
pub(super) const SEARCH_TOP: f32 = 53.0;
pub(super) const SEARCH_HEIGHT: f32 = 28.0;
pub(super) const SEARCH_GLYPH: f32 = 15.0;
/// The list scrolls from y 98 (window), 9 under the search field.
pub(super) const LIST_TOP_GAP: f32 = 9.0;
/// Rows and the search field are inset 10 inside the panel.
pub(super) const SIDEBAR_ROW_INSET: f32 = 10.0;
pub(super) const SIDEBAR_ROW_HEIGHT: f32 = 32.0;
pub(super) const SIDEBAR_ROW_RADIUS: f32 = 8.0;
/// Icon tile 20 at row + 6; label at row + 31.
pub(super) const SIDEBAR_ICON: f32 = 20.0;
pub(super) const SIDEBAR_ICON_X: f32 = 6.0;
pub(super) const SIDEBAR_LABEL_X: f32 = 31.0;
pub(super) const SIDEBAR_SECTION_GAP: f32 = 13.0;
/// The account row is 46 tall with a 38 pt avatar; text starts at row + 52.
pub(super) const ACCOUNT_ROW_HEIGHT: f32 = 46.0;
pub(super) const ACCOUNT_AVATAR: f32 = 38.0;
pub(super) const ACCOUNT_TEXT_X: f32 = 52.0;

pub(super) fn window_fill() -> Hsla {
    if dark() {
        rgb(0x20212d).into()
    } else {
        rmac_ui::mac::window()
    }
}

pub(super) fn sidebar_panel() -> Hsla {
    if dark() {
        rgb(0x1d1d28).into()
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

pub(super) fn search_fill() -> Hsla {
    if dark() {
        rgb(0x2e2f3a).into()
    } else {
        rmac_ui::mac::control_fill()
    }
}

pub(super) fn search_glyph() -> Hsla {
    if dark() {
        rgb(0xa1a2a6).into()
    } else {
        rmac_ui::mac::text_secondary()
    }
}

pub(super) fn sidebar_text() -> Hsla {
    if dark() {
        rgb(0xf4f4f4).into()
    } else {
        rmac_ui::mac::text()
    }
}

/// Selection while the sidebar holds keyboard focus in the key window.
pub(super) fn sidebar_selection_focused() -> Hsla {
    if dark() {
        rgb(0x2458ca).into()
    } else {
        rmac_ui::mac::accent()
    }
}

/// Selection otherwise: a neutral grey, never the accent.
pub(super) fn sidebar_selection() -> Hsla {
    if dark() {
        rgb(0x464646).into()
    } else {
        rmac_ui::mac::sidebar_selection()
    }
}

// ---- toolbar ---------------------------------------------------------------

/// Back/forward: one 73 × 36 capsule, 8 from the top and 8 after the
/// sidebar column; two 36 pt segments around a 1 pt divider.
pub(super) const CAPSULE_LEADING: f32 = 8.0;
pub(super) const CAPSULE_HEIGHT: f32 = 36.0;
pub(super) const CAPSULE_SEGMENT: f32 = 36.0;
pub(super) const CAPSULE_DIVIDER_HEIGHT: f32 = 20.0;
pub(super) const CAPSULE_GLYPH: f32 = 17.0;
/// The title starts 12 after the capsule: 15 pt bold.
pub(super) const TITLE_GAP: f32 = 12.0;
pub(super) const TITLE_SIZE: f32 = 15.0;

pub(super) fn capsule_fill() -> Hsla {
    if dark() {
        rgb(0x272838).into()
    } else {
        rmac_ui::mac::material_clear()
    }
}

pub(super) fn capsule_edge() -> Hsla {
    if dark() {
        rgb(0x383a54).into()
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn capsule_divider() -> Hsla {
    if dark() {
        white(0.10)
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn toolbar_glyph(enabled: bool) -> Hsla {
    match (dark(), enabled) {
        (true, true) => rgb(0xe9e9eb).into(),
        (true, false) => rgb(0x696a79).into(),
        (false, true) => rmac_ui::mac::text(),
        (false, false) => rmac_ui::mac::text_tertiary(),
    }
}

pub(super) fn title_text() -> Hsla {
    if dark() {
        rgb(0xe8e8ea).into()
    } else {
        rmac_ui::mac::text()
    }
}

// ---- detail ----------------------------------------------------------------

/// Content is inset 20 from the detail column: 460 wide in a 723 window.
pub(super) const DETAIL_INSET: f32 = 20.0;
pub(super) const DETAIL_CONTENT_WIDTH: f32 = 460.0;
pub(super) const GROUP_RADIUS: f32 = 12.0;
pub(super) const GROUP_GAP: f32 = 10.0;
/// A form row is 37 tall with 10 padding; separators are 1 pt, inset 10.
pub(super) const ROW_HEIGHT: f32 = 37.0;
pub(super) const ROW_PADDING: f32 = 10.0;
pub(super) const SEPARATOR: f32 = 1.0;
/// Navigation rows (General's list) are 42 tall; icon 20, label at 40.
pub(super) const NAV_ROW_HEIGHT: f32 = 42.0;
pub(super) const ROW_ICON: f32 = 20.0;
pub(super) const NAV_ICON_GAP: f32 = 10.0;
pub(super) const NAV_CHEVRON: f32 = 14.0;
pub(super) const NAV_TRAILING: f32 = 14.0;
/// Section heads: 13 bold, 30 below the previous group (its 10 gap + 20)
/// and 10 above their own group; the first one sits 3 below the toolbar.
pub(super) const SECTION_TOP: f32 = 20.0;
pub(super) const SECTION_BOTTOM: f32 = 10.0;
pub(super) const FIRST_SECTION_TOP: f32 = 3.0;
/// General's hero: a 164 pt group, icon 52 at 24, title 22 bold.
pub(super) const HERO_HEIGHT: f32 = 164.0;
pub(super) const HERO_ICON: f32 = 52.0;
pub(super) const HERO_ICON_TOP: f32 = 24.0;
pub(super) const HERO_TITLE: f32 = 22.0;
pub(super) const HERO_TEXT_WIDTH: f32 = 397.0;
/// Wi-Fi / Bluetooth header card icon.
pub(super) const HEADER_ICON: f32 = 26.0;
/// The circled ▶ / (i) buttons at the end of a row.
pub(super) const INFO_BUTTON: f32 = 17.0;

pub(super) fn group_fill() -> Hsla {
    if dark() {
        rgb(0x272833).into()
    } else {
        rmac_ui::mac::raised()
    }
}

pub(super) fn group_separator() -> Hsla {
    if dark() {
        rgb(0x31323d).into()
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn label_text() -> Hsla {
    if dark() {
        rgb(0xdedee0).into()
    } else {
        rmac_ui::mac::text()
    }
}

pub(super) fn secondary_text() -> Hsla {
    if dark() {
        rgb(0x9e9ea3).into()
    } else {
        rmac_ui::mac::text_secondary()
    }
}

pub(super) fn heading_text() -> Hsla {
    if dark() {
        rgb(0xdcdcde).into()
    } else {
        rmac_ui::mac::text()
    }
}

pub(super) fn chevron() -> Hsla {
    if dark() {
        rgb(0x5c5d65).into()
    } else {
        rmac_ui::mac::text_tertiary()
    }
}

/// Push buttons, the help circle and the pop-up chevron circle.
pub(super) fn control_fill() -> Hsla {
    if dark() {
        rgb(0x373843).into()
    } else {
        rmac_ui::mac::button_secondary()
    }
}

// ---- second wave (Network … Storage) -------------------------------------

/// Icon rows (Network services, notification apps, Focus modes, background
/// items): 52 tall with a 26 pt icon at x + 11 and the text at x + 48.
pub(super) const LARGE_ROW_HEIGHT: f32 = 52.0;
pub(super) const LARGE_ICON: f32 = 26.0;
pub(super) const LARGE_ICON_X: f32 = 11.0;
pub(super) const LARGE_ICON_GAP: f32 = 11.0;
/// Sharing's service rows are 50 tall.
pub(super) const SHARING_ROW_HEIGHT: f32 = 50.0;
/// The status dot before "Connected" / "Not connected".
pub(super) const STATUS_DOT: f32 = 8.0;
pub(super) const STATUS_DOT_GAP: f32 = 4.0;
/// Header cards (Notifications, Privacy, Accessibility, Spotlight): 65 tall,
/// the icon 12 from the top.
pub(super) const HEADER_CARD_HEIGHT: f32 = 65.0;
pub(super) const HEADER_ICON_TOP: f32 = 2.0;
/// A section note sits 2 under its head and 10 above the group.
pub(super) const SECTION_NOTE_GAP: f32 = 2.0;
/// Slider rows (Trackpad, Mouse): a 242 pt control from x + 208.
pub(super) const SLIDER_WIDTH: f32 = 242.0;
/// Keyboard's two sliders share a 75 pt group, 210 wide each, 20 apart.
pub(super) const TWIN_SLIDER_WIDTH: f32 = 210.0;
pub(super) const TWIN_SLIDER_GAP: f32 = 20.0;
pub(super) const SLIDER_TRACK: f32 = 6.0;
pub(super) const SLIDER_KNOB_WIDTH: f32 = 20.0;
pub(super) const SLIDER_KNOB_HEIGHT: f32 = 16.0;
/// Trackpad's tab bar: 24 tall, radius 6, the group 18 under it.
pub(super) const TAB_HEIGHT: f32 = 24.0;
pub(super) const TAB_RADIUS: f32 = 6.0;
pub(super) const TAB_GAP_BELOW: f32 = 18.0;
/// Table wells: 28 pt header, 24 pt rows and a 24 pt +/− bar.
pub(super) const WELL_HEADER_HEIGHT: f32 = 28.0;
pub(super) const WELL_ROW_HEIGHT: f32 = 24.0;
pub(super) const WELL_BAR_HEIGHT: f32 = 24.0;
/// Radio circles and the checkboxes in wells and previews.
pub(super) const RADIO: f32 = 16.0;
pub(super) const RADIO_GAP: f32 = 14.0;
/// Storage: the bar is 21 tall, radius 3, with 1 pt gaps between segments.
pub(super) const STORAGE_BAR_HEIGHT: f32 = 21.0;
pub(super) const STORAGE_BAR_RADIUS: f32 = 3.0;
/// Sheets (Keyboard Shortcuts): radius 26, a 200 pt sidebar panel inset 8,
/// content from x 228, a 65 pt footer under a 1 pt rule.
pub(super) const SHEET_RADIUS: f32 = 26.0;
pub(super) const SHEET_SIDEBAR_WIDTH: f32 = 200.0;
pub(super) const SHEET_FOOTER_HEIGHT: f32 = 65.0;

pub(super) fn status_connected() -> Hsla {
    rgb(0x68ce67).into()
}

pub(super) fn status_disconnected() -> Hsla {
    rgb(0xeb534e).into()
}

pub(super) fn status_inactive() -> Hsla {
    rgb(0x5c5d65).into()
}

/// Section notes under a head (Notification Centre, Login Items).
pub(super) fn note_text() -> Hsla {
    if dark() {
        rgb(0x9a9ba0).into()
    } else {
        rmac_ui::mac::text_secondary()
    }
}

/// The accent drawn by switches, sliders and the selected tab.
pub(super) fn control_accent() -> Hsla {
    if dark() {
        rgb(0x397cf7).into()
    } else {
        rmac_ui::mac::accent()
    }
}

pub(super) fn control_off() -> Hsla {
    if dark() {
        rgb(0x3c3d47).into()
    } else {
        rmac_ui::mac::control_fill()
    }
}

pub(super) fn slider_knob() -> Hsla {
    if dark() {
        rgb(0xdfdfe1).into()
    } else {
        gpui::white()
    }
}

pub(super) fn tab_fill() -> Hsla {
    if dark() {
        rgb(0x30313c).into()
    } else {
        rmac_ui::mac::control_fill()
    }
}

/// Storage categories in the Mac's order; System Data is grey.
pub(super) const STORAGE_COLORS: [u32; 6] =
    [0xeb534e, 0xf09748, 0xf8d849, 0x68ce67, 0x63d7c3, 0x5fcfdd];

pub(super) fn storage_system_data() -> Hsla {
    rgb(0x808080).into()
}

pub(super) fn storage_free() -> Hsla {
    if dark() {
        rgb(0x44454e).into()
    } else {
        rmac_ui::mac::control_fill()
    }
}

pub(super) fn storage_gap() -> Hsla {
    if dark() {
        rgb(0x31323c).into()
    } else {
        rmac_ui::mac::separator()
    }
}

pub(super) fn sheet_fill() -> Hsla {
    if dark() {
        rgb(0x1f212c).into()
    } else {
        rmac_ui::mac::raised()
    }
}

pub(super) fn sheet_sidebar() -> Hsla {
    if dark() {
        rgb(0x1b1c23).into()
    } else {
        rmac_ui::mac::material_sidebar()
    }
}

pub(super) fn sheet_edge() -> Hsla {
    if dark() {
        white(0.12)
    } else {
        rmac_ui::mac::separator()
    }
}

/// The Displays pane's arrangement well, darker than the window.
pub(super) fn well_fill() -> Hsla {
    if dark() {
        rgb(0x1c1c24).into()
    } else {
        rmac_ui::mac::control_fill()
    }
}
