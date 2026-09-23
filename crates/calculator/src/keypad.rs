//! Basic-mode keypad layout, metrics, colours and keyboard mapping.
//!
//! Geometry is measured from a macOS 26 Calculator capture at 2× and given
//! here in points (`target/evidence/mac-2026-09-23/calculator.png`).

use crate::engine::{Key, Operator};

/// Content size of the fixed-size window.
pub const WINDOW_WIDTH: f32 = 230.0;
pub const WINDOW_HEIGHT: f32 = 406.0;

/// Keys are 48 pt circles on a 54 pt pitch (a 6 pt gap).
pub const KEY_DIAMETER: f32 = 48.0;
pub const KEY_PITCH: f32 = 54.0;
/// Left edge of the first key column and top edge of the first key row.
pub const KEYPAD_LEFT: f32 = 10.0;
pub const KEYPAD_TOP: f32 = 132.0;

/// Traffic-light centre of the close button, from the window's top-left.
pub const TRAFFIC_LIGHT_CENTER: (f32, f32) = (26.0, 25.0);
/// Toolbar buttons: 36 pt glass circles centred on these x positions.
pub const TOOLBAR_BUTTON_DIAMETER: f32 = 36.0;
pub const SIDEBAR_BUTTON_CENTER_X: f32 = 124.0;
pub const MODE_BUTTON_CENTER_X: f32 = 204.0;

/// Right inset of the expression and result text.
pub const DISPLAY_RIGHT_INSET: f32 = 11.0;
/// Expression line: 22 pt text whose baseline sits 79.5 pt from the top.
pub const EXPRESSION_TOP: f32 = 57.5;
pub const EXPRESSION_LINE: f32 = 28.0;
pub const EXPRESSION_SIZE: f32 = 22.0;
/// Result line: up to 31 pt light text, baseline 118.5 pt from the top.
pub const RESULT_TOP: f32 = 88.0;
pub const RESULT_LINE: f32 = 38.0;
pub const RESULT_MAX_SIZE: f32 = 31.0;
pub const RESULT_MIN_SIZE: f32 = 15.0;
/// Key label size (digits are 15.5 pt tall in the capture).
pub const LABEL_SIZE: f32 = 22.0;
/// Operator and function glyphs are drawn from 24 pt SVG boxes.
pub const GLYPH_SIZE: f32 = 24.0;

/// Rows top to bottom, columns left to right, as on macOS 26.
pub const LAYOUT: [[Key; 4]; 5] = [
    [
        Key::Backspace,
        Key::Clear,
        Key::Percent,
        Key::Operator(Operator::Divide),
    ],
    [
        Key::Digit(7),
        Key::Digit(8),
        Key::Digit(9),
        Key::Operator(Operator::Multiply),
    ],
    [
        Key::Digit(4),
        Key::Digit(5),
        Key::Digit(6),
        Key::Operator(Operator::Subtract),
    ],
    [
        Key::Digit(1),
        Key::Digit(2),
        Key::Digit(3),
        Key::Operator(Operator::Add),
    ],
    [Key::ToggleSign, Key::Digit(0), Key::Decimal, Key::Equals],
];

/// Top-left corner of the key at `row`, `column`.
pub fn key_origin(row: usize, column: usize) -> (f32, f32) {
    (
        KEYPAD_LEFT + column as f32 * KEY_PITCH,
        KEYPAD_TOP + row as f32 * KEY_PITCH,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyStyle {
    /// Light grey: ⌫, AC/C, %.
    Function,
    /// Dark grey: digits, ±, decimal point.
    Digit,
    /// Orange: ÷ × − + =.
    Operator,
}

pub fn key_style(key: Key) -> KeyStyle {
    match key {
        Key::Backspace | Key::Clear | Key::Percent => KeyStyle::Function,
        Key::Operator(_) | Key::Equals => KeyStyle::Operator,
        Key::Digit(_) | Key::Decimal | Key::ToggleSign => KeyStyle::Digit,
    }
}

/// What a key shows: text, or an SVG glyph path from the app's assets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyFace {
    Text(&'static str),
    Glyph(&'static str),
}

/// The face of every key except the clear key, whose text follows the engine.
pub fn key_face(key: Key) -> KeyFace {
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    match key {
        Key::Digit(digit) => KeyFace::Text(DIGITS[usize::from(digit.min(9))]),
        Key::Decimal => KeyFace::Text("."),
        Key::Percent => KeyFace::Text("%"),
        Key::Clear => KeyFace::Text("AC"),
        Key::Backspace => KeyFace::Glyph("icons/calculator/backspace.svg"),
        Key::ToggleSign => KeyFace::Glyph("icons/calculator/plus-minus.svg"),
        Key::Equals => KeyFace::Glyph("icons/calculator/equals.svg"),
        Key::Operator(Operator::Add) => KeyFace::Glyph("icons/calculator/plus.svg"),
        Key::Operator(Operator::Subtract) => KeyFace::Glyph("icons/calculator/minus.svg"),
        Key::Operator(Operator::Multiply) => KeyFace::Glyph("icons/calculator/multiply.svg"),
        Key::Operator(Operator::Divide) => KeyFace::Glyph("icons/calculator/divide.svg"),
    }
}

/// Accessible name for each key.
pub fn key_name(key: Key) -> &'static str {
    match key {
        Key::Digit(0) => "zero",
        Key::Digit(1) => "one",
        Key::Digit(2) => "two",
        Key::Digit(3) => "three",
        Key::Digit(4) => "four",
        Key::Digit(5) => "five",
        Key::Digit(6) => "six",
        Key::Digit(7) => "seven",
        Key::Digit(8) => "eight",
        Key::Digit(_) => "nine",
        Key::Decimal => "decimal point",
        Key::Percent => "percent",
        Key::Clear => "clear",
        Key::Backspace => "delete",
        Key::ToggleSign => "negate",
        Key::Equals => "equals",
        Key::Operator(Operator::Add) => "add",
        Key::Operator(Operator::Subtract) => "subtract",
        Key::Operator(Operator::Multiply) => "multiply",
        Key::Operator(Operator::Divide) => "divide",
    }
}

/// Map a hardware key press to a keypad key. `key` is GPUI's key name and
/// `key_char` the character it types. Presses with ⌘ or Ctrl never map, so
/// ⌘C and ⌘V reach their menu actions.
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
        '.' | ',' => Key::Decimal,
        '+' => Key::Operator(Operator::Add),
        '-' | '\u{2212}' => Key::Operator(Operator::Subtract),
        '*' | 'x' | 'X' | '×' => Key::Operator(Operator::Multiply),
        '/' | '÷' => Key::Operator(Operator::Divide),
        '=' => Key::Equals,
        '%' => Key::Percent,
        'c' | 'C' => Key::Clear,
        _ => return None,
    })
}

/// Colours as 0xRRGGBB.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub window: u32,
    pub expression: u32,
    pub result: u32,
    pub function_key: u32,
    pub function_label: u32,
    pub digit_key: u32,
    pub digit_label: u32,
    pub operator_key: u32,
    pub operator_label: u32,
    /// The pending operator inverts: a white key with an orange glyph.
    pub operator_selected_key: u32,
    pub operator_selected_label: u32,
    pub toolbar_button: u32,
    pub toolbar_glyph: u32,
    /// Hairline rim on every key and toolbar button, as 0xRRGGBBAA.
    pub rim: u32,
    /// Overlay while a key is pressed, as 0xRRGGBBAA.
    pub pressed_overlay: u32,
}

/// Measured from the dark-mode capture.
pub const DARK: Palette = Palette {
    window: 0x22252D,
    expression: 0x9B9DA0,
    result: 0xDDDEDF,
    function_key: 0x727479,
    function_label: 0xF6F6F6,
    digit_key: 0x46494E,
    digit_label: 0xF6F6F6,
    operator_key: 0xFF9200,
    operator_label: 0xFFFAF2,
    operator_selected_key: 0xFFFAF2,
    operator_selected_label: 0xFF9200,
    toolbar_button: 0x1C1E23,
    toolbar_glyph: 0xE8E8E8,
    rim: 0xFFFFFF1A,
    pressed_overlay: 0xFFFFFF40,
};

/// Light appearance. Not yet measured from a light-mode capture; these follow
/// the dark measurements with the system's light-mode greys.
pub const LIGHT: Palette = Palette {
    window: 0xF2F2F4,
    expression: 0x86868B,
    result: 0x1D1D1F,
    function_key: 0xC9C9CE,
    function_label: 0x1D1D1F,
    digit_key: 0xFFFFFF,
    digit_label: 0x1D1D1F,
    operator_key: 0xFF9200,
    operator_label: 0xFFFFFF,
    operator_selected_key: 0xFFFFFF,
    operator_selected_label: 0xFF9200,
    toolbar_button: 0xE4E4E7,
    toolbar_glyph: 0x3A3A3C,
    rim: 0x0000001A,
    pressed_overlay: 0x00000026,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypad_fills_the_window_with_measured_margins() {
        let (left, top) = key_origin(0, 0);
        let (right, bottom) = key_origin(4, 3);
        assert_eq!((left, top), (10.0, 132.0));
        assert_eq!(WINDOW_WIDTH - (right + KEY_DIAMETER), left);
        assert_eq!(WINDOW_HEIGHT - (bottom + KEY_DIAMETER), 10.0);
        assert_eq!(KEY_PITCH - KEY_DIAMETER, 6.0);
    }

    #[test]
    fn layout_matches_macos_26() {
        assert_eq!(LAYOUT[0][0], Key::Backspace);
        assert_eq!(LAYOUT[0][1], Key::Clear);
        assert_eq!(LAYOUT[4][0], Key::ToggleSign);
        assert_eq!(LAYOUT[4][1], Key::Digit(0));
        assert_eq!(LAYOUT[4][3], Key::Equals);
        let digits = LAYOUT
            .iter()
            .flatten()
            .filter(|key| matches!(key, Key::Digit(_)))
            .count();
        assert_eq!(digits, 10);
        assert!(LAYOUT
            .iter()
            .all(|row| key_style(row[3]) == KeyStyle::Operator));
        assert!(LAYOUT[0][..3]
            .iter()
            .all(|key| key_style(*key) == KeyStyle::Function));
    }

    #[test]
    fn every_key_has_a_face_and_a_name() {
        for key in LAYOUT.iter().flatten() {
            assert!(!key_name(*key).is_empty());
            match key_face(*key) {
                KeyFace::Text(text) => assert!(!text.is_empty()),
                KeyFace::Glyph(path) => assert!(path.starts_with("icons/calculator/")),
            }
        }
        assert_eq!(key_face(Key::Digit(7)), KeyFace::Text("7"));
    }

    #[test]
    fn keyboard_maps_digits_operators_and_editing_keys() {
        assert_eq!(key_for_input("7", Some("7"), false), Some(Key::Digit(7)));
        assert_eq!(
            key_for_input("=", Some("+"), false),
            Some(Key::Operator(Operator::Add))
        );
        assert_eq!(
            key_for_input("-", Some("-"), false),
            Some(Key::Operator(Operator::Subtract))
        );
        assert_eq!(
            key_for_input("8", Some("*"), false),
            Some(Key::Operator(Operator::Multiply))
        );
        assert_eq!(
            key_for_input("/", Some("/"), false),
            Some(Key::Operator(Operator::Divide))
        );
        assert_eq!(key_for_input("=", Some("="), false), Some(Key::Equals));
        assert_eq!(key_for_input("enter", None, false), Some(Key::Equals));
        assert_eq!(key_for_input("escape", None, false), Some(Key::Clear));
        assert_eq!(
            key_for_input("backspace", None, false),
            Some(Key::Backspace)
        );
        assert_eq!(key_for_input("delete", None, false), Some(Key::Backspace));
        assert_eq!(key_for_input("5", Some("%"), false), Some(Key::Percent));
        assert_eq!(key_for_input(".", Some("."), false), Some(Key::Decimal));
        assert_eq!(key_for_input("c", Some("c"), false), Some(Key::Clear));
    }

    #[test]
    fn keyboard_ignores_shortcuts_and_other_keys() {
        assert_eq!(key_for_input("c", Some("c"), true), None);
        assert_eq!(key_for_input("v", None, true), None);
        assert_eq!(key_for_input("a", Some("a"), false), None);
        assert_eq!(key_for_input("left", None, false), None);
        assert_eq!(key_for_input("space", Some(" "), false), None);
    }

    #[test]
    fn dark_palette_uses_the_measured_colours() {
        assert_eq!(DARK.operator_key, 0xFF9200);
        assert_eq!(DARK.digit_key, 0x46494E);
        assert_eq!(DARK.function_key, 0x727479);
        assert_eq!(LIGHT.operator_key, DARK.operator_key);
    }
}
