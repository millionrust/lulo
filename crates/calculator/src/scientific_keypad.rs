//! Scientific-mode keypad layout, metrics and keyboard mapping (CALC-02).
//!
//! Measured live from the owner's Mac (macOS 26.2, 2026-09-25): opened
//! Calculator, switched to Scientific (View ▸ Scientific), and read the AX
//! tree with System Events (`position`/`size` of every button) plus a
//! `screencapture -x` capture read pixel-for-pixel. Both agree to the point:
//!
//! - Window: **674 × 408** (content size; height matches Basic's own
//!   406/408 — see `keypad.rs`'s doc comment on that 2 pt discrepancy —
//!   reused here rather than re-litigated).
//! - **10 columns × 5 rows**, all 50 cells filled (no unused slots, unlike
//!   earlier guesses). The rightmost 4 columns are *exactly* Basic's own
//!   4-column digit/operator grid (`⌫ AC % ÷` / `7 8 9 ×` / `4 5 6 −` /
//!   `1 2 3 +` / `± 0 . =`), with 6 function columns prepended.
//! - Keys are **60 × 48 pt stadium-rounded pills** (corner radius = height
//!   ÷ 2 = 24, i.e. fully rounded top and bottom, straight sides) on a
//!   **66 × 54 pt pitch** — a 6 pt gap on both axes, matching Basic's own
//!   gap ratio. Row pitch (54) and origin (10, 132) are exactly Basic's;
//!   only the column pitch (66, vs Basic's 54) and column count are new.
//! - Toolbar, traffic lights and display geometry are all unchanged from
//!   Basic (same 52 pt toolbar, same two-line display); only the two
//!   toolbar buttons' x position moves with the new width (mode button
//!   keeps Basic's measured 26 pt margin from the trailing edge; the
//!   sidebar button keeps Basic's absolute 124 pt from the leading edge).
//!
//! Confirmed by clicking real keys (`click at` + reading the AX tree, not
//! guessed): `2nd` toggles `eˣ`↔`yˣ`, `10ˣ`↔`2ˣ`, `ln`↔`logᵧ`,
//! `log₁₀`↔`log₂`, and the trig/hyperbolic-trig rows to their inverses —
//! `x²`, `x³`, `xʸ`, `²√x`, `³√x`, `ʸ√x` and `1/x` do **not** change with
//! `2nd`, they are permanent keys. Rad/Deg additionally shows a small
//! persistent label above the keypad naming the *active* mode (separate
//! from the toggle key, which always names the mode pressing it would
//! switch *to* — `view.rs` renders it).
use crate::keypad::{self, KeyStyle, Palette};
use crate::scientific::{AngleMode, BinaryOp, Key};

/// Content size of the fixed-size window. Measured.
pub const WINDOW_WIDTH: f32 = 674.0;
/// Reuses Basic's own constant, including its documented 2 pt discrepancy
/// against the true measured 408 — see `keypad.rs`. Scientific's toolbar,
/// row count and row pitch are identical to Basic's, so re-deriving a
/// different number here would be a new mismatch, not a fix.
pub const WINDOW_HEIGHT: f32 = keypad::WINDOW_HEIGHT;

/// Keys are 60 × 48 pt stadium pills (corner radius `KEY_HEIGHT / 2`) on a
/// 66 × 54 pt pitch (a 6 pt gap on both axes, matching Basic's ratio).
pub const KEY_WIDTH: f32 = 60.0;
pub const KEY_HEIGHT: f32 = 48.0;
pub const KEY_PITCH_X: f32 = 66.0;
/// Row pitch, reused unchanged from Basic.
pub const KEY_PITCH_Y: f32 = keypad::KEY_PITCH;
/// Left edge of the first key column and top edge of the first key row.
/// Reuses Basic's measured margins unchanged.
pub const KEYPAD_LEFT: f32 = keypad::KEYPAD_LEFT;
pub const KEYPAD_TOP: f32 = keypad::KEYPAD_TOP;
pub const COLUMNS: usize = 10;
pub const ROWS: usize = 5;

/// Toolbar buttons keep Basic's measured 36 pt diameter. The sidebar
/// (history) button keeps Basic's absolute x position; the mode button
/// keeps Basic's measured 26 pt margin from the window's trailing edge.
pub const TOOLBAR_BUTTON_DIAMETER: f32 = keypad::TOOLBAR_BUTTON_DIAMETER;
pub const SIDEBAR_BUTTON_CENTER_X: f32 = keypad::SIDEBAR_BUTTON_CENTER_X;
pub const MODE_BUTTON_CENTER_X: f32 = WINDOW_WIDTH - 26.0;

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

/// Function-key label size. Not pixel-measured (the Mac auto-shrinks long
/// labels like `log₁₀`/`sinh⁻¹` per-string rather than using one fixed
/// size); this is a reasonable fit for the widest labels in a 60 pt pill.
pub const LABEL_SIZE: f32 = 17.0; // S: reasonable fit, not measured per-glyph.
/// Digits and the four basic operators keep Basic's measured label size.
pub const DIGIT_LABEL_SIZE: f32 = keypad::LABEL_SIZE;
pub const GLYPH_SIZE: f32 = keypad::GLYPH_SIZE;

/// The small "Rad"/nothing indicator above the keypad, shown only while
/// radians is active (see the module doc comment). Position estimated from
/// the capture (it sits left-aligned, just above row 1); not pixel-measured
/// to the point.
pub const ANGLE_INDICATOR_TOP: f32 = KEYPAD_TOP - 20.0; // S
pub const ANGLE_INDICATOR_SIZE: f32 = 12.0; // S

/// Rows top to bottom, columns left to right, exactly as read from the Mac.
/// Every cell is filled — unlike the old (unmeasured) 9-column guess, there
/// are no unused slots.
pub const LAYOUT: [[Key; COLUMNS]; ROWS] = [
    [
        Key::OpenParen,
        Key::CloseParen,
        Key::MemoryClear,
        Key::MemoryAdd,
        Key::MemorySubtract,
        Key::MemoryRecall,
        Key::Backspace,
        Key::Clear,
        Key::Percent,
        Key::Operator(BinaryOp::Divide),
    ],
    [
        Key::Second,
        Key::Square,
        Key::Cube,
        Key::Power,
        Key::ExpOrYPower,
        Key::TenPowOrTwoPow,
        Key::Digit(7),
        Key::Digit(8),
        Key::Digit(9),
        Key::Operator(BinaryOp::Multiply),
    ],
    [
        Key::Reciprocal,
        Key::SquareRoot,
        Key::CubeRoot,
        Key::YRoot,
        Key::LnOrLogY,
        Key::Log10OrLog2,
        Key::Digit(4),
        Key::Digit(5),
        Key::Digit(6),
        Key::Operator(BinaryOp::Subtract),
    ],
    [
        Key::Factorial,
        Key::Sin,
        Key::Cos,
        Key::Tan,
        Key::E,
        Key::Ee,
        Key::Digit(1),
        Key::Digit(2),
        Key::Digit(3),
        Key::Operator(BinaryOp::Add),
    ],
    [
        Key::Rand,
        Key::Sinh,
        Key::Cosh,
        Key::Tanh,
        Key::Pi,
        Key::RadDeg,
        Key::ToggleSign,
        Key::Digit(0),
        Key::Decimal,
        Key::Equals,
    ],
];

/// Top-left corner of the key at `row`, `column`.
pub fn key_origin(row: usize, column: usize) -> (f32, f32) {
    (
        KEYPAD_LEFT + column as f32 * KEY_PITCH_X,
        KEYPAD_TOP + row as f32 * KEY_PITCH_Y,
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
        // Never placed on the keypad directly (Power/YRoot keys are used
        // instead), kept only so this match stays exhaustive over `BinaryOp`.
        Key::Operator(BinaryOp::Power | BinaryOp::Root | BinaryOp::LogBase) => KeyFace::Text("xʸ"),
        Key::Second => KeyFace::Text("2nd"),
        Key::Square => KeyFace::Text("x²"),
        Key::Cube => KeyFace::Text("x³"),
        Key::Power => KeyFace::Text("xʸ"),
        Key::SquareRoot => KeyFace::Text("²√x"),
        Key::CubeRoot => KeyFace::Text("³√x"),
        Key::YRoot => KeyFace::Text("ʸ√x"),
        Key::Reciprocal => KeyFace::Text("1/x"),
        Key::Factorial => KeyFace::Text("x!"),
        Key::ExpOrYPower => KeyFace::Text(if second { "yˣ" } else { "eˣ" }),
        Key::TenPowOrTwoPow => KeyFace::Text(if second { "2ˣ" } else { "10ˣ" }),
        Key::LnOrLogY => KeyFace::Text(if second { "logᵧ" } else { "ln" }),
        Key::Log10OrLog2 => KeyFace::Text(if second { "log₂" } else { "log₁₀" }),
        Key::Sin => KeyFace::Text(if second { "sin⁻¹" } else { "sin" }),
        Key::Cos => KeyFace::Text(if second { "cos⁻¹" } else { "cos" }),
        Key::Tan => KeyFace::Text(if second { "tan⁻¹" } else { "tan" }),
        Key::Sinh => KeyFace::Text(if second { "sinh⁻¹" } else { "sinh" }),
        Key::Cosh => KeyFace::Text(if second { "cosh⁻¹" } else { "cosh" }),
        Key::Tanh => KeyFace::Text(if second { "tanh⁻¹" } else { "tanh" }),
        Key::Pi => KeyFace::Text("π"),
        Key::E => KeyFace::Text("e"),
        Key::Rand => KeyFace::Text("Rand"),
        Key::Ee => KeyFace::Text("EE"),
        // The key itself always names the mode pressing it would switch TO
        // (measured: it read "Rad" while in degrees, "Deg" while already in
        // radians) — the opposite sense from the persistent indicator label.
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
/// and accessible names, where e.g. "sin/sin⁻¹" needs one name regardless
/// of `2nd`.
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
        Key::Operator(BinaryOp::Power | BinaryOp::Root | BinaryOp::LogBase) => "operator",
        Key::Second => "second",
        Key::Square => "square",
        Key::Cube => "cube",
        Key::Power => "power",
        Key::SquareRoot => "square-root",
        Key::CubeRoot => "cube-root",
        Key::YRoot => "y-root",
        Key::Reciprocal => "reciprocal",
        Key::Factorial => "factorial",
        Key::ExpOrYPower => "exp-or-y-power",
        Key::TenPowOrTwoPow => "ten-power-or-two-power",
        Key::LnOrLogY => "ln-or-log-base-y",
        Key::Log10OrLog2 => "log-ten-or-log-two",
        Key::Sin => "sine",
        Key::Cos => "cosine",
        Key::Tan => "tangent",
        Key::Sinh => "hyperbolic-sine",
        Key::Cosh => "hyperbolic-cosine",
        Key::Tanh => "hyperbolic-tangent",
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
    fn keypad_geometry_matches_the_measured_grid() {
        let (left, top) = key_origin(0, 0);
        assert_eq!((left, top), (KEYPAD_LEFT, KEYPAD_TOP));
        let (right_col, bottom_row) = key_origin(ROWS - 1, COLUMNS - 1);
        assert_eq!(
            WINDOW_WIDTH,
            right_col + KEY_WIDTH + KEYPAD_LEFT,
            "width should match key_origin's rightmost column plus a margin \
             equal to the left margin"
        );
        assert_eq!(KEY_PITCH_X - KEY_WIDTH, 6.0, "keeps a 6 pt column gap");
        assert_eq!(KEY_PITCH_Y - KEY_HEIGHT, 6.0, "keeps Basic's 6 pt row gap");
        assert_eq!(WINDOW_WIDTH, 674.0, "measured on the Mac");
        let _ = bottom_row; // exercised via WINDOW_HEIGHT's reuse of Basic's.
    }

    #[test]
    fn the_rightmost_four_columns_are_exactly_basics_grid() {
        for row in 0..ROWS {
            for column in 0..4 {
                assert_eq!(
                    LAYOUT[row][6 + column],
                    basic_key_to_scientific(keypad::LAYOUT[row][column])
                );
            }
        }
    }

    #[test]
    fn layout_has_all_fifty_measured_keys() {
        let keys: Vec<Key> = LAYOUT.iter().flatten().copied().collect();
        assert_eq!(keys.len(), 50);
        let digits = keys
            .iter()
            .filter(|key| matches!(key, Key::Digit(_)))
            .count();
        assert_eq!(digits, 10);
        for always_present in [
            Key::Square,
            Key::Cube,
            Key::Power,
            Key::SquareRoot,
            Key::CubeRoot,
            Key::YRoot,
            Key::Reciprocal,
            Key::Second,
            Key::OpenParen,
            Key::CloseParen,
            Key::MemoryClear,
            Key::Rand,
            Key::Ee,
            Key::RadDeg,
        ] {
            assert!(keys.contains(&always_present), "{always_present:?} missing");
        }
    }

    #[test]
    fn every_present_key_has_a_face_and_a_stable_name() {
        for key in LAYOUT.iter().flatten().copied() {
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
    fn second_flips_only_the_measured_pairs() {
        // Always-present, never toggled.
        assert_eq!(
            key_face(Key::Square, true, AngleMode::Degrees),
            KeyFace::Text("x²")
        );
        assert_eq!(
            key_face(Key::SquareRoot, true, AngleMode::Degrees),
            KeyFace::Text("²√x")
        );
        // The measured 2nd pairs.
        assert_eq!(
            key_face(Key::ExpOrYPower, false, AngleMode::Degrees),
            KeyFace::Text("eˣ")
        );
        assert_eq!(
            key_face(Key::ExpOrYPower, true, AngleMode::Degrees),
            KeyFace::Text("yˣ")
        );
        assert_eq!(
            key_face(Key::TenPowOrTwoPow, true, AngleMode::Degrees),
            KeyFace::Text("2ˣ")
        );
        assert_eq!(
            key_face(Key::Sin, true, AngleMode::Degrees),
            KeyFace::Text("sin⁻¹")
        );
        assert_eq!(
            key_face(Key::Sinh, true, AngleMode::Degrees),
            KeyFace::Text("sinh⁻¹")
        );
    }

    #[test]
    fn rad_deg_key_names_the_mode_pressing_it_switches_to() {
        // Measured: the key reads "Rad" while in degrees (press it to go to
        // radians) and "Deg" while already in radians.
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
        assert_eq!(key_style(Key::Square), KeyStyle::Function);
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
    fn toolbar_buttons_stay_inside_the_window_with_basics_margins() {
        assert_eq!(WINDOW_WIDTH - MODE_BUTTON_CENTER_X, 26.0);
        assert_eq!(SIDEBAR_BUTTON_CENTER_X, keypad::SIDEBAR_BUTTON_CENTER_X);
        const { assert!(MODE_BUTTON_CENTER_X + TOOLBAR_BUTTON_DIAMETER / 2.0 < WINDOW_WIDTH) };
        const { assert!(SIDEBAR_BUTTON_CENTER_X - TOOLBAR_BUTTON_DIAMETER / 2.0 > 0.0) };
    }
}
