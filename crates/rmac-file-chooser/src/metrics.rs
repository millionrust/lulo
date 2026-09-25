//! Measured geometry and colours of the macOS 26 Open/Save panel
//! (design-lab/file-chooser.html). Points; colours are dark-mode sRGB taken
//! from Retina captures of TextEdit's ⌘O/⌘S on the owner's Mac. Light mode
//! has not been measured and falls back to the shared rmac tokens.

/// Open panel / expanded Save panel.
pub const PANEL_WIDTH: f32 = 880.0;
pub const PANEL_HEIGHT: f32 = 448.0;
/// Compact Save sheet width.
pub const COMPACT_WIDTH: f32 = 390.0;
/// niri clips every window at 16 (the Open panel's radius); the Save window
/// uses its own app id so the session config can clip it at 24.
pub const OPEN_RADIUS: f32 = 16.0;
pub const SAVE_RADIUS: f32 = 24.0;

pub const SIDEBAR_INSET: f32 = 8.0;
pub const SIDEBAR_WIDTH: f32 = 119.0;
/// Right edge of the sidebar pane; content starts here.
pub const CONTENT_LEFT: f32 = 127.0;
pub const SIDEBAR_RADIUS: f32 = 8.0;
pub const SIDEBAR_FIRST_ROW: f32 = 10.0;
pub const SIDEBAR_ROW_HEIGHT: f32 = 32.0;
pub const SIDEBAR_PILL_INSET: f32 = 10.0;
pub const SIDEBAR_PILL_WIDTH: f32 = 99.0;
pub const SIDEBAR_PILL_RADIUS: f32 = 7.0;
pub const SIDEBAR_ICON: f32 = 18.0;
pub const SIDEBAR_ICON_LEFT: f32 = 10.0;
pub const SIDEBAR_LABEL_LEFT: f32 = 35.0;
pub const SIDEBAR_HEADER_GAP: f32 = 13.0;
pub const SIDEBAR_HEADER_HEIGHT: f32 = 19.0;
pub const SIDEBAR_HEADER_LEFT: f32 = 15.0;

pub const CONTROL_HEIGHT: f32 = 26.0;
/// Controls draw a 24 pt plate inset 1 pt inside their 26 pt frame.
pub const PLATE_INSET: f32 = 1.0;
pub const PLATE_RADIUS: f32 = 6.0;
pub const TOOLBAR_TOP: f32 = 19.0;
pub const NAV_X: f32 = 146.0;
pub const NAV_WIDTH: f32 = 49.0;
pub const VIEW_X: f32 = 201.0;
pub const VIEW_WIDTH: f32 = 69.0;
pub const SORT_X: f32 = 276.0;
pub const SORT_WIDTH: f32 = 66.0;
pub const WHERE_X: f32 = 387.0;
pub const WHERE_WIDTH_OPEN: f32 = 232.0;
pub const WHERE_WIDTH_SAVE: f32 = 200.0;
pub const DISCLOSURE_X: f32 = 593.0;
pub const DISCLOSURE_WIDTH: f32 = 26.0;
pub const SEARCH_X: f32 = 664.0;
pub const SEARCH_WIDTH: f32 = 196.0;
/// Hairline below the toolbar row, from the row's top.
pub const TOOLBAR_TO_HAIRLINE: f32 = 45.5;
/// Hairline above the bottom row, from the panel's bottom.
pub const BOTTOM_HAIRLINE: f32 = 65.0;
/// Bottom-row buttons, from the panel's bottom.
pub const BOTTOM_ROW: f32 = 45.0;
pub const BUTTON_WIDTH: f32 = 76.0;
pub const CANCEL_X: f32 = 703.0;
pub const DEFAULT_X: f32 = 785.0;
pub const NEW_FOLDER_WIDTH: f32 = 95.0;

pub const ICON_SIZE: f32 = 64.0;
pub const ICON_FIRST_X: f32 = 38.0;
pub const ICON_FIRST_Y: f32 = 24.0;
pub const ICON_PITCH_X: f32 = 122.5;
pub const ICON_PITCH_Y: f32 = 130.0;
pub const ICON_LABEL_WIDTH: f32 = 112.0;
pub const ICON_LABEL_GAP: f32 = 6.0;
pub const ICON_LABEL_LINE: f32 = 16.0;
/// List view rows follow Files (not captured in the panel).
pub const LIST_ROW_HEIGHT: f32 = 24.0;

/// Save header rows: pitch 36 from y 19; label column and field column.
pub const HEADER_FIRST_ROW: f32 = 19.0;
pub const HEADER_ROW_PITCH: f32 = 36.0;
pub const LABEL_WIDTH: f32 = 73.0;
pub const COMPACT_LABEL_X: f32 = 39.0;
pub const COMPACT_FIELD_X: f32 = 118.0;
pub const EXPANDED_LABEL_X: f32 = 308.0;
pub const FIELD_WIDTH: f32 = 232.0;
pub const COMPACT_DISCLOSURE_X: f32 = 324.0;
/// Compact: buttons sit 45 below the Where row; 19 below them.
pub const COMPACT_BUTTONS_BELOW_WHERE: f32 = 45.0;
pub const COMPACT_BOTTOM_MARGIN: f32 = 19.0;
pub const COMPACT_CANCEL_X: f32 = 212.0;
pub const COMPACT_SAVE_X: f32 = 294.0;
/// Expanded: toolbar row 44 below the last header row.
pub const EXPANDED_TOOLBAR_BELOW_HEADER: f32 = 44.0;
/// The Mac's expanded sheet is 448 tall with three header rows (Save As,
/// Tags, File Format); each omitted row removes one pitch.
pub const EXPANDED_HEIGHT_THREE_ROWS: f32 = 448.0;

/// The "already exists, do you want to replace it?" alert: a 260 pt NSAlert
/// sheet with two sentences (title + explanation) and 112×30 buttons —
/// larger than the panel's own 76×26 controls (OTHER-11).
pub const REPLACE_WIDTH: f32 = 260.0;
pub const REPLACE_PADDING: f32 = 20.0;
pub const REPLACE_BUTTON_WIDTH: f32 = 112.0;
pub const REPLACE_BUTTON_HEIGHT: f32 = 30.0;
/// Generous headroom for the two-line title and up to three lines of body
/// text at `REPLACE_WIDTH`; the card does not grow past this.
pub const REPLACE_HEIGHT: f32 = 172.0;

/// The New Folder sheet: "New Folder" / "Name of new folder inside
/// “<folder>”:" / a field defaulted to "untitled folder" / Cancel · Create,
/// 320×158 on the Mac (OTHER-10).
pub const NEW_FOLDER_SHEET_WIDTH: f32 = 320.0;
pub const NEW_FOLDER_SHEET_HEIGHT: f32 = 158.0;
pub const NEW_FOLDER_SHEET_PADDING: f32 = 20.0;

pub const GOTO_WIDTH: f32 = 460.0;
pub const GOTO_HEIGHT: f32 = 183.0;
pub const GOTO_RADIUS: f32 = 24.0;
pub const GOTO_FIELD_X: f32 = 15.0;
pub const GOTO_FIELD_Y: f32 = 7.0;
pub const GOTO_FIELD_WIDTH: f32 = 344.0;
pub const GOTO_FIELD_HEIGHT: f32 = 22.0;
pub const GOTO_HEAD_HEIGHT: f32 = 35.0;
pub const GOTO_CLOSE_X: f32 = 427.0;
pub const GOTO_CLOSE_Y: f32 = 9.0;
pub const GOTO_CLOSE_SIZE: f32 = 18.0;

/// Dark palette, sRGB hex.
pub mod dark {
    pub const PANEL: u32 = 0x21212E;
    pub const SHEET: u32 = 0x22212E;
    pub const RIM: u32 = 0x4D4D58;
    pub const SIDEBAR: u32 = 0x21212A;
    pub const SIDEBAR_RIM: u32 = 0x3F3F51;
    pub const PILL: u32 = 0x303038;
    pub const HAIRLINE: u32 = 0x373742;
    pub const CONTROL: u32 = 0x31313E;
    pub const TEXT: u32 = 0xDFDFE1;
    pub const TEXT_SIDEBAR: u32 = 0xF3F3FB;
    pub const TEXT_FILE: u32 = 0xDDDDDF;
    pub const LABEL: u32 = 0x9B9BA1;
    pub const SECTION: u32 = 0x9E9EA6;
    pub const SIDEBAR_SELECTED: u32 = 0x1892FF;
    pub const GLYPH_DISABLED: u32 = 0x64646E;
    pub const GLYPH: u32 = 0xE0E0E2;
    pub const SEARCH_RIM: u32 = 0x2A2A37;
    pub const DEFAULT: u32 = 0x3478F6;
    pub const DEFAULT_DISABLED: u32 = 0x292936;
    pub const TEXT_DISABLED: u32 = 0x5E5E68;
    pub const FIELD_RIM: u32 = 0x2B2A37;
    pub const FOCUS_RING: u32 = 0x3E6998;
    pub const GOTO_HEAD: u32 = 0x22222E;
    pub const GOTO_LIST: u32 = 0x262632;
    pub const GOTO_HAIRLINE: u32 = 0x383842;
    pub const GOTO_PLACEHOLDER: u32 = 0x595962;
    pub const GOTO_CLOSE: u32 = 0x9C9CA1;
}

/// Compact Save height for `rows` header rows (Save As [+ Format] + Where).
pub fn compact_height(rows: usize) -> f32 {
    let where_row = HEADER_FIRST_ROW + HEADER_ROW_PITCH * (rows.saturating_sub(1)) as f32;
    where_row + COMPACT_BUTTONS_BELOW_WHERE + CONTROL_HEIGHT + COMPACT_BOTTOM_MARGIN
}

/// Expanded Save height for `rows` header rows (Save As [+ Format]).
pub fn expanded_height(rows: usize) -> f32 {
    EXPANDED_HEIGHT_THREE_ROWS - HEADER_ROW_PITCH * (3usize.saturating_sub(rows)) as f32
}

/// Toolbar row top in the expanded Save panel.
pub fn expanded_toolbar_top(rows: usize) -> f32 {
    HEADER_FIRST_ROW
        + HEADER_ROW_PITCH * (rows.saturating_sub(1)) as f32
        + EXPANDED_TOOLBAR_BELOW_HEADER
}
