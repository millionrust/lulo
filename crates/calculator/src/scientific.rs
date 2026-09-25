//! The Scientific-mode calculator engine: pure state, no GPUI.
//!
//! Measured on the owner's Mac (macOS 26.2, 2026-09-25, AX tree read live
//! with System Events): **Scientific evaluates with real algebraic
//! precedence, unlike Basic mode.** `2 + 3 × 5 =` gives `17`, not `25` — the
//! display shows the whole pending formula (`2+3×5`) as it is typed, and
//! only "=" actually computes it, applying `×`/`÷` before `+`/`−` and
//! resolving parentheses innermost-first. This is a deliberate difference
//! from `crate::engine`'s Basic mode (which applies operators immediately,
//! left to right, with no precedence at all) — confirmed by pressing the
//! real keys and reading the display, not assumed.
//!
//! Unary functions (`x²`, `sin`, `1/x`, …) apply the moment they are
//! pressed — there is nothing left for `=` to decide about a function of a
//! single already-known value — but the *display* still shows the formula
//! text (`2²`, `sin(6)`, `(1÷5)`) rather than the number, until `=` (or the
//! next operator) moves on. Confirmed on the Mac: `5 → 1/x` shows `(1÷5)`,
//! not `0.2`.
//!
//! The measured 10-column keypad (`scientific_keypad.rs`) has almost every
//! function on its own permanent key — `x²`, `x³`, `xʸ`, `²√x`, `³√x`,
//! `ʸ√x` and `1/x` are all always visible, not folded into `2nd` pairs as
//! earlier guesses assumed. `2nd` only flips: `eˣ`↔`yˣ`, `10ˣ`↔`2ˣ`,
//! `ln`↔`logᵧ`, `log₁₀`↔`log₂`, and the three trig/hyperbolic-trig rows to
//! their inverses (`sin`↔`sin⁻¹`, …, `sinh`↔`sinh⁻¹`, …).
//!
//! A Rad/Deg toggle affects `sin`/`cos`/`tan` and their inverses (hyperbolic
//! functions are angle-mode independent, like every real calculator). The
//! Mac also shows a small persistent "Rad" label above the keypad whenever
//! radians is active, separate from the toggle key itself, which always
//! reads as the *other* mode (i.e. it reads "Rad" while in degrees, "Deg"
//! while in radians) — `view.rs` renders this indicator.
//!
//! `π`, `e`, `Rand` (a fresh random value each press) and `EE` (scientific
//! exponent entry, e.g. `1.5e10`) insert into the expression like a typed
//! number. A single memory register: `mc` clears it, `m+`/`m-` add/subtract
//! the current value, `mr` recalls it as a fresh entry. `(`/`)` nest to any
//! depth and are real tokens in the expression, evaluated by the same
//! precedence climb as everything else.
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

/// A binary operator, evaluated with real precedence at `=` (see the module
/// doc comment). `Power`/`Root`/`LogBase` bind tighter than `×`/`÷`, which
/// bind tighter than `+`/`−`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    /// `xʸ` (and `2nd`'s `yˣ`, which uses the same maths): the left operand
    /// raised to the right.
    Power,
    /// `ʸ√x`: the right-operand-th root of the left.
    Root,
    /// `2nd`'s `logᵧ`: log base *right* of *left*.
    LogBase,
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
            Self::LogBase => "logᵧ",
        }
    }

    /// Higher binds tighter. `Add`/`Subtract` < `Multiply`/`Divide` <
    /// `Power`/`Root`/`LogBase`, all left-associative.
    fn precedence(self) -> u8 {
        match self {
            Self::Add | Self::Subtract => 1,
            Self::Multiply | Self::Divide => 2,
            Self::Power | Self::Root | Self::LogBase => 3,
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
            Self::LogBase if left <= 0.0 || right <= 0.0 || right == 1.0 => return None,
            Self::LogBase => left.ln() / right.ln(),
        };
        value.is_finite().then_some(value)
    }
}

/// Every key on the Scientific keypad (`scientific_keypad::LAYOUT`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Digit(u8),
    Decimal,
    Operator(BinaryOp),
    Equals,
    Percent,
    ToggleSign,
    Clear,
    Backspace,
    Second,
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
    /// `x²`. Always present, not paired with `2nd`.
    Square,
    /// `x³`. Always present.
    Cube,
    /// `xʸ`. Always present; prompts for the exponent like a binary operator.
    Power,
    /// `²√x`. Always present.
    SquareRoot,
    /// `³√x`. Always present.
    CubeRoot,
    /// `ʸ√x`. Always present; prompts for the root degree.
    YRoot,
    /// `1/x`. Always present.
    Reciprocal,
    /// `x!`. Always present.
    Factorial,
    /// `eˣ` when `2nd` is off, `yˣ` (a second `xʸ`-shaped prompt) when on.
    ExpOrYPower,
    /// `10ˣ` when `2nd` is off, `2ˣ` when on.
    TenPowOrTwoPow,
    /// `ln` when `2nd` is off, `logᵧ` (prompts for the base) when on.
    LnOrLogY,
    /// `log₁₀` when `2nd` is off, `log₂` when on.
    Log10OrLog2,
    /// `sin`/`sin⁻¹` via `2nd`.
    Sin,
    /// `cos`/`cos⁻¹` via `2nd`.
    Cos,
    /// `tan`/`tan⁻¹` via `2nd`.
    Tan,
    /// `sinh`/`sinh⁻¹` via `2nd`. Angle-mode independent.
    Sinh,
    /// `cosh`/`cosh⁻¹` via `2nd`. Angle-mode independent.
    Cosh,
    /// `tanh`/`tanh⁻¹` via `2nd`. Angle-mode independent.
    Tanh,
}

/// One element of the expression being built since the last `=`/`AC`.
#[derive(Clone, Debug, PartialEq)]
enum Term {
    /// A resolved operand: a typed number, or the result of a function,
    /// constant or closed group. `display` is its formula text (`"5"`,
    /// `"sin(6)"`, `"2²"`, `"(1÷5)"`), which is what the display actually
    /// shows — the Mac keeps the formula visible, not the number, until
    /// something forces a computation.
    Value {
        value: f64,
        display: String,
    },
    Op(BinaryOp),
    Open,
    Close,
}

#[derive(Clone, Debug, Default)]
pub struct ScientificCalculator {
    /// The resting value: what the display falls back to when nothing is
    /// being typed and no expression is in progress (after `AC` or `=`).
    value: f64,
    /// The expression built so far, alternating `Value`/`Op`, with `Open`
    /// and `Close` for parenthesised groups. Empty means "just `value`".
    terms: Vec<Term>,
    /// The digits being typed for the operand at the tail of `terms`, not
    /// yet folded in.
    entry: Option<String>,
    /// A right operand exists for the pending operation. While false the
    /// last operator key stays lit, exactly like Basic.
    operand_ready: bool,
    entry_active: bool,
    /// The operator and right operand `=` repeats when pressed again with
    /// nothing new typed (`2+3==` → `8`), mirroring Basic.
    last: Option<(BinaryOp, f64)>,
    error: bool,
    /// The secondary line above the result: the just-evaluated expression.
    expression: String,
    /// `2nd`: flips the paired keys documented on `Key`.
    second: bool,
    angle: AngleMode,
    memory: f64,
    open_parens: usize,
    /// Set once a function, operator or paren has actually built up the
    /// expression, so a lone `=` afterwards (`terms.len() == 1`, e.g. right
    /// after `x²`) is logged to history like a real calculation — unlike a
    /// bare typed number followed by `=`, which the Mac treats as a no-op
    /// (`ac.equals_alone_does_nothing` in `engine.rs`'s Basic tests).
    has_operation: bool,
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
            Key::Operator(operator) => self.push_operator(operator),
            Key::Equals => self.equals(),
            Key::Percent => self.percent(),
            Key::ToggleSign => self.toggle_sign(),
            Key::Clear => self.clear(),
            Key::Backspace => self.backspace(),
            Key::Second => self.second = !self.second,
            Key::Pi => {
                self.insert_constant(std::f64::consts::PI, &format_value(std::f64::consts::PI))
            }
            Key::E => self.insert_constant(std::f64::consts::E, &format_value(std::f64::consts::E)),
            Key::Rand => {
                let value = random_unit();
                self.insert_constant(value, &format_value(value));
            }
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
            Key::MemoryRecall => self.insert_constant(self.memory, &format_value(self.memory)),
            Key::OpenParen => self.open_paren(),
            Key::CloseParen => self.close_paren(),
            Key::Square => self.apply_fn(|d| format!("{d}²"), |v| Some(v * v)),
            Key::Cube => self.apply_fn(|d| format!("{d}³"), |v| Some(v * v * v)),
            Key::Power => self.push_operator(BinaryOp::Power),
            Key::SquareRoot => {
                self.apply_fn(|d| format!("√({d})"), |v| (v >= 0.0).then(|| v.sqrt()))
            }
            Key::CubeRoot => self.apply_fn(|d| format!("∛({d})"), |v| Some(v.cbrt())),
            Key::YRoot => self.push_operator(BinaryOp::Root),
            Key::Reciprocal => {
                self.apply_fn(|d| format!("(1÷{d})"), |v| (v != 0.0).then(|| 1.0 / v))
            }
            Key::Factorial => self.apply_fn(|d| format!("{d}!"), factorial),
            Key::ExpOrYPower => {
                if self.second {
                    self.push_operator(BinaryOp::Power);
                } else {
                    self.apply_fn(|d| format!("eˣ({d})"), |v| Some(v.exp()));
                }
            }
            Key::TenPowOrTwoPow => {
                if self.second {
                    self.apply_fn(|d| format!("2ˣ({d})"), |v| Some(2f64.powf(v)));
                } else {
                    self.apply_fn(|d| format!("10ˣ({d})"), |v| Some(10f64.powf(v)));
                }
            }
            Key::LnOrLogY => {
                if self.second {
                    self.push_operator(BinaryOp::LogBase);
                } else {
                    self.apply_fn(|d| format!("ln({d})"), |v| (v > 0.0).then(|| v.ln()));
                }
            }
            Key::Log10OrLog2 => {
                if self.second {
                    self.apply_fn(|d| format!("log₂({d})"), |v| (v > 0.0).then(|| v.log2()));
                } else {
                    self.apply_fn(|d| format!("log₁₀({d})"), |v| (v > 0.0).then(|| v.log10()));
                }
            }
            Key::Sin => self.apply_trig("sin", Trig::Sin),
            Key::Cos => self.apply_trig("cos", Trig::Cos),
            Key::Tan => self.apply_trig("tan", Trig::Tan),
            Key::Sinh => self.apply_hyperbolic("sinh", Hyperbolic::Sinh),
            Key::Cosh => self.apply_hyperbolic("cosh", Hyperbolic::Cosh),
            Key::Tanh => self.apply_hyperbolic("tanh", Hyperbolic::Tanh),
        }
    }

    /// The main display text: the expression built so far, plus whatever is
    /// being typed. Falls back to the resting value when nothing is pending
    /// — the Mac's own behaviour, confirmed live (`display()` shows
    /// `"2+3×"`, `"2²"` or `"(1÷5)"` before `=`, not a computed number).
    pub fn display(&self) -> String {
        if self.error {
            return ERROR_TEXT.to_owned();
        }
        let mut text = terms_display(&self.terms);
        match &self.entry {
            Some(entry) => text.push_str(&format_scientific_entry(entry)),
            None if self.terms.is_empty() => return format_value(self.value),
            None => {}
        }
        text
    }

    /// The secondary expression line above the result. Empty when idle.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    pub fn highlighted_operator(&self) -> Option<BinaryOp> {
        if self.error || self.entry.is_some() {
            return None;
        }
        match self.terms.last() {
            Some(Term::Op(op)) => Some(*op),
            _ => None,
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
        self.open_parens
    }

    /// Completed calculations, oldest first.
    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// The value ⌘C puts on the clipboard.
    pub fn copy_text(&self) -> String {
        self.display().replace(',', "")
    }

    /// Paste a number as a fresh operand, or load a history result back in.
    pub fn paste(&mut self, text: &str) -> bool {
        let Some(value) = parse_number(text) else {
            return false;
        };
        self.insert_constant(value, &format_value(value));
        true
    }

    /// Peek at the operand currently being edited (entry in progress, or the
    /// value at the tail of `terms`), without consuming it. Used by memory
    /// and percent, which read the current value but do not replace it.
    fn current(&self) -> f64 {
        if let Some(entry) = &self.entry {
            return parse_current_entry(entry);
        }
        match self.terms.last() {
            Some(Term::Value { value, .. }) => *value,
            _ => self.value,
        }
    }

    /// Take the operand currently being edited, consuming it (the entry is
    /// cleared, or the trailing `Value` is popped) so a function or operator
    /// can replace it. Returns its value and formula text.
    fn take_current(&mut self) -> (f64, String) {
        if let Some(entry) = self.entry.take() {
            return (parse_current_entry(&entry), format_scientific_entry(&entry));
        }
        if matches!(self.terms.last(), Some(Term::Value { .. })) {
            if let Some(Term::Value { value, display }) = self.terms.pop() {
                return (value, display);
            }
        }
        (self.value, format_value(self.value))
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
    }

    /// Begin typing a fresh operand. If the tail of `terms` already holds a
    /// completed value with no operator after it (e.g. right after a
    /// function or `)`), a bare digit press discards it and starts over,
    /// the same way a digit after `=` starts a whole new calculation.
    fn start_entry(&mut self, text: &str) {
        if self.error {
            self.all_clear();
        }
        if matches!(self.terms.last(), Some(Term::Value { .. })) {
            self.terms.pop();
        }
        if self.terms.is_empty() {
            // Nothing left pending: this digit starts a whole new number, so
            // a later bare `=` should stay silent, not log a calculation.
            self.has_operation = false;
        }
        self.entry = Some(text.to_owned());
        self.operand_ready = true;
        self.entry_active = true;
    }

    /// `π`, `e`, `Rand`, `mr` and paste all replace the current operand with
    /// a fresh value, ready to be combined by the next operator.
    fn insert_constant(&mut self, value: f64, display: &str) {
        if self.error {
            self.all_clear();
        }
        self.entry = None;
        if matches!(self.terms.last(), Some(Term::Value { .. })) {
            self.terms.pop();
        }
        if self.terms.is_empty() {
            self.has_operation = false;
        }
        self.terms.push(Term::Value {
            value,
            display: display.to_owned(),
        });
        self.operand_ready = true;
        self.entry_active = true;
    }

    /// Apply a unary function to the current operand immediately, replacing
    /// it with the result — but keeping the formula (not the number) as the
    /// operand's display text until `=` moves on. `wrap` builds the formula
    /// text from the old operand's text; `compute` is the maths.
    fn apply_fn(
        &mut self,
        wrap: impl FnOnce(&str) -> String,
        compute: impl FnOnce(f64) -> Option<f64>,
    ) {
        if self.error {
            return;
        }
        let (value, text) = self.take_current();
        match compute(value).filter(|result| result.is_finite()) {
            Some(result) => {
                self.terms.push(Term::Value {
                    value: result,
                    display: wrap(&text),
                });
                self.operand_ready = true;
                self.entry_active = true;
                self.has_operation = true;
            }
            None => self.fail(),
        }
    }

    fn apply_trig(&mut self, name: &'static str, which: Trig) {
        let (angle, second) = (self.angle, self.second);
        self.apply_fn(
            move |d| {
                if second {
                    format!("{name}⁻¹({d})")
                } else {
                    format!("{name}({d})")
                }
            },
            move |value| trig(value, which, angle, second),
        );
    }

    fn apply_hyperbolic(&mut self, name: &'static str, which: Hyperbolic) {
        let second = self.second;
        self.apply_fn(
            move |d| {
                if second {
                    format!("{name}⁻¹({d})")
                } else {
                    format!("{name}({d})")
                }
            },
            move |value| hyperbolic(value, which, second),
        );
    }

    fn push_operator(&mut self, operator: BinaryOp) {
        if self.error {
            return;
        }
        if let Some(entry) = self.entry.take() {
            self.terms.push(Term::Value {
                value: parse_current_entry(&entry),
                display: format_scientific_entry(&entry),
            });
        } else if self.terms.is_empty() {
            self.terms.push(Term::Value {
                value: self.value,
                display: format_value(self.value),
            });
        } else if matches!(self.terms.last(), Some(Term::Op(_))) {
            // Pressing another operator swaps the pending one, like Basic.
            *self.terms.last_mut().expect("checked above") = Term::Op(operator);
            self.operand_ready = false;
            self.entry_active = false;
            return;
        }
        self.terms.push(Term::Op(operator));
        self.operand_ready = false;
        self.entry_active = false;
        self.has_operation = true;
    }

    fn equals(&mut self) {
        if self.error {
            return;
        }
        if let Some(entry) = self.entry.take() {
            self.terms.push(Term::Value {
                value: parse_current_entry(&entry),
                display: format_scientific_entry(&entry),
            });
        }
        if self.terms.is_empty() {
            // Nothing new since the last `=`: repeat the last operation, if
            // there was one (`2+3==` → `8`), exactly like Basic.
            if let Some((operator, right)) = self.last {
                match operator.apply(self.value, right).filter(|v| v.is_finite()) {
                    Some(result) => {
                        self.expression = format!(
                            "{}{}{}",
                            format_value(self.value),
                            operator.symbol(),
                            format_value(right)
                        );
                        self.history.push(HistoryEntry {
                            expression: self.expression.clone(),
                            result: format_value(result),
                        });
                        self.value = result;
                    }
                    None => return self.fail(),
                }
            }
            self.entry_active = false;
            self.has_operation = false;
            return;
        }
        if self.terms.len() == 1 && matches!(self.terms.first(), Some(Term::Value { .. })) {
            let Some(Term::Value { value, display }) = self.terms.pop() else {
                unreachable!("checked above that terms == [Value(..)]")
            };
            // A bare typed number with `=`: reuse the last operation if one
            // exists (`2+3=10=` → `13`); otherwise a single value that came
            // from a function (`5 → x²` shows "5²") is still a completed
            // calculation and gets logged, but a plain typed number just
            // commits silently, matching Basic's `equals_alone_does_nothing`.
            if let Some((operator, right)) = self.last {
                match operator.apply(value, right).filter(|v| v.is_finite()) {
                    Some(result) => {
                        self.expression = format!(
                            "{}{}{}",
                            format_value(value),
                            operator.symbol(),
                            format_value(right)
                        );
                        self.history.push(HistoryEntry {
                            expression: self.expression.clone(),
                            result: format_value(result),
                        });
                        self.value = result;
                    }
                    None => return self.fail(),
                }
            } else if self.has_operation {
                self.history.push(HistoryEntry {
                    expression: display.clone(),
                    result: format_value(value),
                });
                self.expression = display;
                self.value = value;
            } else {
                self.value = value;
            }
            self.open_parens = 0;
            self.entry_active = false;
            self.has_operation = false;
            return;
        }
        match eval_terms(&self.terms).filter(|v| v.is_finite()) {
            Some(result) => {
                self.last = last_top_level_op(&self.terms);
                self.expression = terms_display(&self.terms);
                self.history.push(HistoryEntry {
                    expression: self.expression.clone(),
                    result: format_value(result),
                });
                self.value = result;
                self.terms.clear();
                self.open_parens = 0;
                self.operand_ready = false;
                self.entry_active = false;
                self.has_operation = false;
            }
            None => self.fail(),
        }
    }

    fn percent(&mut self) {
        if self.error {
            return;
        }
        let current = self.current();
        let scale = match self.terms.last() {
            Some(Term::Op(BinaryOp::Add | BinaryOp::Subtract)) => {
                match self.terms.iter().rev().nth(1) {
                    Some(Term::Value { value: left, .. }) => left / 100.0,
                    _ => 1.0 / 100.0,
                }
            }
            _ => 1.0 / 100.0,
        };
        let value = current * scale;
        self.entry = None;
        if matches!(self.terms.last(), Some(Term::Value { .. })) {
            self.terms.pop();
        }
        self.terms.push(Term::Value {
            value,
            display: format_value(value),
        });
        self.operand_ready = true;
        self.entry_active = true;
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
        } else if let Some(Term::Value { value, display }) = self.terms.last_mut() {
            *value = -*value;
            *display = format_value(*value);
        } else if (!self.operand_ready && matches!(self.terms.last(), Some(Term::Op(_))))
            || (self.terms.is_empty() && self.value == 0.0)
        {
            self.entry = Some("-0".to_owned());
            self.operand_ready = true;
            self.entry_active = true;
        } else {
            self.value = -self.value;
            self.operand_ready = true;
            self.entry_active = true;
        }
    }

    fn clear(&mut self) {
        match self.clear_label() {
            ClearLabel::Clear => self.clear_entry(),
            ClearLabel::AllClear => self.all_clear(),
        }
    }

    fn clear_entry(&mut self) {
        if self.entry.is_some() {
            self.entry = None;
        } else if matches!(self.terms.last(), Some(Term::Value { .. })) {
            self.terms.pop();
        }
        self.value = 0.0;
        self.operand_ready = false;
        self.entry_active = false;
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
                let value = self.current();
                if matches!(self.terms.last(), Some(Term::Value { .. })) {
                    self.terms.pop();
                }
                let mut entry = plain_number(value);
                entry.push('e');
                self.entry = Some(entry);
                self.operand_ready = true;
                self.entry_active = true;
            }
        }
    }

    /// `(`: start a fresh parenthesised group, only in a position where a
    /// new operand may begin (after an operator, another `(`, or at the very
    /// start).
    fn open_paren(&mut self) {
        if self.error {
            self.all_clear();
        }
        if let Some(entry) = self.entry.take() {
            self.terms.push(Term::Value {
                value: parse_current_entry(&entry),
                display: format_scientific_entry(&entry),
            });
        }
        let valid =
            self.terms.is_empty() || matches!(self.terms.last(), Some(Term::Op(_) | Term::Open));
        if valid {
            self.terms.push(Term::Open);
            self.open_parens += 1;
            self.operand_ready = false;
            self.entry_active = false;
        }
    }

    /// `)`: close the innermost open group, once it holds a complete value.
    fn close_paren(&mut self) {
        if self.error || self.open_parens == 0 {
            return;
        }
        if let Some(entry) = self.entry.take() {
            self.terms.push(Term::Value {
                value: parse_current_entry(&entry),
                display: format_scientific_entry(&entry),
            });
        }
        if matches!(self.terms.last(), Some(Term::Value { .. } | Term::Close)) {
            self.terms.push(Term::Close);
            self.open_parens -= 1;
            self.operand_ready = true;
            self.entry_active = true;
        }
    }
}

#[derive(Clone, Copy)]
enum Trig {
    Sin,
    Cos,
    Tan,
}

fn trig(value: f64, which: Trig, angle: AngleMode, inverse: bool) -> Option<f64> {
    if !inverse {
        let radians = match angle {
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
    Some(match angle {
        AngleMode::Degrees => radians.to_degrees(),
        AngleMode::Radians => radians,
    })
}

#[derive(Clone, Copy)]
enum Hyperbolic {
    Sinh,
    Cosh,
    Tanh,
}

/// Hyperbolic trig and its inverses are angle-mode independent everywhere,
/// including on the Mac.
fn hyperbolic(value: f64, which: Hyperbolic, inverse: bool) -> Option<f64> {
    Some(match (which, inverse) {
        (Hyperbolic::Sinh, false) => value.sinh(),
        (Hyperbolic::Sinh, true) => value.asinh(),
        (Hyperbolic::Cosh, false) => value.cosh(),
        (Hyperbolic::Cosh, true) => (value >= 1.0).then(|| value.acosh())?,
        (Hyperbolic::Tanh, false) => value.tanh(),
        (Hyperbolic::Tanh, true) => (-1.0..1.0).contains(&value).then(|| value.atanh())?,
    })
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

/// Parse an in-progress entry (which may have a trailing/empty exponent) to
/// its numeric value, for evaluation rather than display.
fn parse_current_entry(entry: &str) -> f64 {
    let text = entry
        .strip_suffix("e-")
        .or_else(|| entry.strip_suffix('e'))
        .unwrap_or(entry);
    text.parse::<f64>().unwrap_or(0.0)
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

/// Render a finished (or in-progress-but-committed) expression as the Mac
/// shows it: every value's formula text and every operator/paren symbol,
/// concatenated in order.
fn terms_display(terms: &[Term]) -> String {
    let mut text = String::new();
    for term in terms {
        match term {
            Term::Value { display, .. } => text.push_str(display),
            Term::Op(op) => text.push_str(op.symbol()),
            Term::Open => text.push('('),
            Term::Close => text.push(')'),
        }
    }
    text
}

/// Evaluate a complete expression with real operator precedence (see the
/// module doc comment). Returns `None` if it is malformed (a dangling
/// operator, unmatched paren) or a step is undefined (division by zero,
/// domain errors already having failed earlier at `apply_fn` time).
fn eval_terms(terms: &[Term]) -> Option<f64> {
    let mut pos = 0;
    let value = eval_expr(terms, &mut pos, 1)?;
    (pos == terms.len()).then_some(value)
}

fn eval_expr(terms: &[Term], pos: &mut usize, min_precedence: u8) -> Option<f64> {
    let mut left = eval_atom(terms, pos)?;
    while let Some(Term::Op(operator)) = terms.get(*pos) {
        let operator = *operator;
        let precedence = operator.precedence();
        if precedence < min_precedence {
            break;
        }
        *pos += 1;
        let right = eval_expr(terms, pos, precedence + 1)?;
        left = operator.apply(left, right)?;
    }
    Some(left)
}

fn eval_atom(terms: &[Term], pos: &mut usize) -> Option<f64> {
    match terms.get(*pos)? {
        Term::Value { value, .. } => {
            *pos += 1;
            Some(*value)
        }
        Term::Open => {
            *pos += 1;
            let value = eval_expr(terms, pos, 1)?;
            if matches!(terms.get(*pos), Some(Term::Close)) {
                *pos += 1;
            }
            Some(value)
        }
        _ => None,
    }
}

/// The last operator applied at the top level (outside every paren), with
/// the value of the operand right after it — what a bare `=` repeats. `None`
/// when the last top-level element after an operator was itself a
/// parenthesised group rather than a plain value (rare enough to just not
/// support repeating).
fn last_top_level_op(terms: &[Term]) -> Option<(BinaryOp, f64)> {
    let mut depth = 0i32;
    for index in (0..terms.len()).rev() {
        match &terms[index] {
            Term::Close => depth += 1,
            Term::Open => depth -= 1,
            Term::Op(op) if depth == 0 => {
                return match terms.get(index + 1) {
                    Some(Term::Value { value, .. }) => Some((*op, *value)),
                    _ => None,
                };
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use BinaryOp::{Add, Divide, Multiply, Subtract};
    use Key::{
        Backspace, Clear, CloseParen, Decimal, Digit, Ee, Equals, MemoryAdd, MemoryClear,
        MemoryRecall, MemorySubtract, OpenParen, Percent, RadDeg, Rand, Second, ToggleSign,
    };

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
    fn multiplication_binds_tighter_than_addition_unlike_basic_mode() {
        // Measured on the Mac: 2 + 3 × 5 = shows "2+3×5" while typing, and
        // computes 17 (not 25) once "=" is pressed.
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        press_digits(&mut calculator, "3");
        calculator.press(Key::Operator(Multiply));
        press_digits(&mut calculator, "5");
        assert_eq!(calculator.display(), "2+3×5");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "17");
    }

    #[test]
    fn division_binds_tighter_than_subtraction() {
        // 10 − 8 ÷ 4 = 10 − 2 = 8.
        let mut calculator = calc();
        press_digits(&mut calculator, "10");
        calculator.press(Key::Operator(Subtract));
        press_digits(&mut calculator, "8");
        calculator.press(Key::Operator(Divide));
        press_digits(&mut calculator, "4");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "8");
    }

    #[test]
    fn pressing_another_operator_replaces_the_pending_one() {
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(Key::Operator(Add));
        calculator.press(Key::Operator(Multiply));
        assert_eq!(calculator.highlighted_operator(), Some(Multiply));
        press_digits(&mut calculator, "2");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "10");
    }

    #[test]
    fn square_shows_the_formula_until_equals_computes_it() {
        // Measured on the Mac: 2, x² shows "2²"; = then shows "4".
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Square);
        assert_eq!(calculator.display(), "2²");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "4");
        assert_eq!(
            calculator.history(),
            [HistoryEntry {
                expression: "2²".to_owned(),
                result: "4".to_owned(),
            }]
        );
    }

    #[test]
    fn cube_and_roots_are_always_present_not_toggled_by_second() {
        let mut calculator = calc();
        press_digits(&mut calculator, "3");
        calculator.press(Key::Cube);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "27");

        let mut calculator = calc();
        press_digits(&mut calculator, "27");
        calculator.press(Key::CubeRoot);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "3");

        let mut calculator = calc();
        press_digits(&mut calculator, "25");
        calculator.press(Key::SquareRoot);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "5");
    }

    #[test]
    fn reciprocal_shows_a_parenthesised_division_like_the_mac() {
        // Measured: 5, 1/x shows "(1÷5)" before "=", then "0.2".
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(Key::Reciprocal);
        assert_eq!(calculator.display(), "(1÷5)");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "0.2");

        let mut calculator = calc();
        calculator.press(Key::Reciprocal);
        assert!(calculator.is_error());
    }

    #[test]
    fn power_and_y_root_prompt_for_a_second_operand() {
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Power);
        press_digits(&mut calculator, "10");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1,024");

        let mut calculator = calc();
        press_digits(&mut calculator, "8");
        calculator.press(Key::YRoot);
        press_digits(&mut calculator, "3");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "2");
    }

    #[test]
    fn root_of_a_negative_base_is_real_for_an_odd_root_only() {
        let mut calculator = calc();
        press_digits(&mut calculator, "8");
        calculator.press(ToggleSign);
        calculator.press(Key::YRoot);
        press_digits(&mut calculator, "3");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "-2");

        let mut calculator = calc();
        press_digits(&mut calculator, "8");
        calculator.press(ToggleSign);
        calculator.press(Key::YRoot);
        press_digits(&mut calculator, "2");
        calculator.press(Equals);
        assert!(calculator.is_error());
    }

    #[test]
    fn factorial_of_zero_is_one_and_negative_is_an_error() {
        let mut calculator = calc();
        calculator.press(Key::Factorial);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1");
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(Key::Factorial);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "120");
        let mut calculator = calc();
        press_digits(&mut calculator, "3");
        calculator.press(ToggleSign);
        calculator.press(Key::Factorial);
        assert!(calculator.is_error());
        let mut calculator = calc();
        press_digits(&mut calculator, "2.5");
        calculator.press(Key::Factorial);
        assert!(calculator.is_error());
    }

    #[test]
    fn exp_and_ten_pow_toggle_to_y_power_and_two_pow_with_second() {
        let mut calculator = calc();
        calculator.press(Key::ExpOrYPower);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1");

        let mut calculator = calc();
        press_digits(&mut calculator, "3");
        calculator.press(Second);
        calculator.press(Key::ExpOrYPower);
        press_digits(&mut calculator, "2");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "9");

        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::TenPowOrTwoPow);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "100");

        let mut calculator = calc();
        press_digits(&mut calculator, "3");
        calculator.press(Second);
        calculator.press(Key::TenPowOrTwoPow);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "8");
    }

    #[test]
    fn ln_and_log10_toggle_to_log_base_y_and_log2_with_second() {
        let mut calculator = calc();
        press_digits(&mut calculator, "100");
        calculator.press(Key::Log10OrLog2);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "2");

        let mut calculator = calc();
        press_digits(&mut calculator, "8");
        calculator.press(Second);
        calculator.press(Key::Log10OrLog2);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "3");

        let mut calculator = calc();
        press_digits(&mut calculator, "8");
        calculator.press(Second);
        calculator.press(Key::LnOrLogY);
        press_digits(&mut calculator, "2");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "3");
    }

    #[test]
    fn log_of_zero_or_negative_is_an_error() {
        let mut calculator = calc();
        calculator.press(Key::Log10OrLog2);
        assert!(calculator.is_error());
        let mut calculator = calc();
        calculator.press(Key::LnOrLogY);
        assert!(calculator.is_error());
    }

    #[test]
    fn sin_cos_tan_use_degrees_by_default_and_show_the_formula_first() {
        let mut calculator = calc();
        press_digits(&mut calculator, "90");
        calculator.press(Key::Sin);
        assert_eq!(calculator.display(), "sin(90)");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1");

        let mut calculator = calc();
        press_digits(&mut calculator, "0");
        calculator.press(Key::Cos);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1");

        let mut calculator = calc();
        press_digits(&mut calculator, "45");
        calculator.press(Key::Tan);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1");
    }

    #[test]
    fn rad_deg_toggle_changes_the_trig_result() {
        let mut degrees = calc();
        press_digits(&mut degrees, "90");
        degrees.press(Key::Sin);
        degrees.press(Equals);

        let mut radians = calc();
        radians.press(RadDeg);
        press_digits(&mut radians, "90");
        radians.press(Key::Sin);
        radians.press(Equals);

        assert_eq!(radians.angle_mode(), AngleMode::Radians);
        assert_ne!(degrees.display(), radians.display());

        let mut half_pi_radians = calc();
        half_pi_radians.press(RadDeg);
        press_digits(&mut half_pi_radians, "1.5707963");
        half_pi_radians.press(Key::Sin);
        half_pi_radians.press(Equals);
        assert_eq!(half_pi_radians.display(), "1");
    }

    #[test]
    fn inverse_trig_toggles_with_second_and_respects_domain() {
        let mut calculator = calc();
        press_digits(&mut calculator, "1");
        calculator.press(Second);
        calculator.press(Key::Sin);
        assert_eq!(calculator.display(), "sin⁻¹(1)");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "90");

        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Second);
        calculator.press(Key::Sin);
        assert!(calculator.is_error());

        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Second);
        calculator.press(Key::Cos);
        assert!(calculator.is_error());
    }

    #[test]
    fn hyperbolic_trig_and_inverses_are_not_affected_by_rad_deg() {
        let mut degrees = calc();
        press_digits(&mut degrees, "1");
        degrees.press(Key::Sinh);
        degrees.press(Equals);

        let mut radians = calc();
        radians.press(RadDeg);
        press_digits(&mut radians, "1");
        radians.press(Key::Sinh);
        radians.press(Equals);
        assert_eq!(degrees.display(), radians.display());

        let mut calculator = calc();
        press_digits(&mut calculator, "1");
        calculator.press(Key::Cosh);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "1.54308063");

        // acosh(1) = 0 exactly, unlike acosh(cosh(1)) which would round-trip
        // through a truncated 9-digit display and land a floating-point
        // hair off 1.
        let mut calculator = calc();
        press_digits(&mut calculator, "1");
        calculator.press(Second);
        calculator.press(Key::Cosh);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "0");

        let mut calculator = calc();
        press_digits(&mut calculator, "0.5");
        calculator.press(Second);
        calculator.press(Key::Tanh);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "0.549306144");

        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Second);
        calculator.press(Key::Tanh);
        assert!(calculator.is_error());
        let mut calculator = calc();
        press_digits(&mut calculator, "0.5");
        calculator.press(Second);
        calculator.press(Key::Cosh);
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
    fn parentheses_group_a_sub_calculation_and_take_real_precedence_into_account() {
        // 2 + (3 × 4) = 14; without the parens 2+3×4 would already be 14
        // too (× binds tighter than + regardless), so also check a case
        // where the parens change the answer: (2 + 3) × 4 = 20.
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
        calculator.press(Equals);
        assert_eq!(calculator.display(), "14");

        let mut calculator = calc();
        calculator.press(OpenParen);
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        press_digits(&mut calculator, "3");
        calculator.press(CloseParen);
        calculator.press(Key::Operator(Multiply));
        press_digits(&mut calculator, "4");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "20");
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
        calculator.press(Key::Operator(Subtract));
        press_digits(&mut calculator, "1");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "13");
    }

    #[test]
    fn division_by_zero_inside_parens_is_an_error() {
        let mut calculator = calc();
        calculator.press(OpenParen);
        press_digits(&mut calculator, "1");
        calculator.press(Key::Operator(Divide));
        press_digits(&mut calculator, "0");
        calculator.press(CloseParen);
        calculator.press(Equals);
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
    fn repeated_equals_repeats_the_last_operation() {
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        press_digits(&mut calculator, "3");
        calculator.press(Equals);
        calculator.press(Equals);
        assert_eq!(calculator.display(), "8");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "11");
    }

    #[test]
    fn a_new_number_then_equals_applies_the_last_operation() {
        let mut calculator = calc();
        press_digits(&mut calculator, "2");
        calculator.press(Key::Operator(Add));
        press_digits(&mut calculator, "3");
        calculator.press(Equals);
        press_digits(&mut calculator, "10");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "13");
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

        let mut calculator = calc();
        press_digits(&mut calculator, "50");
        calculator.press(Percent);
        assert_eq!(calculator.display(), "0.5");
    }

    #[test]
    fn copy_text_strips_grouping_separators() {
        let mut calculator = calc();
        press_digits(&mut calculator, "1234567");
        assert_eq!(calculator.copy_text(), "1234567");
    }

    #[test]
    fn a_fresh_digit_after_a_function_result_starts_a_new_number() {
        let mut calculator = calc();
        press_digits(&mut calculator, "5");
        calculator.press(Key::Square);
        assert_eq!(calculator.display(), "5²");
        press_digits(&mut calculator, "9");
        assert_eq!(calculator.display(), "9");
    }
}
