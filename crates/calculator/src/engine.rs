//! The Basic-mode calculator engine: pure state, no GPUI.
//!
//! It follows macOS Calculator's Basic mode:
//!
//! - Operators evaluate straight away, so `2 + 3 ×` shows `5` before the `×`
//!   applies. There is no operator precedence in Basic mode.
//! - The operator key stays lit until the next operand starts. Pressing a
//!   different operator while it is lit swaps the pending operator.
//! - `=` with no second operand reuses the first (`5 + =` gives `10`).
//!   Repeating `=` repeats the last operation (`2 + 3 = =` gives `8`), and a
//!   freshly typed number followed by `=` applies it again (`10 =` gives `13`).
//! - The clear key reads `C` while there is an entry to clear. `C` clears only
//!   that entry and keeps the pending operation. `AC` clears everything.
//! - `%` divides by 100 unless the pending operation is `+` or `−`. Then it
//!   takes that percentage of the first operand (`50 + 10 %` shows `5`).
//! - The display holds at most 9 significant digits. Larger or smaller values
//!   switch to scientific notation (`1e9`, `1.2345679e-12`).
//! - Division by zero, or any result that is not finite, shows `Error`. The
//!   next digit starts over.

/// Most digits the user can type into one entry, and most significant digits
/// the display shows, like macOS.
pub const MAX_DIGITS: usize = 9;

/// Most significant digits in a scientific-notation mantissa. Together with
/// the exponent this keeps long results within the display.
const MAX_SCIENTIFIC_DIGITS: usize = 8;

/// The text shown for a failed calculation.
pub const ERROR_TEXT: &str = "Error";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl Operator {
    /// The glyph macOS draws on the key and in the expression line.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "−",
            Self::Multiply => "×",
            Self::Divide => "÷",
        }
    }

    fn apply(self, left: f64, right: f64) -> Option<f64> {
        let value = match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide if right == 0.0 => return None,
            Self::Divide => left / right,
        };
        value.is_finite().then_some(value)
    }
}

/// Which label the clear key shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClearLabel {
    /// `AC`: all clear.
    AllClear,
    /// `C`: clear the current entry only.
    Clear,
}

impl ClearLabel {
    pub fn text(self) -> &'static str {
        match self {
            Self::AllClear => "AC",
            Self::Clear => "C",
        }
    }
}

/// Every key on the Basic keypad.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Digit(u8),
    Decimal,
    Operator(Operator),
    Equals,
    Percent,
    ToggleSign,
    Clear,
    Backspace,
}

#[derive(Clone, Debug, Default)]
pub struct Calculator {
    /// The value on the display when the user is not typing.
    value: f64,
    /// The raw digits the user is typing, such as `-12.50`.
    entry: Option<String>,
    /// The left operand of the pending operation.
    accumulator: Option<f64>,
    pending: Option<Operator>,
    /// A right operand exists for the pending operation (typed, pasted, or
    /// produced by `%` or `±`). While false the operator key stays lit.
    operand_ready: bool,
    /// The clear key reads `C` rather than `AC`.
    entry_active: bool,
    /// The operation `=` repeats.
    last: Option<(Operator, f64)>,
    error: bool,
    /// The secondary line above the result, such as `3.66+3.59`.
    expression: String,
}

impl Calculator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one key press.
    pub fn press(&mut self, key: Key) {
        match key {
            Key::Digit(digit) => self.digit(digit),
            Key::Decimal => self.decimal(),
            Key::Operator(operator) => self.operator(operator),
            Key::Equals => self.equals(),
            Key::Percent => self.percent(),
            Key::ToggleSign => self.toggle_sign(),
            Key::Clear => self.clear(),
            Key::Backspace => self.backspace(),
        }
    }

    /// The main display text, grouped and rounded for display.
    pub fn display(&self) -> String {
        if self.error {
            return ERROR_TEXT.to_owned();
        }
        match &self.entry {
            Some(entry) => format_entry(entry),
            None => format_value(self.value),
        }
    }

    /// The secondary expression line above the result. Empty when idle.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// The operator whose key should be highlighted, if any.
    pub fn highlighted_operator(&self) -> Option<Operator> {
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

    /// The value ⌘C puts on the clipboard: the display without grouping
    /// separators, so it pastes cleanly into other apps.
    pub fn copy_text(&self) -> String {
        self.display().replace(',', "")
    }

    /// Paste a number as the current entry. Returns false and changes nothing
    /// when the text is not a number.
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
        match &self.entry {
            Some(entry) => entry.parse::<f64>().unwrap_or(0.0),
            None => self.value,
        }
    }

    /// Replace an in-progress entry with its value.
    fn commit_entry(&mut self) {
        if self.entry.is_some() {
            self.value = self.current();
            self.entry = None;
        }
    }

    fn fail(&mut self) {
        *self = Self {
            error: true,
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
                if entry_digits(entry) >= MAX_DIGITS {
                    return;
                }
                let unsigned = entry.trim_start_matches('-');
                if unsigned == "0" {
                    entry.pop();
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
                if !entry.contains('.') && entry_digits(entry) < MAX_DIGITS {
                    entry.push('.');
                }
            }
            _ => self.start_entry("0."),
        }
        self.update_expression();
    }

    fn operator(&mut self, operator: Operator) {
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
            (Some(Operator::Add | Operator::Subtract), Some(left)) => left * current / 100.0,
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
            match entry.strip_prefix('-') {
                Some(unsigned) => *entry = unsigned.to_owned(),
                None => entry.insert(0, '-'),
            }
        } else if (!self.operand_ready && self.pending.is_some()) || self.value == 0.0 {
            // macOS starts a new "-0" entry rather than negating nothing.
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
        *self = Self::default();
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
        let Some((left, operator)) = self.accumulator.zip(self.pending) else {
            return;
        };
        self.expression = format!("{}{}", format_value(left), operator.symbol());
        if self.operand_ready {
            let right = match &self.entry {
                Some(entry) => format_entry(entry),
                None => format_value(self.value),
            };
            self.expression.push_str(&right);
        }
    }
}

/// Count the digits in a typed entry, ignoring a lone leading `0` before the
/// decimal point, so `0.123456789` still accepts nine decimals.
fn entry_digits(entry: &str) -> usize {
    let unsigned = entry.trim_start_matches('-');
    let unsigned = unsigned.strip_prefix("0.").unwrap_or(unsigned);
    unsigned.chars().filter(char::is_ascii_digit).count()
}

/// Group a typed entry, keeping the user's trailing `.` and zeros.
pub fn format_entry(entry: &str) -> String {
    let (sign, unsigned) = match entry.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", entry),
    };
    let (integer, fraction) = match unsigned.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (unsigned, None),
    };
    let integer = if integer.is_empty() { "0" } else { integer };
    let mut text = format!("{sign}{}", group_thousands(integer));
    if let Some(fraction) = fraction {
        text.push('.');
        text.push_str(fraction);
    }
    text
}

/// Format a computed value with at most 9 significant digits, thousands
/// separators, and scientific notation once it no longer fits.
pub fn format_value(value: f64) -> String {
    if !value.is_finite() {
        return ERROR_TEXT.to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let rounded = format!("{:.*e}", MAX_DIGITS - 1, value);
    let exponent = rounded
        .split_once('e')
        .and_then(|(_, exponent)| exponent.parse::<i32>().ok())
        .unwrap_or(0);
    if !(-8..MAX_DIGITS as i32).contains(&exponent) {
        return format_scientific(value);
    }
    // Leading zeros after the point count towards the nine digits, so
    // 1 ÷ 3 shows 0.333333333 and 0.000012345 keeps its tail.
    let decimals = if exponent >= 0 {
        MAX_DIGITS - 1 - exponent as usize
    } else {
        MAX_DIGITS
    };
    let fixed = format!("{value:.decimals$}");
    let fixed = trim_fraction(&fixed);
    if fixed == "0" || fixed == "-0" {
        return format_scientific(value);
    }
    format_entry(fixed)
}

fn format_scientific(value: f64) -> String {
    let text = format!("{:.*e}", MAX_SCIENTIFIC_DIGITS - 1, value);
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text;
    };
    format!("{}e{exponent}", trim_fraction(mantissa))
}

fn trim_fraction(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

fn group_thousands(digits: &str) -> String {
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Parse pasted text: surrounding whitespace, thousands separators and a
/// typographic minus are accepted. Anything else is rejected.
pub fn parse_number(text: &str) -> Option<f64> {
    let cleaned = text
        .trim()
        .replace([',', ' ', '\u{a0}'], "")
        .replace('\u{2212}', "-");
    if cleaned.is_empty()
        || !cleaned.chars().all(|character| {
            character.is_ascii_digit() || matches!(character, '-' | '+' | '.' | 'e' | 'E')
        })
    {
        return None;
    }
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// The display font size that fits `text` into `width` points, shrinking
/// from `max_size` like macOS does as digits are added. Digit and sign glyphs
/// are about 0.56 em wide in the display font, separators about 0.27 em.
pub fn fitted_font_size(text: &str, width: f32, max_size: f32, min_size: f32) -> f32 {
    let ems: f32 = text
        .chars()
        .map(|character| match character {
            ',' | '.' => 0.27,
            'e' => 0.55,
            _ => 0.56,
        })
        .sum();
    if ems <= 0.0 {
        return max_size;
    }
    (width / ems).clamp(min_size, max_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    use Key::{Backspace, Clear, Decimal, Digit, Equals, Percent, ToggleSign};
    use Operator::{Add, Divide, Multiply, Subtract};

    fn run(keys: &str) -> Calculator {
        let mut calculator = Calculator::new();
        for key in keys.chars() {
            let key = match key {
                '0'..='9' => Digit(key as u8 - b'0'),
                '.' => Decimal,
                '+' => Key::Operator(Add),
                '-' => Key::Operator(Subtract),
                '*' => Key::Operator(Multiply),
                '/' => Key::Operator(Divide),
                '=' => Equals,
                '%' => Percent,
                'n' => ToggleSign,
                'c' => Clear,
                'b' => Backspace,
                ' ' => continue,
                other => panic!("unknown test key {other}"),
            };
            calculator.press(key);
        }
        calculator
    }

    fn shows(keys: &str) -> String {
        run(keys).display()
    }

    #[test]
    fn starts_at_zero_with_all_clear() {
        let calculator = Calculator::new();
        assert_eq!(calculator.display(), "0");
        assert_eq!(calculator.clear_label(), ClearLabel::AllClear);
        assert_eq!(calculator.expression(), "");
        assert_eq!(calculator.highlighted_operator(), None);
    }

    #[test]
    fn digits_accumulate_and_drop_leading_zeros() {
        assert_eq!(shows("0007"), "7");
        assert_eq!(shows("123"), "123");
        assert_eq!(shows("0"), "0");
    }

    #[test]
    fn entry_is_grouped_with_thousands_separators() {
        assert_eq!(shows("1234"), "1,234");
        assert_eq!(shows("1234567"), "1,234,567");
        assert_eq!(shows("123456789"), "123,456,789");
    }

    #[test]
    fn entry_stops_at_nine_digits() {
        assert_eq!(shows("1234567890"), "123,456,789");
        assert_eq!(shows("0.1234567891"), "0.123456789");
        assert_eq!(shows("12345.67891"), "12,345.6789");
    }

    #[test]
    fn decimal_input_keeps_trailing_point_and_zeros() {
        assert_eq!(shows("."), "0.");
        assert_eq!(shows("1."), "1.");
        assert_eq!(shows("1.50"), "1.50");
        assert_eq!(shows("1..5"), "1.5");
        assert_eq!(shows("1.5.2"), "1.52");
        assert_eq!(shows("0.05"), "0.05");
    }

    #[test]
    fn basic_operations() {
        assert_eq!(shows("2+3="), "5");
        assert_eq!(shows("9-12="), "-3");
        assert_eq!(shows("6*7="), "42");
        assert_eq!(shows("8/2="), "4");
        assert_eq!(shows("3.66+3.59="), "7.25");
    }

    #[test]
    fn floating_point_noise_is_hidden() {
        assert_eq!(shows(".1+.2="), "0.3");
        assert_eq!(shows("1/3="), "0.333333333");
        assert_eq!(shows("2/3="), "0.666666667");
        assert_eq!(shows("1/3*3="), "1");
    }

    #[test]
    fn operators_evaluate_immediately_without_precedence() {
        let calculator = run("2+3*");
        assert_eq!(calculator.display(), "5");
        assert_eq!(calculator.highlighted_operator(), Some(Multiply));
        assert_eq!(shows("2+3*4="), "20");
        assert_eq!(shows("10-2-3="), "5");
    }

    #[test]
    fn pressing_another_operator_replaces_the_pending_one() {
        let calculator = run("5+*");
        assert_eq!(calculator.highlighted_operator(), Some(Multiply));
        assert_eq!(calculator.display(), "5");
        assert_eq!(shows("5+*2="), "10");
        assert_eq!(shows("5+-*/2="), "2.5");
    }

    #[test]
    fn operator_highlight_clears_once_the_operand_starts() {
        let mut calculator = run("7+");
        assert_eq!(calculator.highlighted_operator(), Some(Add));
        calculator.press(Digit(1));
        assert_eq!(calculator.highlighted_operator(), None);
        calculator.press(Equals);
        assert_eq!(calculator.highlighted_operator(), None);
    }

    #[test]
    fn equals_without_second_operand_reuses_the_first() {
        assert_eq!(shows("5+="), "10");
        assert_eq!(shows("5*="), "25");
    }

    #[test]
    fn repeated_equals_repeats_the_last_operation() {
        assert_eq!(shows("2+3=="), "8");
        assert_eq!(shows("2+3==="), "11");
        assert_eq!(shows("10-1==="), "7");
        assert_eq!(shows("2*3=="), "18");
        assert_eq!(shows("81/3=="), "9");
    }

    #[test]
    fn a_new_number_then_equals_applies_the_last_operation() {
        assert_eq!(shows("2+3=10="), "13");
    }

    #[test]
    fn equals_alone_does_nothing() {
        assert_eq!(shows("="), "0");
        assert_eq!(shows("12="), "12");
    }

    #[test]
    fn a_digit_after_equals_starts_a_new_calculation() {
        let calculator = run("2+3=4");
        assert_eq!(calculator.display(), "4");
        assert_eq!(calculator.expression(), "");
        assert_eq!(shows("2+3=4*2="), "8");
    }

    #[test]
    fn an_operator_after_equals_continues_from_the_result() {
        assert_eq!(shows("2+3=*2="), "10");
    }

    #[test]
    fn expression_line_tracks_the_calculation() {
        assert_eq!(run("3.66+").expression(), "3.66+");
        assert_eq!(run("3.66+3.5").expression(), "3.66+3.5");
        assert_eq!(run("3.66+3.59=").expression(), "3.66+3.59");
        assert_eq!(run("2+3==").expression(), "5+3");
        assert_eq!(run("1000*2").expression(), "1,000×2");
        assert_eq!(run("8/").expression(), "8÷");
        assert_eq!(run("8-").expression(), "8−");
    }

    #[test]
    fn clear_key_toggles_between_c_and_ac() {
        let mut calculator = run("12");
        assert_eq!(calculator.clear_label(), ClearLabel::Clear);
        calculator.press(Clear);
        assert_eq!(calculator.display(), "0");
        assert_eq!(calculator.clear_label(), ClearLabel::AllClear);
        assert_eq!(run("2+3=").clear_label(), ClearLabel::AllClear);
        assert_eq!(run("2+").clear_label(), ClearLabel::AllClear);
    }

    #[test]
    fn c_clears_only_the_entry_and_keeps_the_pending_operation() {
        let mut calculator = run("5+3");
        calculator.press(Clear);
        assert_eq!(calculator.display(), "0");
        assert_eq!(calculator.highlighted_operator(), Some(Add));
        assert_eq!(calculator.clear_label(), ClearLabel::AllClear);
        calculator.press(Digit(2));
        calculator.press(Equals);
        assert_eq!(calculator.display(), "7");
    }

    #[test]
    fn ac_clears_everything() {
        let mut calculator = run("5+3c");
        calculator.press(Clear);
        assert_eq!(calculator.highlighted_operator(), None);
        assert_eq!(calculator.expression(), "");
        calculator.press(Digit(4));
        calculator.press(Equals);
        assert_eq!(calculator.display(), "4");
        // The repeat operation is gone too.
        let mut calculator = run("2+3=cc");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "0");
    }

    #[test]
    fn toggle_sign() {
        assert_eq!(shows("5n"), "-5");
        assert_eq!(shows("5nn"), "5");
        assert_eq!(shows("n"), "-0");
        assert_eq!(shows("n5"), "-5");
        assert_eq!(shows("2+3=n"), "-5");
        assert_eq!(shows("9-n3="), "12");
        assert_eq!(shows("1.5n"), "-1.5");
    }

    #[test]
    fn toggle_sign_after_an_operator_starts_a_negative_operand() {
        let calculator = run("7*n");
        assert_eq!(calculator.display(), "-0");
        assert_eq!(calculator.highlighted_operator(), None);
        assert_eq!(shows("7*n2="), "-14");
    }

    #[test]
    fn percent_of_a_plain_number_divides_by_one_hundred() {
        assert_eq!(shows("50%"), "0.5");
        assert_eq!(shows("5%%"), "0.0005");
    }

    #[test]
    fn percent_with_add_or_subtract_takes_a_share_of_the_first_operand() {
        assert_eq!(shows("50+10%"), "5");
        assert_eq!(shows("50+10%="), "55");
        assert_eq!(shows("200-15%="), "170");
        assert_eq!(shows("50+%"), "25");
    }

    #[test]
    fn percent_with_multiply_or_divide_divides_by_one_hundred() {
        assert_eq!(shows("200*10%"), "0.1");
        assert_eq!(shows("200*10%="), "20");
        assert_eq!(shows("50/50%="), "100");
    }

    #[test]
    fn divide_by_zero_shows_error() {
        let calculator = run("5/0=");
        assert_eq!(calculator.display(), "Error");
        assert!(calculator.is_error());
        assert_eq!(calculator.clear_label(), ClearLabel::AllClear);
        assert_eq!(calculator.highlighted_operator(), None);
        assert_eq!(shows("0/0="), "Error");
        assert_eq!(shows("5/0+"), "Error");
    }

    #[test]
    fn error_ignores_operators_and_a_digit_starts_over() {
        assert_eq!(shows("5/0=+="), "Error");
        assert_eq!(shows("5/0=7"), "7");
        assert_eq!(shows("5/0=7+1="), "8");
        assert_eq!(shows("5/0=c"), "0");
        assert_eq!(shows("5/0=b"), "0");
        assert_eq!(shows("5/0=."), "0.");
    }

    #[test]
    fn backspace_deletes_the_last_typed_character() {
        assert_eq!(shows("123b"), "12");
        assert_eq!(shows("1bb"), "0");
        assert_eq!(shows("1.5b"), "1.");
        assert_eq!(shows("1.5bb"), "1");
        assert_eq!(shows("5nb"), "0");
        assert_eq!(shows("12345b"), "1,234");
    }

    #[test]
    fn backspace_leaves_a_result_alone() {
        assert_eq!(shows("2+3=b"), "5");
        let calculator = run("2+b");
        assert_eq!(calculator.display(), "2");
        assert_eq!(calculator.highlighted_operator(), Some(Add));
    }

    #[test]
    fn large_results_switch_to_scientific_notation() {
        assert_eq!(shows("999999999+1="), "1e9");
        assert_eq!(shows("123456789*1000="), "1.2345679e11");
        assert_eq!(shows("999999999*999999999="), "1e18");
        assert_eq!(shows("987654321*987654321="), "9.7546106e17");
        assert_eq!(shows("123456789*-"), "123,456,789");
        assert_eq!(shows("99999999*9="), "899,999,991");
    }

    #[test]
    fn tiny_results_switch_to_scientific_notation() {
        assert_eq!(shows("1/100000000/10="), "1e-9");
        assert_eq!(shows("0.00000001/1000="), "1e-11");
        assert_eq!(shows("0.00001234/10="), "0.000001234");
    }

    #[test]
    fn rounding_carry_into_ten_digits_goes_scientific() {
        assert_eq!(format_value(999_999_999.6), "1e9");
        assert_eq!(format_value(-999_999_999.6), "-1e9");
    }

    #[test]
    fn value_formatting() {
        assert_eq!(format_value(0.0), "0");
        assert_eq!(format_value(-0.0), "0");
        assert_eq!(format_value(1234.5), "1,234.5");
        assert_eq!(format_value(-1234567.25), "-1,234,567.25");
        assert_eq!(format_value(12_345.678_912_3), "12,345.6789");
        assert_eq!(format_value(0.5), "0.5");
        assert_eq!(format_value(f64::INFINITY), "Error");
        assert_eq!(format_value(f64::NAN), "Error");
        assert_eq!(format_value(1.5e300), "1.5e300");
        assert_eq!(format_value(-2.5e-20), "-2.5e-20");
    }

    #[test]
    fn overflow_is_an_error() {
        let mut calculator = Calculator::new();
        assert!(calculator.paste("1e300"));
        calculator.press(Key::Operator(Multiply));
        assert!(calculator.paste("1e300"));
        calculator.press(Equals);
        assert_eq!(calculator.display(), "Error");
    }

    #[test]
    fn copy_strips_grouping_separators() {
        assert_eq!(run("1234567").copy_text(), "1234567");
        assert_eq!(run("1.5n").copy_text(), "-1.5");
        assert_eq!(run("999999999+1=").copy_text(), "1e9");
    }

    #[test]
    fn paste_accepts_numbers_and_rejects_text() {
        let mut calculator = Calculator::new();
        assert!(calculator.paste(" 1,234.5 \n"));
        assert_eq!(calculator.display(), "1,234.5");
        assert_eq!(calculator.clear_label(), ClearLabel::Clear);
        assert!(calculator.paste("\u{2212}42"));
        assert_eq!(calculator.display(), "-42");
        assert!(calculator.paste("2.5e3"));
        assert_eq!(calculator.display(), "2,500");
        assert!(!calculator.paste("hello"));
        assert!(!calculator.paste(""));
        assert!(!calculator.paste("inf"));
        assert!(!calculator.paste("NaN"));
        assert!(!calculator.paste("1e999"));
        assert_eq!(calculator.display(), "2,500");
    }

    #[test]
    fn paste_supplies_the_pending_operand() {
        let mut calculator = run("10+");
        assert!(calculator.paste("5"));
        assert_eq!(calculator.highlighted_operator(), None);
        assert_eq!(calculator.expression(), "10+5");
        calculator.press(Equals);
        assert_eq!(calculator.display(), "15");
    }

    #[test]
    fn paste_recovers_from_error() {
        let mut calculator = run("1/0=");
        assert!(calculator.paste("3"));
        assert_eq!(calculator.display(), "3");
        assert!(!calculator.is_error());
    }

    #[test]
    fn digits_typed_after_a_percent_start_a_new_entry() {
        assert_eq!(shows("50%7"), "7");
    }

    #[test]
    fn negative_zero_results_display_as_zero() {
        assert_eq!(shows("0n*5="), "0");
    }

    #[test]
    fn fitted_font_shrinks_long_numbers() {
        let full = fitted_font_size("7.25", 209.0, 64.0, 24.0);
        assert_eq!(full, 64.0);
        let long = fitted_font_size("123,456,789", 209.0, 64.0, 24.0);
        assert!(long < 64.0 && long > 24.0, "{long}");
        let longer = fitted_font_size("-9.99999998e17", 209.0, 64.0, 24.0);
        assert!(longer <= long);
        assert_eq!(fitted_font_size("", 209.0, 64.0, 24.0), 64.0);
        assert_eq!(fitted_font_size(&"8".repeat(100), 209.0, 64.0, 24.0), 24.0);
    }

    #[test]
    fn operator_symbols_match_the_keypad() {
        assert_eq!(Add.symbol(), "+");
        assert_eq!(Subtract.symbol(), "−");
        assert_eq!(Multiply.symbol(), "×");
        assert_eq!(Divide.symbol(), "÷");
        assert_eq!(ClearLabel::AllClear.text(), "AC");
        assert_eq!(ClearLabel::Clear.text(), "C");
    }
}
