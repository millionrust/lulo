//! Convert-mode layout and metrics (CALC-06, View ▸ Convert ⌥⌘C).
//!
//! No Mac capture of Convert mode exists yet (same caveat as
//! `programmer_keypad.rs`). The keypad below the category/unit rows reuses
//! Basic's own measured digit-key metrics (48 pt circles on a 54 pt pitch)
//! so it matches Basic/Scientific's visual language; the extra header
//! above it (category selector, From/To rows) is an original layout.
//! Convert is integer-and-decimal entry only — no arithmetic operators —
//! so the keypad drops Basic's operator column entirely bar a single ⇄
//! (swap) key in its place.

use crate::keypad;

/// Same width as Basic: both are narrow, single-value windows.
pub const WINDOW_WIDTH: f32 = keypad::WINDOW_WIDTH;

/// Header above the keypad: a category row, then From/To value+unit rows.
pub const CATEGORY_ROW_TOP: f32 = mac_toolbar_height();
pub const CATEGORY_ROW_HEIGHT: f32 = 28.0;
pub const FROM_ROW_TOP: f32 = CATEGORY_ROW_TOP + CATEGORY_ROW_HEIGHT + 6.0;
pub const ROW_HEIGHT: f32 = 56.0;
pub const TO_ROW_TOP: f32 = FROM_ROW_TOP + ROW_HEIGHT + 10.0;
pub const HEADER_BOTTOM: f32 = TO_ROW_TOP + ROW_HEIGHT + 10.0;

pub const KEYPAD_LEFT: f32 = keypad::KEYPAD_LEFT;
pub const KEYPAD_TOP: f32 = HEADER_BOTTOM;
pub const KEY_DIAMETER: f32 = keypad::KEY_DIAMETER;
pub const KEY_PITCH: f32 = keypad::KEY_PITCH;
pub const COLUMNS: usize = 4;
pub const ROWS: usize = 5;

pub const WINDOW_HEIGHT: f32 = KEYPAD_TOP + (ROWS - 1) as f32 * KEY_PITCH + KEY_DIAMETER + 12.0;

/// `rmac_ui::mac::toolbar_height()` is not `const`, so it is read once at
/// layout-constant-definition time via a thin wrapper kept in step with
/// `keypad::EXPRESSION_TOP`'s own assumptions (52 pt).
const fn mac_toolbar_height() -> f32 {
    52.0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Digit(u8),
    Decimal,
    Clear,
    Backspace,
    /// ⇄: swap the From/To units.
    Swap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyStyle {
    Function,
    Digit,
    Swap,
}

pub fn key_style(key: Key) -> KeyStyle {
    match key {
        Key::Backspace | Key::Clear => KeyStyle::Function,
        Key::Swap => KeyStyle::Swap,
        Key::Digit(_) | Key::Decimal => KeyStyle::Digit,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyFace {
    Text(&'static str),
    Glyph(&'static str),
}

pub fn key_face(key: Key) -> KeyFace {
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    match key {
        Key::Digit(digit) => KeyFace::Text(DIGITS[usize::from(digit.min(9))]),
        Key::Decimal => KeyFace::Text("."),
        Key::Clear => KeyFace::Text("AC"),
        Key::Backspace => KeyFace::Glyph("icons/calculator/backspace.svg"),
        Key::Swap => KeyFace::Glyph("icons/calculator/swap.svg"),
    }
}

pub fn key_name(key: Key) -> &'static str {
    const DIGIT_NAMES: [&str; 10] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    match key {
        Key::Digit(digit) => DIGIT_NAMES[usize::from(digit.min(9))],
        Key::Decimal => "decimal point",
        Key::Clear => "clear",
        Key::Backspace => "delete",
        Key::Swap => "swap units",
    }
}

/// Rows top to bottom, columns left to right. `None` cells render nothing
/// — see the module doc comment for why the operator column is mostly
/// empty.
pub const LAYOUT: [[Option<Key>; COLUMNS]; ROWS] = [
    [Some(Key::Backspace), Some(Key::Clear), None, None],
    [
        Some(Key::Digit(7)),
        Some(Key::Digit(8)),
        Some(Key::Digit(9)),
        None,
    ],
    [
        Some(Key::Digit(4)),
        Some(Key::Digit(5)),
        Some(Key::Digit(6)),
        None,
    ],
    [
        Some(Key::Digit(1)),
        Some(Key::Digit(2)),
        Some(Key::Digit(3)),
        None,
    ],
    [
        None,
        Some(Key::Digit(0)),
        Some(Key::Decimal),
        Some(Key::Swap),
    ],
];

/// Top-left corner of the key at `row`, `column`.
pub fn key_origin(row: usize, column: usize) -> (f32, f32) {
    (
        KEYPAD_LEFT + column as f32 * KEY_PITCH,
        KEYPAD_TOP + row as f32 * KEY_PITCH,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_filled_cell_has_a_face_and_a_name() {
        for row in LAYOUT.iter() {
            for key in row.iter().flatten() {
                assert!(!key_name(*key).is_empty());
                match key_face(*key) {
                    KeyFace::Text(text) => assert!(!text.is_empty()),
                    KeyFace::Glyph(path) => assert!(path.starts_with("icons/calculator/")),
                }
            }
        }
    }

    #[test]
    fn swap_sits_where_equals_used_to() {
        assert_eq!(LAYOUT[4][3], Some(Key::Swap));
    }

    #[test]
    fn window_reuses_basics_measured_key_metrics() {
        assert_eq!(WINDOW_WIDTH, keypad::WINDOW_WIDTH);
        let (left, _) = key_origin(0, 0);
        assert_eq!(left, KEYPAD_LEFT);
    }
}
