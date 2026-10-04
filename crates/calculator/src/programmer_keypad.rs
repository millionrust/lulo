//! Programmer-mode keypad layout, metrics and keyboard mapping (CALC-06).
//!
//! No Mac capture of Programmer mode exists yet (see `programmer.rs`'s
//! module doc comment), so none of this geometry is pixel-measured. It is
//! an original design that reuses Basic's own measured key metrics (48 pt
//! circles, Scientific's 60×48 stadium pills for the wider function
//! columns) so Programmer fits the same visual language as the other two
//! modes rather than inventing a third one.
//!
//! Eight columns × five rows: the left two columns are the hex digits A–F
//! (greyed out in any base narrower than hex), the next two are the bitwise
//! operators, and the rightmost four columns are exactly Basic's own
//! digit/operator block (`crate::keypad::LAYOUT`), minus the decimal point
//! (Programmer is integer-only) and percent (not a Programmer concept).
//! Above the keypad, a short base/word-size strip replaces the space a
//! third display line would otherwise take.

use crate::keypad;
use crate::programmer::{Base, Key, Operator};

/// Content size of the fixed-size window. Width follows from eight columns
/// at Scientific's 66 pt pitch; height adds a 24 pt base/word-size strip to
/// Basic's own measured height.
pub const WINDOW_WIDTH: f32 = 542.0;
pub const WINDOW_HEIGHT: f32 = keypad::WINDOW_HEIGHT + BASE_STRIP_HEIGHT;

pub const BASE_STRIP_HEIGHT: f32 = 24.0;
pub const BASE_STRIP_TOP: f32 = keypad::KEYPAD_TOP - 6.0;

/// Keys reuse Scientific's 60×48 stadium pills on a 66×54 pt pitch (6 pt
/// gap), matching Basic/Scientific's own gap ratio.
pub const KEY_WIDTH: f32 = 60.0;
pub const KEY_HEIGHT: f32 = 48.0;
pub const KEY_PITCH_X: f32 = 66.0;
pub const KEY_PITCH_Y: f32 = keypad::KEY_PITCH;
pub const KEYPAD_LEFT: f32 = keypad::KEYPAD_LEFT;
pub const KEYPAD_TOP: f32 = keypad::KEYPAD_TOP + BASE_STRIP_HEIGHT;
pub const COLUMNS: usize = 8;
pub const ROWS: usize = 5;

pub const TOOLBAR_BUTTON_DIAMETER: f32 = keypad::TOOLBAR_BUTTON_DIAMETER;
pub const SIDEBAR_BUTTON_CENTER_X: f32 = keypad::SIDEBAR_BUTTON_CENTER_X;
pub const MODE_BUTTON_CENTER_X: f32 = WINDOW_WIDTH - 26.0;

pub const DISPLAY_RIGHT_INSET: f32 = keypad::DISPLAY_RIGHT_INSET;
pub const EXPRESSION_TOP: f32 = keypad::EXPRESSION_TOP;
pub const EXPRESSION_LINE: f32 = keypad::EXPRESSION_LINE;
pub const RESULT_TOP: f32 = keypad::RESULT_TOP;
pub const RESULT_LINE: f32 = keypad::RESULT_LINE;
pub const RESULT_MAX_SIZE: f32 = keypad::RESULT_MAX_SIZE;
pub const RESULT_MIN_SIZE: f32 = 13.0;
pub const LABEL_SIZE: f32 = keypad::LABEL_SIZE;
pub const FUNCTION_LABEL_SIZE: f32 = 14.0;
pub const GLYPH_SIZE: f32 = keypad::GLYPH_SIZE;

/// `None` cells render nothing (no key, no hit target) — see the module
/// doc comment for why the grid isn't fully packed.
pub const LAYOUT: [[Option<Key>; COLUMNS]; ROWS] = [
    [
        Some(Key::Digit(10)), // A
        Some(Key::Digit(11)), // B
        Some(Key::Operator(Operator::And)),
        Some(Key::Operator(Operator::Or)),
        Some(Key::Backspace),
        Some(Key::Clear),
        None,
        Some(Key::Operator(Operator::Divide)),
    ],
    [
        Some(Key::Digit(12)), // C
        Some(Key::Digit(13)), // D
        Some(Key::Operator(Operator::Xor)),
        Some(Key::Not),
        Some(Key::Digit(7)),
        Some(Key::Digit(8)),
        Some(Key::Digit(9)),
        Some(Key::Operator(Operator::Multiply)),
    ],
    [
        Some(Key::Digit(14)), // E
        Some(Key::Digit(15)), // F
        Some(Key::Operator(Operator::ShiftLeft)),
        Some(Key::Operator(Operator::ShiftRight)),
        Some(Key::Digit(4)),
        Some(Key::Digit(5)),
        Some(Key::Digit(6)),
        Some(Key::Operator(Operator::Subtract)),
    ],
    [
        None,
        None,
        None,
        None,
        Some(Key::Digit(1)),
        Some(Key::Digit(2)),
        Some(Key::Digit(3)),
        Some(Key::Operator(Operator::Add)),
    ],
    [
        None,
        None,
        None,
        None,
        Some(Key::ToggleSign),
        Some(Key::Digit(0)),
        None,
        Some(Key::Equals),
    ],
];

/// Top-left corner of the key at `row`, `column`.
pub fn key_origin(row: usize, column: usize) -> (f32, f32) {
    (
        KEYPAD_LEFT + column as f32 * KEY_PITCH_X,
        KEYPAD_TOP + row as f32 * KEY_PITCH_Y,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyStyle {
    Function,
    Digit,
    Hex,
    Bitwise,
    Operator,
}

pub fn key_style(key: Key) -> KeyStyle {
    match key {
        Key::Backspace | Key::Clear => KeyStyle::Function,
        Key::Operator(
            Operator::And
            | Operator::Or
            | Operator::Xor
            | Operator::ShiftLeft
            | Operator::ShiftRight,
        )
        | Key::Not => KeyStyle::Bitwise,
        Key::Operator(_) | Key::Equals => KeyStyle::Operator,
        Key::Digit(digit) if digit >= 10 => KeyStyle::Hex,
        Key::Digit(_) | Key::ToggleSign => KeyStyle::Digit,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyFace {
    Text(&'static str),
    Glyph(&'static str),
}

pub fn key_face(key: Key) -> KeyFace {
    const DIGITS: [&str; 16] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D", "E", "F",
    ];
    match key {
        Key::Digit(digit) => KeyFace::Text(DIGITS[usize::from(digit.min(15))]),
        Key::Not => KeyFace::Text("NOT"),
        Key::Clear => KeyFace::Text("AC"),
        Key::Backspace => KeyFace::Glyph("icons/calculator/backspace.svg"),
        Key::ToggleSign => KeyFace::Glyph("icons/calculator/plus-minus.svg"),
        Key::Equals => KeyFace::Glyph("icons/calculator/equals.svg"),
        Key::Operator(operator) => KeyFace::Text(operator.symbol()),
    }
}

pub fn key_name(key: Key) -> &'static str {
    const DIGIT_NAMES: [&str; 16] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "A", "B",
        "C", "D", "E", "F",
    ];
    match key {
        Key::Digit(digit) => DIGIT_NAMES[usize::from(digit.min(15))],
        Key::Not => "not",
        Key::Clear => "clear",
        Key::Backspace => "delete",
        Key::ToggleSign => "negate",
        Key::Equals => "equals",
        Key::Operator(Operator::Add) => "add",
        Key::Operator(Operator::Subtract) => "subtract",
        Key::Operator(Operator::Multiply) => "multiply",
        Key::Operator(Operator::Divide) => "divide",
        Key::Operator(Operator::And) => "and",
        Key::Operator(Operator::Or) => "or",
        Key::Operator(Operator::Xor) => "xor",
        Key::Operator(Operator::ShiftLeft) => "shift left",
        Key::Operator(Operator::ShiftRight) => "shift right",
    }
}

/// Whether `key` is usable in `base` — the hex digits A–F grey out and stop
/// responding to clicks/keystrokes outside hexadecimal, like the Mac.
pub fn key_enabled(key: Key, base: Base) -> bool {
    match key {
        Key::Digit(digit) => base.allows(digit),
        _ => true,
    }
}

/// Map a hardware key press to a Programmer key. Letters a-f map to the hex
/// digits; shortcuts (⌘ held) never map here, same convention as
/// `keypad::key_for_input`.
pub fn key_for_input(key: &str, key_char: Option<&str>, command_or_control: bool) -> Option<Key> {
    if command_or_control {
        return None;
    }
    match key {
        "enter" | "kp_enter" => return Some(Key::Equals),
        "escape" => return Some(Key::Clear),
        "backspace" | "delete" => return Some(Key::Backspace),
        _ => {}
    }
    let typed = key_char.unwrap_or(key);
    let mut characters = typed.chars();
    let (Some(character), None) = (characters.next(), characters.next()) else {
        return None;
    };
    Some(match character {
        '0'..='9' => Key::Digit(character as u8 - b'0'),
        'a'..='f' => Key::Digit(character as u8 - b'a' + 10),
        'A'..='F' => Key::Digit(character as u8 - b'A' + 10),
        '+' => Key::Operator(Operator::Add),
        '-' | '\u{2212}' => Key::Operator(Operator::Subtract),
        '*' | 'x' | 'X' | '×' => Key::Operator(Operator::Multiply),
        '/' | '÷' => Key::Operator(Operator::Divide),
        '&' => Key::Operator(Operator::And),
        '|' => Key::Operator(Operator::Or),
        '^' => Key::Operator(Operator::Xor),
        '~' => Key::Not,
        '=' => Key::Equals,
        _ => return None,
    })
}

/// The four selectable bases, in the Mac's menu order.
pub const BASES: [Base; 4] = [Base::Hex, Base::Dec, Base::Oct, Base::Bin];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_digits_sit_in_the_left_two_columns() {
        for (row, cells) in LAYOUT.iter().take(3).enumerate() {
            assert_eq!(
                key_style(cells[0].unwrap()),
                KeyStyle::Hex,
                "row {row} col 0"
            );
            assert_eq!(
                key_style(cells[1].unwrap()),
                KeyStyle::Hex,
                "row {row} col 1"
            );
        }
    }

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
    fn hex_digits_disable_outside_hex_base() {
        let a = Key::Digit(10);
        assert!(key_enabled(a, Base::Hex));
        assert!(!key_enabled(a, Base::Dec));
        assert!(!key_enabled(a, Base::Oct));
        assert!(!key_enabled(a, Base::Bin));
        assert!(key_enabled(Key::Digit(0), Base::Bin));
        assert!(!key_enabled(Key::Digit(2), Base::Bin));
    }

    #[test]
    fn keyboard_maps_hex_letters_and_bitwise_symbols() {
        assert_eq!(key_for_input("a", Some("a"), false), Some(Key::Digit(10)));
        assert_eq!(key_for_input("f", Some("F"), false), Some(Key::Digit(15)));
        assert_eq!(
            key_for_input("&", Some("&"), false),
            Some(Key::Operator(Operator::And))
        );
        assert_eq!(
            key_for_input("|", Some("|"), false),
            Some(Key::Operator(Operator::Or))
        );
        assert_eq!(key_for_input("~", Some("~"), false), Some(Key::Not));
    }

    #[test]
    fn window_fits_eight_columns_at_the_measured_pitch() {
        let (left, _) = key_origin(0, 0);
        let (right, _) = key_origin(0, COLUMNS - 1);
        assert_eq!(left, KEYPAD_LEFT);
        assert_eq!(WINDOW_WIDTH - (right + KEY_WIDTH), KEYPAD_LEFT);
    }
}
