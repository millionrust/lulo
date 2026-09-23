//! Measured macOS 26 Clock geometry and colours (dark appearance), in
//! points from the window's top-left. Source: AX frames of all four tabs
//! and Retina pixels ÷ 2, 2026-09-23; see design-lab/apps-clock-weather-player.html.
//! `S` marks values drawn from knowledge rather than measured.

pub const WINDOW: (f32, f32) = (1024.0, 768.0);
/// S: the Mac window can be resized; this floor keeps the tab capsule and
/// the stopwatch columns clear of each other.
pub const MIN_WINDOW: (f32, f32) = (640.0, 480.0);
pub const TOOLBAR_HEIGHT: f32 = 52.0;
pub const TRAFFIC_LIGHT_CENTER: (f32, f32) = (26.0, 26.0);

pub const WINDOW_FILL: u32 = 0x1E1E1E;

/// Tab capsule: 396 × 38 at y 7, centred; segments 98 then 97 wide.
pub const TABS_WIDTH: f32 = 396.0;
pub const TABS_HEIGHT: f32 = 38.0;
pub const TABS_TOP: f32 = 7.0;
pub const TAB_WIDTHS: [f32; 4] = [98.0, 97.0, 97.0, 97.0];
/// Segment x offsets inside the capsule (315, 414, 513, 611 − 314).
pub const TAB_LEFTS: [f32; 4] = [1.0, 100.0, 199.0, 297.0];
pub const TAB_HEIGHT: f32 = 36.0;
pub const TAB_LABEL_SIZE: f32 = 14.0;
pub const CAPSULE_FILL: u32 = 0x272727;
pub const CAPSULE_RIM: u32 = 0x464646;
pub const TAB_SELECTED: u32 = 0x3E3E3E;

/// Add button: 38 pt circle 7 from the right edge (979 of 1024), y 7.
pub const ADD_DIAMETER: f32 = 38.0;
pub const ADD_RIGHT: f32 = 7.0;
pub const ADD_GLYPH: f32 = 13.5;

/// World Clock.
pub const PIN_DIAMETER: f32 = 7.0;
pub const PIN_FILL: u32 = 0xF09748;
pub const PIN_NAME_SIZE: f32 = 13.0;
pub const PIN_TIME_SIZE: f32 = 10.0;
pub const CARD_LEFT: f32 = 15.0;
pub const CARD_GAP_BELOW_MAP: f32 = 15.0;
pub const CARD_WIDTH: f32 = 186.5;
/// S: the Mac's card ran past the window's bottom edge.
pub const CARD_HEIGHT: f32 = 236.0;
/// S.
pub const CARD_GAP: f32 = 15.0;
/// S.
pub const CARD_RADIUS: f32 = 16.0;
pub const CARD_FILL: u32 = 0x323232;
pub const FACE_DIAMETER: f32 = 120.0;
pub const FACE_TOP: f32 = 20.0;
/// S: numerals measured 13.5 × 10 for "12".
pub const NUMERAL_SIZE: f32 = 13.0;
pub const CARD_LINES_TOP: f32 = 157.0;
pub const CARD_LINE: f32 = 16.0;
pub const CARD_TEXT_SIZE: f32 = 13.0;
pub const CARD_SECONDARY: u32 = 0xB9B9B9;
pub const SECOND_HAND: u32 = 0xF09748;

/// Alarms empty state.
pub const EMPTY_GLYPH: f32 = 53.0;
pub const EMPTY_GLYPH_TOP: f32 = 339.5;
pub const EMPTY_GLYPH_FILL: u32 = 0x9A9A9A;
pub const EMPTY_LABEL_TOP: f32 = 414.0;
pub const EMPTY_LABEL: u32 = 0xDDDDDD;
/// Alarm rows (S).
pub const ALARM_ROW_HEIGHT: f32 = 84.0;
pub const ALARM_ROW_INSET: f32 = 15.0;
pub const ALARM_TIME_SIZE: f32 = 48.0;
pub const ALARM_SEPARATOR: u32 = 0x3A3A3A;
pub const SECONDARY_TEXT: u32 = 0x9A9A9A;

/// Stopwatch and Timers digits: 101 pt thin, 73.5 pt digit height.
pub const DIGITS_SIZE: f32 = 101.0;
pub const DIGITS_LINE: f32 = 124.0;
pub const STOPWATCH_DIGITS_TOP: f32 = 61.0;
pub const TIMER_DIGITS_TOP: f32 = 249.0;
/// Lap table: 320 wide centred (352–672 on 1024).
pub const LAP_TABLE_WIDTH: f32 = 320.0;
pub const LAP_HEADER_TOP: f32 = 205.5;
pub const LAP_HEADER_SIZE: f32 = 11.0;
pub const LAP_HEADER: u32 = 0x565656;
pub const LAP_RULE_TOP: f32 = 225.0;
pub const LAP_RULE: u32 = 0x484848;
/// S.
pub const LAP_ROW_HEIGHT: f32 = 28.0;
pub const LAP_TEXT_SIZE: f32 = 13.0;
/// Split column centred at 511.5 − 352, 120 wide.
pub const SPLIT_COLUMN_LEFT: f32 = 100.0;
pub const SPLIT_COLUMN_WIDTH: f32 = 120.0;
pub const LAP_FASTEST: u32 = 0x68CE67;
pub const LAP_SLOWEST: u32 = 0xEB5545;

/// Buttons: 150 × 28 capsules, 20 apart, centred.
pub const BUTTON_WIDTH: f32 = 150.0;
pub const BUTTON_HEIGHT: f32 = 28.0;
pub const BUTTON_GAP: f32 = 20.0;
pub const BUTTON_TEXT_SIZE: f32 = 13.0;
/// Stopwatch buttons sit 50 above the bottom (690 of 768).
pub const STOPWATCH_BUTTONS_BOTTOM: f32 = 50.0;
/// Timer buttons at y 581.
pub const TIMER_BUTTONS_TOP: f32 = 581.0;
pub const BUTTON_DISABLED_FILL: u32 = 0x262626;
pub const BUTTON_DISABLED_TEXT: u32 = 0x5C5C5C;
/// S: enabled grey button.
pub const BUTTON_FILL: u32 = 0x3A3A3A;
pub const START_FILL: u32 = 0x68CE67;
/// S.
pub const STOP_FILL: u32 = 0xEB5545;
/// S.
pub const PAUSE_FILL: u32 = 0xF09A37;

/// Timer entry labels "hr" "min" "sec", centred on these x at y 231.
pub const TIMER_LABEL_CENTERS: [f32; 3] = [374.0, 512.0, 650.5];
pub const TIMER_LABEL_TOP: f32 = 231.0;
pub const TIMER_LABEL_SIZE: f32 = 13.0;
/// Running timer ring (S).
pub const RING_DIAMETER: f32 = 320.0;
pub const RING_WIDTH: f32 = 8.0;
pub const RING_TRACK: u32 = 0x3A3A3A;
pub const RING_FILL: u32 = 0xF09A37;
pub const RING_DIGITS_SIZE: f32 = 64.0;
