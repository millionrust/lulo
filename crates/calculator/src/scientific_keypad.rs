//! Scientific-mode keypad layout, metrics and keyboard mapping (CALC-02).
//!
//! **None of this geometry is measured.** The audit
//! (`docs/parity-audit-2026-09-24-apps.md`, "5. Calculator") only measured
//! Basic mode (230×408 on the Mac); it never opened Scientific, so there is
//! no capture to read pixels from here. Every constant below is an estimate,
//! marked `// S`, built from Basic's *measured* 48 pt / 54 pt circular-key
//! grid (`crate::keypad`) plus the standard nine-column, five-row macOS
//! Scientific key set (2nd, x²/x³/xʸ, eˣ/10ˣ, trig, memory, parentheses — the
//! same keys `crates/calculator/src/scientific.rs` implements).
//!
//! The estimate keeps Basic's 5-row rhythm and 6 pt key gap, but a uniform
//! grid wide enough for nine columns within the requested ~330–350 pt width
//! cannot also keep Basic's 54 pt row pitch inside ~406–408 pt of height —
//! nine columns of 54 pt keys would be 524 pt wide. This mock instead shrinks
//! every key uniformly (30 pt circles on a 36 pt pitch, the same 6 pt gap
//! ratio as Basic) so the grid stays square and legible; the trade-off is a
//! shorter window (316 pt) than the "same height" suggestion in the design
//! brief. `design-lab/calculator.html` shows the reasoning next to the mock.
use crate::keypad::{self, KeyStyle, Palette};
use crate::scientific::{AngleMode, BinaryOp, Key, UnaryFn};

/// Content size of the fixed-size window. Both dimensions are S.
pub const WINDOW_WIDTH: f32 = 338.0; // S
pub const WINDOW_HEIGHT: f32 = 316.0; // S

/// Keys are 30 pt circles on a 36 pt pitch (a 6 pt gap, the same ratio as
/// Basic's 48 pt / 54 pt grid). S.
pub const KEY_DIAMETER: f32 = 30.0; // S
pub const KEY_PITCH: f32 = 36.0; // S
/// Left edge of the first key column and top edge of the first key row.
/// Reuses Basic's measured margins unchanged. S (applied to a new grid).
pub const KEYPAD_LEFT: f32 = keypad::KEYPAD_LEFT;
pub const KEYPAD_TOP: f32 = keypad::KEYPAD_TOP;

/// Toolbar buttons keep Basic's measured 36 pt diameter and the traffic
/// lights' measured position; only their x position is new (S), keeping
/// Basic's measured 80 pt gap between the two buttons and its 26 pt margin
/// from the mode button to the window's trailing edge.
pub const TOOLBAR_BUTTON_DIAMETER: f32 = keypad::TOOLBAR_BUTTON_DIAMETER;
pub const MODE_BUTTON_CENTER_X: f32 = WINDOW_WIDTH - 26.0; // S
pub const SIDEBAR_BUTTON_CENTER_X: f32 = MODE_BUTTON_CENTER_X - 80.0; // S

/// The display geometry (insets, line positions, font sizes) is shared with
/// Basic mode: same toolbar height, same two-line layout, just a wider box
/// to right-align text into.
pub const DISPLAY_RIGHT_INSET: f32 = keypad::DISPLAY_RIGHT_INSET;
pub const EXPRESSION_TOP: f32 = keypad::EXPRESSION_TOP;
pub const EXPRESSION_LINE: f32 = keypad::EXPRESSION_LINE;
pub const EXPRESSION_SIZE: f32 = keypad::EXPRESSION_SIZE;
pub const RESULT_TOP: f32 = keypad::RESULT_TOP;
pub const RESULT_LINE: f32 = keypad::RESULT_LINE;
pub const RESULT_MAX_SIZE: f32 = keypad::RESULT_MAX_SIZE;
pub const RESULT_MIN_SIZE: f32 = keypad::RESULT_MIN_SIZE;

/// Function-key labels are shorter than Basic's digits, so they run smaller.
pub const LABEL_SIZE: f32 = 15.0; // S
/// Digits and the four basic operators keep Basic's label size.
pub const DIGIT_LABEL_SIZE: f32 = keypad::LABEL_SIZE;
pub const GLYPH_SIZE: f32 = keypad::GLYPH_SIZE;

/// Nine columns (five function columns, then Basic's four digit/operator
/// columns), five rows. `None` cells are unused — the function set (22 keys)
/// does not fill all 25 function-column slots. S throughout: this is the
/// estimated key placement, not a measured grid.
pub const LAYOUT: [[Option<Key>; 9]; 5] = [
    [
        Some(Key::MemoryClear),
        Some(Key::MemoryAdd),
        Some(Key::MemorySubtract),
        Some(Key::MemoryRecall),
        Some(Key::OpenParen),
        Some(Key::Backspace),
        Some(Key::Clear),
        Some(Key::Percent),
        Some(Key::Operator(BinaryOp::Divide)),
    ],
    [
        Some(Key::Second),
        Some(Key::Unary(UnaryFn::SquareOrRoot)),
        Some(Key::Unary(UnaryFn::CubeOrCbrt)),
        Some(Key::PowerOrRoot),
        Some(Key::Unary(UnaryFn::ExpOrLn)),
        Some(Key::Digit(7)),
        Some(Key::Digit(8)),
        Some(Key::Digit(9)),
        Some(Key::Operator(BinaryOp::Multiply)),
    ],
    [
        Some(Key::Unary(UnaryFn::TenPowOrLog10)),
        Some(Key::Unary(UnaryFn::Sin)),
        Some(Key::Unary(UnaryFn::Cos)),
        Some(Key::Unary(UnaryFn::Tan)),
        Some(Key::Unary(UnaryFn::Reciprocal)),
        Some(Key::Digit(4)),
        Some(Key::Digit(5)),
        Some(Key::Digit(6)),
        Some(Key::Operator(BinaryOp::Subtract)),
    ],
    [
        Some(Key::Unary(UnaryFn::Factorial)),
        Some(Key::Pi),
        Some(Key::E),
        Some(Key::Rand),
        Some(Key::Ee),
        Some(Key::Digit(1)),
        Some(Key::Digit(2)),
        Some(Key::Digit(3)),
        Some(Key::Operator(BinaryOp::Add)),
    ],
    [
        Some(Key::RadDeg),
        Some(Key::CloseParen),
        None,
        None,
        None,
        Some(Key::ToggleSign),
        Some(Key::Digit(0)),
        Some(Key::Decimal),
        Some(Key::Equals),
    ],
];

/// Top-left corner of the key at `row`, `column`.
pub fn key_origin(row: usize, column: usize) -> (f32, f32) {
    (
        KEYPAD_LEFT + column as f32 * KEY_PITCH,
        KEYPAD_TOP + row as f32 * KEY_PITCH,
    )
}

pub fn key_style(key: Key) -> KeyStyle {
    match key {
        Key::Digit(_) | Key::Decimal | Key::ToggleSign => KeyStyle::Digit,
        Key::Operator(_) | Key::Equals => KeyStyle::Operator,
        _ => KeyStyle::Function,
    }
}

/// What a key shows: text, or an SVG glyph path shared with Basic mode.
/// `second` and `angle` pick the alternate label for the keys `2nd` and
/// Rad/Deg affect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyFace {
    Text(&'static str),
    Glyph(&'static str),
}

pub fn key_face(key: Key, second: bool, angle: AngleMode) -> KeyFace {
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    match key {
        Key::Digit(digit) => KeyFace::Text(DIGITS[usize::from(digit.min(9))]),
        Key::Decimal => KeyFace::Text("."),
        Key::Percent => KeyFace::Text("%"),
        // The view substitutes the live AC/C label, matching Basic mode.
        Key::Clear => KeyFace::Text("AC"),
        Key::Backspace => KeyFace::Glyph("icons/calculator/backspace.svg"),
        Key::ToggleSign => KeyFace::Glyph("icons/calculator/plus-minus.svg"),
        Key::Equals => KeyFace::Glyph("icons/calculator/equals.svg"),
        Key::Operator(BinaryOp::Add) => KeyFace::Glyph("icons/calculator/plus.svg"),
        Key::Operator(BinaryOp::Subtract) => KeyFace::Glyph("icons/calculator/minus.svg"),
        Key::Operator(BinaryOp::Multiply) => KeyFace::Glyph("icons/calculator/multiply.svg"),
        Key::Operator(BinaryOp::Divide) => KeyFace::Glyph("icons/calculator/divide.svg"),
        // Never placed on the keypad directly (PowerOrRoot is used instead),
        // kept only so this match stays exhaustive over `BinaryOp`.
        Key::Operator(BinaryOp::Power | BinaryOp::Root) => KeyFace::Text("xʸ"),
        Key::PowerOrRoot => KeyFace::Text(if second { "ʸ√x" } else { "xʸ" }),
        Key::Second => KeyFace::Text("2nd"),
        Key::Unary(UnaryFn::SquareOrRoot) => KeyFace::Text(if second { "√x" } else { "x²" }),
        Key::Unary(UnaryFn::CubeOrCbrt) => KeyFace::Text(if second { "∛x" } else { "x³" }),
        Key::Unary(UnaryFn::ExpOrLn) => KeyFace::Text(if second { "ln" } else { "eˣ" }),
        Key::Unary(UnaryFn::TenPowOrLog10) => {
            KeyFace::Text(if second { "log₁₀" } else { "10ˣ" })
        }
        Key::Unary(UnaryFn::Sin) => KeyFace::Text(if second { "sin⁻¹" } else { "sin" }),
        Key::Unary(UnaryFn::Cos) => KeyFace::Text(if second { "cos⁻¹" } else { "cos" }),
        Key::Unary(UnaryFn::Tan) => KeyFace::Text(if second { "tan⁻¹" } else { "tan" }),
        Key::Unary(UnaryFn::Reciprocal) => KeyFace::Text("1/x"),
        Key::Unary(UnaryFn::Factorial) => KeyFace::Text("x!"),
        Key::Pi => KeyFace::Text("π"),
        Key::E => KeyFace::Text("e"),
        Key::Rand => KeyFace::Text("Rand"),
        Key::Ee => KeyFace::Text("EE"),
        Key::RadDeg => KeyFace::Text(match angle {
            AngleMode::Degrees => "Rad",
            AngleMode::Radians => "Deg",
        }),
        Key::MemoryClear => KeyFace::Text("mc"),
        Key::MemoryAdd => KeyFace::Text("m+"),
        Key::MemorySubtract => KeyFace::Text("m-"),
        Key::MemoryRecall => KeyFace::Text("mr"),
        Key::OpenParen => KeyFace::Text("("),
        Key::CloseParen => KeyFace::Text(")"),
    }
}

/// A stable, toggle-independent identifier for a key: used for element ids
/// and accessible names, where "x²/√x" needs one name regardless of `2nd`.
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
        Key::Operator(BinaryOp::Add) => "add",
        Key::Operator(BinaryOp::Subtract) => "subtract",
        Key::Operator(BinaryOp::Multiply) => "multiply",
        Key::Operator(BinaryOp::Divide) => "divide",
        Key::Operator(BinaryOp::Power | BinaryOp::Root) => "power-or-root",
        Key::PowerOrRoot => "power-or-root",
        Key::Second => "second",
        Key::Unary(UnaryFn::SquareOrRoot) => "square-or-square-root",
        Key::Unary(UnaryFn::CubeOrCbrt) => "cube-or-cube-root",
        Key::Unary(UnaryFn::ExpOrLn) => "exp-or-natural-log",
        Key::Unary(UnaryFn::TenPowOrLog10) => "ten-power-or-log-ten",
        Key::Unary(UnaryFn::Sin) => "sine",
        Key::Unary(UnaryFn::Cos) => "cosine",
        Key::Unary(UnaryFn::Tan) => "tangent",
        Key::Unary(UnaryFn::Reciprocal) => "reciprocal",
        Key::Unary(UnaryFn::Factorial) => "factorial",
        Key::Pi => "pi",
        Key::E => "e",
        Key::Rand => "random",
        Key::Ee => "exponent-entry",
        Key::RadDeg => "radians-or-degrees",
        Key::MemoryClear => "memory-clear",
        Key::MemoryAdd => "memory-add",
        Key::MemorySubtract => "memory-subtract",
        Key::MemoryRecall => "memory-recall",
        Key::OpenParen => "open-paren",
        Key::CloseParen => "close-paren",
    }
}

/// Map a hardware key press to a Scientific keypad key. Scientific accepts
/// every Basic keyboard shortcut unchanged (digits, `+-*/=%`, Enter, Escape,
/// Backspace, `C`) plus `(` and `)`. It does not yet bind extra keyboard
/// shortcuts for the function keys — those are toolbar/keypad clicks only,
/// matching this task's scope (CALC-02 asks for working keys, not a full
/// keyboard layer).
pub fn key_for_input(key: &str, key_char: Option<&str>, command_or_control: bool) -> Option<Key> {
    if command_or_control {
        return None;
    }
    if let Some(character) = key_char.filter(|text| text.len() == 1) {
        match character {
            "(" => return Some(Key::OpenParen),
            ")" => return Some(Key::CloseParen),
            _ => {}
        }
    }
    keypad::key_for_input(key, key_char, command_or_control).map(basic_key_to_scientific)
}

/// Basic's key set is a subset of Scientific's; every Basic key has a direct
/// Scientific equivalent with the same behaviour.
fn basic_key_to_scientific(key: crate::engine::Key) -> Key {
    use crate::engine::{Key as BasicKey, Operator as BasicOperator};
    match key {
        BasicKey::Digit(digit) => Key::Digit(digit),
        BasicKey::Decimal => Key::Decimal,
        BasicKey::Operator(BasicOperator::Add) => Key::Operator(BinaryOp::Add),
        BasicKey::Operator(BasicOperator::Subtract) => Key::Operator(BinaryOp::Subtract),
        BasicKey::Operator(BasicOperator::Multiply) => Key::Operator(BinaryOp::Multiply),
        BasicKey::Operator(BasicOperator::Divide) => Key::Operator(BinaryOp::Divide),
        BasicKey::Equals => Key::Equals,
        BasicKey::Percent => Key::Percent,
        BasicKey::ToggleSign => Key::ToggleSign,
        BasicKey::Clear => Key::Clear,
        BasicKey::Backspace => Key::Backspace,
    }
}

/// Palettes are shared with Basic mode unchanged: same app, same appearance.
pub type ScientificPalette = Palette;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypad_geometry_matches_the_documented_formula() {
        let (left, top) = key_origin(0, 0);
        assert_eq!((left, top), (KEYPAD_LEFT, KEYPAD_TOP));
        let (right_col, bottom_row) = key_origin(4, 8);
        assert_eq!(
            WINDOW_WIDTH,
            right_col + KEY_DIAMETER + KEYPAD_LEFT,
            "width should match key_origin's rightmost column plus a margin \
             equal to the left margin"
        );
        assert_eq!(
            WINDOW_HEIGHT,
            bottom_row + KEY_DIAMETER + 10.0,
            "height should match key_origin's bottom row plus Basic's 10 pt \
             margin"
        );
        assert_eq!(KEY_PITCH - KEY_DIAMETER, 6.0, "keeps Basic's 6 pt gap");
    }

    #[test]
    fn layout_has_the_expected_key_set() {
        let keys: Vec<Key> = LAYOUT.iter().flatten().flatten().copied().collect();
        assert_eq!(keys.len(), 22 + 20, "22 function keys plus Basic's 20");
        let digits = keys
            .iter()
            .filter(|key| matches!(key, Key::Digit(_)))
            .count();
        assert_eq!(digits, 10);
        assert!(keys.contains(&Key::Second));
        assert!(keys.contains(&Key::PowerOrRoot));
        assert!(keys.contains(&Key::OpenParen));
        assert!(keys.contains(&Key::CloseParen));
        assert!(keys.contains(&Key::MemoryClear));
        assert!(keys.contains(&Key::Rand));
        assert!(keys.contains(&Key::Ee));
        assert!(keys.contains(&Key::RadDeg));
        for row in LAYOUT.iter() {
            assert!(matches!(row[8], Some(Key::Operator(_) | Key::Equals)));
        }
    }

    #[test]
    fn every_present_key_has_a_face_and_a_stable_name() {
        for key in LAYOUT.iter().flatten().flatten().copied() {
            assert!(!key_name(key).is_empty());
            for second in [false, true] {
                for angle in [AngleMode::Degrees, AngleMode::Radians] {
                    match key_face(key, second, angle) {
                        KeyFace::Text(text) => assert!(!text.is_empty()),
                        KeyFace::Glyph(path) => assert!(path.starts_with("icons/calculator/")),
                    }
                }
            }
        }
    }

    #[test]
    fn second_flips_the_paired_function_labels() {
        assert_eq!(
            key_face(Key::Unary(UnaryFn::SquareOrRoot), false, AngleMode::Degrees),
            KeyFace::Text("x²")
        );
        assert_eq!(
            key_face(Key::Unary(UnaryFn::SquareOrRoot), true, AngleMode::Degrees),
            KeyFace::Text("√x")
        );
        assert_eq!(
            key_face(Key::PowerOrRoot, false, AngleMode::Degrees),
            KeyFace::Text("xʸ")
        );
        assert_eq!(
            key_face(Key::PowerOrRoot, true, AngleMode::Degrees),
            KeyFace::Text("ʸ√x")
        );
    }

    #[test]
    fn rad_deg_label_reflects_the_current_mode() {
        assert_eq!(
            key_face(Key::RadDeg, false, AngleMode::Degrees),
            KeyFace::Text("Rad")
        );
        assert_eq!(
            key_face(Key::RadDeg, false, AngleMode::Radians),
            KeyFace::Text("Deg")
        );
    }

    #[test]
    fn key_style_matches_the_three_tiers() {
        assert_eq!(key_style(Key::Digit(4)), KeyStyle::Digit);
        assert_eq!(key_style(Key::ToggleSign), KeyStyle::Digit);
        assert_eq!(key_style(Key::Operator(BinaryOp::Add)), KeyStyle::Operator);
        assert_eq!(key_style(Key::Equals), KeyStyle::Operator);
        assert_eq!(key_style(Key::Second), KeyStyle::Function);
        assert_eq!(key_style(Key::OpenParen), KeyStyle::Function);
        assert_eq!(key_style(Key::MemoryClear), KeyStyle::Function);
    }

    #[test]
    fn keyboard_maps_basic_keys_and_parentheses() {
        assert_eq!(key_for_input("7", Some("7"), false), Some(Key::Digit(7)));
        assert_eq!(
            key_for_input("=", Some("+"), false),
            Some(Key::Operator(BinaryOp::Add))
        );
        assert_eq!(key_for_input("enter", None, false), Some(Key::Equals));
        assert_eq!(key_for_input("escape", None, false), Some(Key::Clear));
        assert_eq!(key_for_input("9", Some("("), false), Some(Key::OpenParen));
        assert_eq!(key_for_input("0", Some(")"), false), Some(Key::CloseParen));
        assert_eq!(key_for_input("c", Some("c"), true), None);
    }

    #[test]
    fn toolbar_buttons_stay_inside_the_window_with_basic_spacing() {
        assert_eq!(MODE_BUTTON_CENTER_X - SIDEBAR_BUTTON_CENTER_X, 80.0);
        assert!(MODE_BUTTON_CENTER_X + TOOLBAR_BUTTON_DIAMETER / 2.0 < WINDOW_WIDTH);
        assert!(SIDEBAR_BUTTON_CENTER_X - TOOLBAR_BUTTON_DIAMETER / 2.0 > 0.0);
    }
}
