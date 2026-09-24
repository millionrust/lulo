//! The Scientific-mode calculator engine: pure state, no GPUI.
//!
//! Scientific keeps Basic's immediate-apply model (CALC-02): operators,
//! including the new `xʸ` / `ʸ√x` pair, evaluate straight away against the
//! running accumulator, left to right, with no algebraic precedence — the
//! same rule `engine.rs` documents and tests for Basic mode. Scientific adds:
//!
//! - Unary functions that apply instantly to the current value: `x²`/`√x`,
//!   `x³`/`∛x`, `eˣ`/`ln`, `10ˣ`/`log₁₀`, `sin`/`cos`/`tan` and their
//!   inverses, `1/x`, `x!`.
//! - `2nd`, which flips every function above to its paired alternate label
//!   and behaviour (macOS Calculator's own convention).
//! - A Rad/Deg toggle that affects `sin`, `cos`, `tan` and their inverses.
//! - `π`, `e`, `Rand` (a fresh random value each press) and `EE` (scientific
//!   exponent entry into the current entry, e.g. `1.5e10`).
//! - A single memory register: `mc` clears it, `m+`/`m-` add/subtract the
//!   current value, `mr` recalls it as a fresh entry.
//! - Real parenthesis nesting: `(` pushes the outer accumulator/pending
//!   operator and starts a fresh sub-calculation; `)` evaluates that
//!   sub-calculation to one value, pops the outer state, and feeds the value
//!   back in as the resumed calculation's operand — nesting works to any
//!   depth.
//!
//! Number formatting, parsing and the `AC`/`C` clear-label rule are shared
//! with Basic mode via `crate::engine` (`format_value`, `format_entry`,
//! `parse_number`, `ClearLabel`, `MAX_DIGITS`) rather than duplicated.

use crate::engine::{
    format_entry, format_value, parse_number, ClearLabel, HistoryEntry, ERROR_TEXT, MAX_DIGITS,
};

/// Most exponent digits `EE` accepts, mirroring `MAX_DIGITS` for the mantissa.
const MAX_EXPONENT_DIGITS: usize = 3;

/// Degrees is macOS Calculator's default angle unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum AngleMode {
    #[default]
    Degrees,
    Radians,
}

/// A key that combines two operands into one, evaluated immediately like
/// Basic's four operators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    /// `xʸ`: the accumulator raised to the following operand.
    Power,
    /// `ʸ√x`: the following operand-th root of the accumulator.
    Root,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "−",
            Self::Multiply => "×",
            Self::Divide => "÷",
            Self::Power => "^",
            Self::Root => "ʸ√",
        }
    }

    fn apply(self, left: f64, right: f64) -> Option<f64> {
        let value = match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide if right == 0.0 => return None,
            Self::Divide => left / right,
            Self::Power => left.powf(right),
            Self::Root if right == 0.0 => return None,
            Self::Root if left < 0.0 => {
                // An odd-integer root of a negative number is real (e.g.
                // cube root of −8 is −2); anything else is undefined here.
                if right.fract() == 0.0 && (right as i64) % 2 != 0 {
                    -((-left).powf(1.0 / right))
                } else {
                    return None;
                }
            }
            Self::Root => left.powf(1.0 / right),
        };
        value.is_finite().then_some(value)
    }
}

/// A key that applies instantly to the current value. `2nd` flips which half
/// of each pair is active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryFn {
    /// `x²` / `√x`.
    SquareOrRoot,
    /// `x³` / `∛x`.
    CubeOrCbrt,
    /// `eˣ` / `ln`.
    ExpOrLn,
    /// `10ˣ` / `log₁₀`.
    TenPowOrLog10,
    Sin,
    Cos,
    Tan,
    /// `1/x`. Not affected by `2nd`.
    Reciprocal,
    /// `x!`. Not affected by `2nd`.
    Factorial,
}

/// Every key on the Scientific keypad.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Digit(u8),
    Decimal,
    Operator(BinaryOp),
    /// `xʸ` when `2nd` is off, `ʸ√x` when it is on.
    PowerOrRoot,
    Equals,
    Percent,
    ToggleSign,
    Clear,
    Backspace,
    Second,
    Unary(UnaryFn),
    Pi,
    E,
    Rand,
    /// Scientific-notation exponent entry.
    Ee,
    RadDeg,
    MemoryClear,
    MemoryAdd,
    MemorySubtract,
    MemoryRecall,
    OpenParen,
    CloseParen,
}

/// The outer calculation a `(` suspends, restored by the matching `)`.
#[derive(Clone, Debug)]
struct Frame {
    accumulator: Option<f64>,
    pending: Option<BinaryOp>,
    expression: String,
}

#[derive(Clone, Debug, Default)]
pub struct ScientificCalculator {
    value: f64,
    entry: Option<String>,
    accumulator: Option<f64>,
    pending: Option<BinaryOp>,
    /// A right operand exists for the pending operation. While false the
    /// operator key stays lit, exactly like Basic.
    operand_ready: bool,
    entry_active: bool,
    last: Option<(BinaryOp, f64)>,
    error: bool,
    expression: String,
    /// `2nd`: flips every paired function to its alternate.
    second: bool,
    angle: AngleMode,
    memory: f64,
    parens: Vec<Frame>,
    history: Vec<HistoryEntry>,
}

impl ScientificCalculator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn press(&mut self, key: Key) {
        match key {
            Key::Digit(digit) => self.digit(digit),
            Key::Decimal => self.decimal(),
            Key::Operator(operator) => self.operator(operator),
            Key::PowerOrRoot => {
                self.operator(if self.second {
                    BinaryOp::Root
                } else {
                    BinaryOp::Power
                });
            }
            Key::Equals => self.equals(),
            Key::Percent => self.percent(),
            Key::ToggleSign => self.toggle_sign(),
            Key::Clear => self.clear(),
            Key::Backspace => self.backspace(),
            Key::Second => self.second = !self.second,
            Key::Unary(func) => self.apply_unary(func),
            Key::Pi => self.constant(std::f64::consts::PI),
            Key::E => self.constant(std::f64::consts::E),
            Key::Rand => self.constant(random_unit()),
            Key::Ee => self.exponent_entry(),
            Key::RadDeg => {
                self.angle = match self.angle {
                    AngleMode::Degrees => AngleMode::Radians,
                    AngleMode::Radians => AngleMode::Degrees,
                };
            }
            Key::MemoryClear => self.memory = 0.0,
            Key::MemoryAdd => {
                if !self.error {
                    self.memory += self.current();
                    self.entry = None;
                    self.entry_active = false;
                }
            }
            Key::MemorySubtract => {
                if !self.error {
                    self.memory -= self.current();
                    self.entry = None;
                    self.entry_active = false;
                }
            }
            Key::MemoryRecall => self.constant(self.memory),
            Key::OpenParen => self.open_paren(),
            Key::CloseParen => self.close_paren(),
        }
    }

    /// The main display text, grouped and rounded for display.
    pub fn display(&self) -> String {
        if self.error {
            return ERROR_TEXT.to_owned();
        }
        match &self.entry {
            Some(entry) => format_scientific_entry(entry),
            None => format_value(self.value),
        }
    }

    /// The secondary expression line above the result. Empty when idle.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    pub fn highlighted_operator(&self) -> Option<BinaryOp> {
        if self.error || self.operand_ready {
            None
        } else {
            self.pending
        }
    }

    pub fn clear_label(&self) -> ClearLabel {
        if self.entry_active && !self.error {
            ClearLabel::Clear
        } else {
            ClearLabel::AllClear
        }
    }

    pub fn is_error(&self) -> bool {
        self.error
    }

    pub fn second(&self) -> bool {
        self.second
    }

    pub fn angle_mode(&self) -> AngleMode {
        self.angle
    }

    pub fn memory(&self) -> f64 {
        self.memory
    }

    /// How many `(` are still unmatched.
    pub fn open_parens(&self) -> usize {
        self.parens.len()
    }

    /// Completed calculations, oldest first.
    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// The value ⌘C puts on the clipboard.
    pub fn copy_text(&self) -> String {
        self.display().replace(',', "")
    }

    /// Paste a number as the current entry, or load a history result back in
    /// (both go through the same path as macOS's history tape).
    pub fn paste(&mut self, text: &str) -> bool {
        let Some(value) = parse_number(text) else {
            return false;
        };
        if self.error {
            self.all_clear();
        }
        if self.pending.is_none() {
            self.expression.clear();
        }
        self.value = value;
        self.entry = None;
        self.operand_ready = true;
        self.entry_active = true;
        self.update_expression();
        true
    }

    fn current(&self) -> f64 {
        let Some(entry) = &self.entry else {
            return self.value;
        };
        let text = entry
            .strip_suffix("e-")
            .or_else(|| entry.strip_suffix('e'))
            .unwrap_or(entry);
        text.parse::<f64>().unwrap_or(0.0)
    }

    fn commit_entry(&mut self) {
        if self.entry.is_some() {
            self.value = self.current();
            self.entry = None;
        }
    }

    fn fail(&mut self) {
        let (memory, angle, second, history) = (
            self.memory,
            self.angle,
            self.second,
            std::mem::take(&mut self.history),
        );
        *self = Self {
            error: true,
            memory,
            angle,
            second,
            history,
            ..Self::default()
        };
    }

    fn start_entry(&mut self, text: &str) {
        if self.error {
            self.all_clear();
        }
        if self.pending.is_none() {
            self.expression.clear();
        }
        self.entry = Some(text.to_owned());
        self.operand_ready = true;
        self.entry_active = true;
    }

    fn digit(&mut self, digit: u8) {
        let digit = char::from(b'0' + digit.min(9));
        match self.entry.as_mut() {
            Some(entry) if !self.error => {
                if let Some(exponent_start) = entry.find('e') {
                    let exponent_digits = entry[exponent_start + 1..]
                        .chars()
                        .filter(char::is_ascii_digit)
                        .count();
                    if exponent_digits >= MAX_EXPONENT_DIGITS {
                        return;
                    }
                } else {
                    if entry_digits(entry) >= MAX_DIGITS {
                        return;
                    }
                    let unsigned = entry.trim_start_matches('-');
                    if unsigned == "0" {
                        entry.pop();
                    }
                }
                entry.push(digit);
            }
            _ => self.start_entry(&digit.to_string()),
        }
        self.update_expression();
    }

    fn decimal(&mut self) {
        match self.entry.as_mut() {
            Some(entry) if !self.error => {
                if !entry.contains('e') && !entry.contains('.') && entry_digits(entry) < MAX_DIGITS
                {
                    entry.push('.');
                }
            }
            _ => self.start_entry("0."),
        }
        self.update_expression();
    }

    fn operator(&mut self, operator: BinaryOp) {
        if self.error {
            return;
        }
        if let (Some(left), Some(pending), true) =
            (self.accumulator, self.pending, self.operand_ready)
        {
            match pending.apply(left, self.current()) {
                Some(value) => {
                    self.value = value;
                    self.entry = None;
                }
                None => return self.fail(),
            }
        }
        self.commit_entry();
        self.accumulator = Some(self.value);
        self.pending = Some(operator);
        self.operand_ready = false;
        self.entry_active = false;
        self.last = None;
        self.update_expression();
    }

    fn equals(&mut self) {
        if self.error {
            return;
        }
        let (left, operator, right) = match (self.pending, self.accumulator, self.last) {
            (Some(operator), Some(left), _) => {
                let right = if self.operand_ready {
                    self.current()
                } else {
                    left
                };
                (left, operator, right)
            }
            (None, _, Some((operator, right))) => (self.current(), operator, right),
            _ => {
                self.commit_entry();
                self.entry_active = false;
                return;
            }
        };
        let Some(value) = operator.apply(left, right) else {
            return self.fail();
        };
        self.expression = format!(
            "{}{}{}",
            format_value(left),
            operator.symbol(),
            format_value(right)
        );
        self.history.push(HistoryEntry {
            expression: self.expression.clone(),
            result: format_value(value),
        });
        self.value = value;
        self.entry = None;
        self.accumulator = None;
        self.pending = None;
        self.operand_ready = false;
        self.entry_active = false;
        self.last = Some((operator, right));
    }

    fn percent(&mut self) {
        if self.error {
            return;
        }
        let current = self.current();
        let value = match (self.pending, self.accumulator) {
            (Some(BinaryOp::Add | BinaryOp::Subtract), Some(left)) => left * current / 100.0,
            _ => current / 100.0,
        };
        self.value = value;
        self.entry = None;
        self.operand_ready = true;
        self.entry_active = true;
        self.update_expression();
    }

    fn toggle_sign(&mut self) {
        if self.error {
            return;
        }
        if let Some(entry) = self.entry.as_mut() {
            match entry.find('e') {
                Some(exponent_start) => {
                    let (mantissa, exponent) = entry.split_at(exponent_start + 1);
                    let mantissa = mantissa.to_owned();
                    let exponent = match exponent.strip_prefix('-') {
                        Some(magnitude) => magnitude.to_owned(),
                        None => format!("-{exponent}"),
                    };
                    *entry = format!("{mantissa}{exponent}");
                }
                None => match entry.strip_prefix('-') {
                    Some(unsigned) => *entry = unsigned.to_owned(),
                    None => entry.insert(0, '-'),
                },
            }
        } else if (!self.operand_ready && self.pending.is_some()) || self.value == 0.0 {
            self.start_entry("-0");
        } else {
            self.value = -self.value;
            self.operand_ready = true;
            self.entry_active = true;
        }
        self.update_expression();
    }

    fn clear(&mut self) {
        match self.clear_label() {
            ClearLabel::Clear => self.clear_entry(),
            ClearLabel::AllClear => self.all_clear(),
        }
    }

    fn clear_entry(&mut self) {
        self.entry = None;
        self.value = 0.0;
        self.operand_ready = false;
        self.entry_active = false;
        self.update_expression();
    }

    fn all_clear(&mut self) {
        let (memory, angle, second, history) = (
            self.memory,
            self.angle,
            self.second,
            std::mem::take(&mut self.history),
        );
        *self = Self {
            memory,
            angle,
            second,
            history,
            ..Self::default()
        };
    }

    fn backspace(&mut self) {
        if self.error {
            return self.all_clear();
        }
        let Some(entry) = self.entry.as_mut() else {
            return;
        };
        entry.pop();
        if entry.is_empty() || entry == "-" {
            *entry = "0".to_owned();
        }
        self.update_expression();
    }

    fn update_expression(&mut self) {
        let prefix = "(".repeat(self.parens.len());
        let Some((left, operator)) = self.accumulator.zip(self.pending) else {
            if !prefix.is_empty() {
                self.expression = prefix;
            }
            return;
        };
        self.expression = format!("{prefix}{}{}", format_value(left), operator.symbol());
        if self.operand_ready {
            let right = match &self.entry {
                Some(entry) => format_scientific_entry(entry),
                None => format_value(self.value),
            };
            self.expression.push_str(&right);
        }
    }

    /// Apply a unary function to the current value straight away.
    fn apply_unary(&mut self, func: UnaryFn) {
        if self.error {
            return;
        }
        let value = self.current();
        let result = match func {
            UnaryFn::SquareOrRoot if !self.second => Some(value * value),
            UnaryFn::SquareOrRoot => (value >= 0.0).then(|| value.sqrt()),
            UnaryFn::CubeOrCbrt if !self.second => Some(value * value * value),
            UnaryFn::CubeOrCbrt => Some(value.cbrt()),
            UnaryFn::ExpOrLn if !self.second => Some(value.exp()),
            UnaryFn::ExpOrLn => (value > 0.0).then(|| value.ln()),
            UnaryFn::TenPowOrLog10 if !self.second => Some(10f64.powf(value)),
            UnaryFn::TenPowOrLog10 => (value > 0.0).then(|| value.log10()),
            UnaryFn::Sin => self.trig(value, Trig::Sin),
            UnaryFn::Cos => self.trig(value, Trig::Cos),
            UnaryFn::Tan => self.trig(value, Trig::Tan),
            UnaryFn::Reciprocal => (value != 0.0).then(|| 1.0 / value),
            UnaryFn::Factorial => factorial(value),
        };
        match result.filter(|value| value.is_finite()) {
            Some(value) => {
                self.value = value;
                self.entry = None;
                self.operand_ready = true;
                self.entry_active = true;
                self.update_expression();
            }
            None => self.fail(),
        }
    }

    fn trig(&self, value: f64, which: Trig) -> Option<f64> {
        if !self.second {
            let radians = match self.angle {
                AngleMode::Degrees => value.to_radians(),
                AngleMode::Radians => value,
            };
            return Some(match which {
                Trig::Sin => radians.sin(),
                Trig::Cos => radians.cos(),
                Trig::Tan => radians.tan(),
            });
        }
        let radians = match which {
            Trig::Sin => (-1.0..=1.0).contains(&value).then(|| value.asin())?,
            Trig::Cos => (-1.0..=1.0).contains(&value).then(|| value.acos())?,
            Trig::Tan => value.atan(),
        };
        Some(match self.angle {
            AngleMode::Degrees => radians.to_degrees(),
            AngleMode::Radians => radians,
        })
    }

    /// `π`, `e`, `Rand` and `mr` all replace the display with a fresh value,
    /// ready to be used as the next operand.
    fn constant(&mut self, value: f64) {
        if self.error {
            self.all_clear();
        }
        if self.pending.is_none() {
            self.expression.clear();
        }
        self.value = value;
        self.entry = None;
        self.operand_ready = true;
        self.entry_active = true;
        self.update_expression();
    }

    /// `EE`: start typing a power-of-ten exponent onto the current entry.
    fn exponent_entry(&mut self) {
        if self.error {
            return;
        }
        match self.entry.as_mut() {
            Some(entry) if !entry.contains('e') => entry.push('e'),
            Some(_) => {}
            None => {
                let mut entry = plain_number(self.value);
                entry.push('e');
                self.entry = Some(entry);
                self.operand_ready = true;
                self.entry_active = true;
            }
        }
        self.update_expression();
    }

    /// `(`: suspend the outer calculation and start a fresh sub-calculation.
    fn open_paren(&mut self) {
        if self.error {
            self.all_clear();
        }
        self.commit_entry();
        self.parens.push(Frame {
            accumulator: self.accumulator,
            pending: self.pending,
            expression: self.expression.clone(),
        });
        self.value = 0.0;
        self.accumulator = None;
        self.pending = None;
        self.operand_ready = false;
        self.entry_active = false;
        self.entry = None;
        // `update_expression` reads `self.parens` for the `(` prefix, so it
        // already renders the fresh `(` here; nothing further is needed.
        self.update_expression();
    }

    /// `)`: evaluate the sub-calculation to one value and resume the frame
    /// `(` suspended, feeding that value in as its operand.
    fn close_paren(&mut self) {
        if self.error {
            return;
        }
        let Some(frame) = self.parens.pop() else {
            return;
        };
        let inner = match (self.pending, self.accumulator) {
            (Some(operator), Some(left)) => {
                let right = if self.operand_ready {
                    self.current()
                } else {
                    left
                };
                operator.apply(left, right)
            }
            _ => {
                self.commit_entry();
                Some(self.value)
            }
        };
        let Some(inner_value) = inner else {
            // `fail` resets to a fresh (empty) paren stack, like Basic's
            // error state resets every other in-progress field.
            return self.fail();
        };
        self.accumulator = frame.accumulator;
        self.pending = frame.pending;
        self.value = inner_value;
        self.entry = None;
        self.operand_ready = true;
        self.entry_active = true;
        self.expression = format!("{}{})", frame.expression, format_value(inner_value));
    }
}

#[derive(Clone, Copy)]
enum Trig {
    Sin,
    Cos,
    Tan,
}

fn factorial(value: f64) -> Option<f64> {
    if value < 0.0 || value.fract() != 0.0 || value > 170.0 {
        return None;
    }
    let n = value as u64;
    let mut result = 1.0f64;
    for i in 2..=n {
        result *= i as f64;
    }
    Some(result)
}

/// A dependency-free xorshift64* generator seeded from the clock: not
/// cryptographic, but a genuine, non-repeating source for `Rand`, matching
/// what a calculator's random key needs. No `rand` crate is in the
/// workspace, so this follows the std-based fallback rather than adding one.
fn random_unit() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0x9E3779B97F4A7C15);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x2545_F491_4F6C_DD1D);
    let mut state = STATE.fetch_add(0x2545_F491_4F6C_DD1D, Ordering::Relaxed) ^ seed;
    if state == 0 {
        state = 0x9E3779B97F4A7C15;
    }
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    let bits = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
    (bits >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Render an in-progress entry that may hold a scientific-notation exponent
/// (`"1.5e10"`, `"1.5e-3"`, or `"1.5e"` while the exponent is still empty),
/// grouping only the mantissa.
fn format_scientific_entry(entry: &str) -> String {
    match entry.split_once('e') {
        Some((mantissa, exponent)) => format!("{}e{exponent}", format_entry(mantissa)),
        None => format_entry(entry),
    }
}

/// A plain (ungrouped) numeral for a value that is about to become an
/// editable entry, such as when `EE` is pressed with no entry in progress.
fn plain_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Count the digits in a typed mantissa, ignoring a lone leading `0` before
/// the decimal point. Mirrors `engine::entry_digits`, which is private to
/// that module.
fn entry_digits(entry: &str) -> usize {
    let unsigned = entry.trim_start_matches('-');
    let unsigned = unsigned.strip_prefix("0.").unwrap_or(unsigned);
    unsigned.chars().filter(char::is_ascii_digit).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    use BinaryOp::{Add, Divide, Multiply, Subtract};
    use Key::{
        Backspace, Clear, CloseParen, Decimal, Digit, Ee, Equals, MemoryAdd, MemoryClear,
        MemoryRecall, MemorySubtract, OpenParen, Percent, RadDeg, Rand, Second, ToggleSign,
    };
    use UnaryFn::{CubeOrCbrt, ExpOrLn, Factorial, Reciprocal, SquareOrRoot, TenPowOrLog10};

    fn calc() -> ScientificCalculator {
        ScientificCalculator::new()
    }

    fn press_digits(calculator: &mut ScientificCalculator, digits: &str) {
        for character in digits.chars() {
            match character {
                '0'..='9' => calculator.press(Digit(character as u8 - b'0')),
                '.' => calculator.press(Decimal),
                '-' => calculator.press(ToggleSign),
                other => panic!("unexpected digit character {other}"),
            }
        }
    }

    #[test]
    fn starts_at_zero_in_degrees_with_no_history() {
        let calculator = calc();
        assert_eq!(calculator.display(), "0");
        assert_eq!(calculator.angle_mode(), AngleMode::Degrees);
        assert!(!calculator.second());
        assert_eq!(calculator.memory(), 0.0);
        assert!(calculator.history().is_empty());
        assert_eq!(calculator.clear_label(), ClearLabel::AllClear);
    }

    #[test]
    fn basic_arithmetic_still_applies_immediately_with_no_precedence() {
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        press_digits(&mut calculator, "3");
        calculator.press(Key::Operator(Multiply));
        assert_eq!(calculator.display(), "5");
        assert_eq!(calculator.highlighted_operator(), Some(Multiply));
        press_digits(&mut calculator, "4");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "20");
    }

    #[test]
    fn square_and_square_root_toggle_with_second() {
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(Key::Unary(SquareOrRoot));
        assert_eq!(calculator.display(), "25");
        calculator.press(Second);
        calculator.press(Key::Unary(SquareOrRoot));
        assert_eq!(calculator.display(), "5");
    }

    #[test]
    fn cube_and_cube_root_toggle_with_second() {
        let mut calculator = calc();
        press_digits(&mut calculator, "3");
        calculator.press(Key::Unary(CubeOrCbrt));
        assert_eq!(calculator.display(), "27");
        let mut calculator = calc();
        press_digits(&mut calculator, "27");
        calculator.press(Second);
        calculator.press(Key::Unary(CubeOrCbrt));
        assert_eq!(calculator.display(), "3");
    }

    #[test]
    fn exp_and_ln_toggle_with_second() {
        let mut calculator = calc();
        calculator.press(Key::Unary(ExpOrLn));
        assert_eq!(calculator.display(), "1");
        let mut calculator = calc();
        press_digits(&mut calculator, "1");
        calculator.press(Second);
        calculator.press(Key::Unary(ExpOrLn));
        assert_eq!(calculator.display(), "0");
    }

    #[test]
    fn ten_pow_and_log10_toggle_with_second() {
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Unary(TenPowOrLog10));
        assert_eq!(calculator.display(), "100");
        let mut calculator = calc();
        press_digits(&mut calculator, "100");
        calculator.press(Second);
        calculator.press(Key::Unary(TenPowOrLog10));
        assert_eq!(calculator.display(), "2");
    }

    #[test]
    fn log_of_zero_or_negative_is_an_error() {
        let mut calculator = calc();
        press_digits(&mut calculator, "0");
        calculator.press(Second);
        calculator.press(Key::Unary(TenPowOrLog10));
        assert!(calculator.is_error());

        let mut calculator = calc();
        press_digits(&mut calculator, "0");
        calculator.press(Second);
        calculator.press(Key::Unary(ExpOrLn));
        assert!(calculator.is_error());
    }

    #[test]
    fn reciprocal_and_division_by_zero_error() {
        let mut calculator = calc();
        press_digits(&mut calculator, "4");
        calculator.press(Key::Unary(Reciprocal));
        assert_eq!(calculator.display(), "0.25");
        let mut calculator = calc();
        calculator.press(Key::Unary(Reciprocal));
        assert!(calculator.is_error());
    }

    #[test]
    fn factorial_of_zero_is_one_and_negative_is_an_error() {
        let mut calculator = calc();
        calculator.press(Key::Unary(Factorial));
        assert_eq!(calculator.display(), "1");
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(Key::Unary(Factorial));
        assert_eq!(calculator.display(), "120");
        let mut calculator = calc();
        press_digits(&mut calculator, "3");
        calculator.press(ToggleSign);
        calculator.press(Key::Unary(Factorial));
        assert!(calculator.is_error());
        let mut calculator = calc();
        press_digits(&mut calculator, "2.5");
        calculator.press(Key::Unary(Factorial));
        assert!(calculator.is_error());
    }

    #[test]
    fn sin_cos_tan_use_degrees_by_default() {
        let mut calculator = calc();
        press_digits(&mut calculator, "90");
        calculator.press(Key::Unary(UnaryFn::Sin));
        assert_eq!(calculator.display(), "1");

        let mut calculator = calc();
        press_digits(&mut calculator, "0");
        calculator.press(Key::Unary(UnaryFn::Cos));
        assert_eq!(calculator.display(), "1");

        let mut calculator = calc();
        press_digits(&mut calculator, "45");
        calculator.press(Key::Unary(UnaryFn::Tan));
        assert_eq!(calculator.display(), "1");
    }

    #[test]
    fn rad_deg_toggle_changes_the_trig_result() {
        let mut degrees = calc();
        press_digits(&mut degrees, "90");
        degrees.press(Key::Unary(UnaryFn::Sin));

        let mut radians = calc();
        radians.press(RadDeg);
        press_digits(&mut radians, "90");
        radians.press(Key::Unary(UnaryFn::Sin));

        assert_eq!(radians.angle_mode(), AngleMode::Radians);
        assert_ne!(degrees.display(), radians.display());

        let mut half_pi_radians = calc();
        half_pi_radians.press(RadDeg);
        press_digits(&mut half_pi_radians, "1.5707963");
        half_pi_radians.press(Key::Unary(UnaryFn::Sin));
        assert_eq!(half_pi_radians.display(), "1");
    }

    #[test]
    fn inverse_trig_toggles_with_second_and_respects_domain() {
        let mut calculator = calc();
        press_digits(&mut calculator, "1");
        calculator.press(Second);
        calculator.press(Key::Unary(UnaryFn::Sin));
        assert_eq!(calculator.display(), "90");

        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Second);
        calculator.press(Key::Unary(UnaryFn::Sin));
        assert!(calculator.is_error());

        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Second);
        calculator.press(Key::Unary(UnaryFn::Cos));
        assert!(calculator.is_error());
    }

    #[test]
    fn power_and_root_toggle_via_second() {
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::PowerOrRoot);
        press_digits(&mut calculator, "10");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1,024");

        let mut calculator = calc();
        press_digits(&mut calculator, "8");
        calculator.press(Second);
        calculator.press(Key::PowerOrRoot);
        press_digits(&mut calculator, "3");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "2");
    }

    #[test]
    fn root_of_a_negative_base_is_real_for_an_odd_root() {
        let mut calculator = calc();
        press_digits(&mut calculator, "8");
        calculator.press(ToggleSign);
        calculator.press(Second);
        calculator.press(Key::PowerOrRoot);
        press_digits(&mut calculator, "3");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "-2");
    }

    #[test]
    fn root_of_a_negative_base_is_an_error_for_an_even_root() {
        let mut calculator = calc();
        press_digits(&mut calculator, "8");
        calculator.press(ToggleSign);
        calculator.press(Second);
        calculator.press(Key::PowerOrRoot);
        press_digits(&mut calculator, "2");
        calculator.press(Equals);
        assert!(calculator.is_error());
    }

    #[test]
    fn pi_and_e_are_the_math_constants() {
        let mut calculator = calc();
        calculator.press(Key::Pi);
        assert_eq!(calculator.display(), format_value(std::f64::consts::PI));
        let mut calculator = calc();
        calculator.press(Key::E);
        assert_eq!(calculator.display(), format_value(std::f64::consts::E));
    }

    #[test]
    fn rand_is_between_zero_and_one_and_changes_each_press() {
        let mut calculator = calc();
        calculator.press(Rand);
        let first = calculator.display();
        let first_value: f64 = first.replace(',', "").parse().unwrap();
        assert!((0.0..1.0).contains(&first_value));
        calculator.press(Rand);
        let second = calculator.display();
        assert_ne!(first, second);
    }

    #[test]
    fn ee_enters_a_scientific_exponent() {
        let mut calculator = calc();
        press_digits(&mut calculator, "1.5");
        calculator.press(Ee);
        press_digits(&mut calculator, "3");
        assert_eq!(calculator.display(), "1.5e3");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1,500");
    }

    #[test]
    fn ee_exponent_sign_toggles_independently_of_the_mantissa() {
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(Ee);
        press_digits(&mut calculator, "2");
        calculator.press(ToggleSign);
        assert_eq!(calculator.display(), "5e-2");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "0.05");
    }

    #[test]
    fn memory_add_subtract_and_recall() {
        let mut calculator = calc();
        press_digits(&mut calculator, "10");
        calculator.press(MemoryAdd);
        assert_eq!(calculator.memory(), 10.0);
        press_digits(&mut calculator, "4");
        calculator.press(MemorySubtract);
        assert_eq!(calculator.memory(), 6.0);
        calculator.press(Clear);
        calculator.press(MemoryRecall);
        assert_eq!(calculator.display(), "6");
    }

    #[test]
    fn memory_clear_resets_the_register() {
        let mut calculator = calc();
        press_digits(&mut calculator, "10");
        calculator.press(MemoryAdd);
        calculator.press(MemoryClear);
        assert_eq!(calculator.memory(), 0.0);
    }

    #[test]
    fn memory_survives_all_clear() {
        let mut calculator = calc();
        press_digits(&mut calculator, "10");
        calculator.press(MemoryAdd);
        calculator.press(Clear);
        calculator.press(Clear);
        assert_eq!(calculator.memory(), 10.0);
    }

    #[test]
    fn parentheses_group_a_sub_calculation() {
        // 2 + (3 × 4) = 14, evaluated with no algebraic precedence anywhere
        // except inside the parens themselves.
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        calculator.press(OpenParen);
        assert_eq!(calculator.open_parens(), 1);
        press_digits(&mut calculator, "3");
        calculator.press(Key::Operator(Multiply));
        press_digits(&mut calculator, "4");
        calculator.press(CloseParen);
        assert_eq!(calculator.open_parens(), 0);
        assert_eq!(calculator.display(), "12");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "14");
    }

    #[test]
    fn nested_parentheses_evaluate_from_the_inside_out() {
        // (2 + (3 × 4)) − 1 = 13.
        let mut calculator = calc();
        calculator.press(OpenParen);
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        calculator.press(OpenParen);
        press_digits(&mut calculator, "3");
        calculator.press(Key::Operator(Multiply));
        press_digits(&mut calculator, "4");
        calculator.press(CloseParen);
        assert_eq!(calculator.open_parens(), 1);
        calculator.press(CloseParen);
        assert_eq!(calculator.open_parens(), 0);
        assert_eq!(calculator.display(), "14");
        calculator.press(Key::Operator(Subtract));
        press_digits(&mut calculator, "1");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "13");
    }

    #[test]
    fn parens_combine_with_a_pending_outer_operator() {
        // 10 ÷ (5 − 3) = 5.
        let mut calculator = calc();
        press_digits(&mut calculator, "10");
        calculator.press(Key::Operator(Divide));
        calculator.press(OpenParen);
        press_digits(&mut calculator, "5");
        calculator.press(Key::Operator(Subtract));
        press_digits(&mut calculator, "3");
        calculator.press(CloseParen);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "5");
    }

    #[test]
    fn division_by_zero_inside_parens_is_an_error() {
        let mut calculator = calc();
        calculator.press(OpenParen);
        press_digits(&mut calculator, "1");
        calculator.press(Key::Operator(Divide));
        press_digits(&mut calculator, "0");
        calculator.press(CloseParen);
        assert!(calculator.is_error());
    }

    #[test]
    fn unmatched_close_paren_does_nothing() {
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(CloseParen);
        assert_eq!(calculator.display(), "5");
        assert_eq!(calculator.open_parens(), 0);
    }

    #[test]
    fn equals_appends_to_history_and_survives_all_clear() {
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        press_digits(&mut calculator, "3");
        calculator.press(Equals);
        assert_eq!(
            calculator.history(),
            [HistoryEntry {
                expression: "2+3".to_owned(),
                result: "5".to_owned(),
            }]
        );
        calculator.press(Clear);
        calculator.press(Clear);
        assert_eq!(calculator.history().len(), 1);
    }

    #[test]
    fn history_result_reloads_via_paste() {
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        press_digits(&mut calculator, "3");
        calculator.press(Equals);
        let entry = calculator.history()[0].clone();
        press_digits(&mut calculator, "9");
        assert!(calculator.paste(&entry.result));
        assert_eq!(calculator.display(), "5");
    }

    #[test]
    fn divide_by_zero_shows_error_and_a_digit_starts_over() {
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(Key::Operator(Divide));
        press_digits(&mut calculator, "0");
        calculator.press(Equals);
        assert!(calculator.is_error());
        assert_eq!(calculator.display(), ERROR_TEXT);
        press_digits(&mut calculator, "7");
        assert_eq!(calculator.display(), "7");
        assert!(!calculator.is_error());
    }

    #[test]
    fn backspace_and_clear_behave_like_basic() {
        let mut calculator = calc();
        press_digits(&mut calculator, "123");
        calculator.press(Backspace);
        assert_eq!(calculator.display(), "12");
        assert_eq!(calculator.clear_label(), ClearLabel::Clear);
        calculator.press(Clear);
        assert_eq!(calculator.display(), "0");
        assert_eq!(calculator.clear_label(), ClearLabel::AllClear);
    }

    #[test]
    fn percent_matches_basic_semantics() {
        let mut calculator = calc();
        press_digits(&mut calculator, "50");
        calculator.press(Key::Operator(Add));
        press_digits(&mut calculator, "10");
        calculator.press(Percent);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "55");
    }

    #[test]
    fn copy_text_strips_grouping_separators() {
        let mut calculator = calc();
        press_digits(&mut calculator, "1234567");
        assert_eq!(calculator.copy_text(), "1234567");
    }
}
