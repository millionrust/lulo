//! The Programmer-mode calculator engine: pure integer state, no GPUI
//! (CALC-06, View ▸ Programmer ⌘3).
//!
//! Unlike Basic/Scientific, macOS's Programmer mode works on whole numbers
//! only: a value in one of four bases (hexadecimal, decimal, octal, binary),
//! clipped to one of four word sizes (byte/word/double word/quad word) with
//! wraparound on overflow, plus the usual bitwise operators (AND, OR, XOR,
//! NOT, shift left, shift right) alongside ordinary `+ − × ÷`.
//!
//! This is a from-scratch, functionally-faithful implementation: no Mac
//! capture of Programmer mode exists yet (it was out of scope until now —
//! see CALC-02's history), so the exact key layout and window geometry here
//! are an original design rather than a measurement, documented as such in
//! `programmer_keypad.rs`. The maths themselves — base conversion, bitwise
//! operators, word-size wraparound — are exact.
//!
//! Values are kept as the magnitude within the current word size (an
//! unsigned `u64` logically clipped to 8/16/32/64 bits): Programmer mode
//! shows `FF`, not `-1`, for a byte's all-one-bits value. `±` still works as
//! an explicit two's-complement negate within the current word size, same
//! as a physical bitwise calculator.

/// The base a value is typed and displayed in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Base {
    Hex,
    Dec,
    Oct,
    Bin,
}

impl Base {
    pub fn radix(self) -> u32 {
        match self {
            Self::Hex => 16,
            Self::Dec => 10,
            Self::Oct => 8,
            Self::Bin => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hex => "HEX",
            Self::Dec => "DEC",
            Self::Oct => "OCT",
            Self::Bin => "BIN",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Hex => Self::Dec,
            Self::Dec => Self::Oct,
            Self::Oct => Self::Bin,
            Self::Bin => Self::Hex,
        }
    }

    /// Whether `digit` (0-15) is a legal digit in this base.
    pub fn allows(self, digit: u8) -> bool {
        u32::from(digit) < self.radix()
    }
}

/// The word size a result is clipped to, like the Mac's word-size menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WordSize {
    Byte,
    Word,
    DWord,
    QWord,
}

impl WordSize {
    pub fn bits(self) -> u32 {
        match self {
            Self::Byte => 8,
            Self::Word => 16,
            Self::DWord => 32,
            Self::QWord => 64,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Byte => "Byte",
            Self::Word => "Word",
            Self::DWord => "Dword",
            Self::QWord => "Qword",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Byte => Self::Word,
            Self::Word => Self::DWord,
            Self::DWord => Self::QWord,
            Self::QWord => Self::Byte,
        }
    }

    /// All bits set within this width (`u64::MAX` for `QWord`, since
    /// `1u64 << 64` would overflow).
    pub fn mask(self) -> u64 {
        if self.bits() >= 64 {
            u64::MAX
        } else {
            (1u64 << self.bits()) - 1
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    And,
    Or,
    Xor,
    ShiftLeft,
    ShiftRight,
}

impl Operator {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "−",
            Self::Multiply => "×",
            Self::Divide => "÷",
            Self::And => "AND",
            Self::Or => "OR",
            Self::Xor => "XOR",
            Self::ShiftLeft => "<<",
            Self::ShiftRight => ">>",
        }
    }

    fn apply(self, left: u64, right: u64, mask: u64) -> Option<u64> {
        let value = match self {
            Self::Add => left.wrapping_add(right),
            Self::Subtract => left.wrapping_sub(right),
            Self::Multiply => left.wrapping_mul(right),
            Self::Divide => {
                if right == 0 {
                    return None;
                }
                left / right
            }
            Self::And => left & right,
            Self::Or => left | right,
            Self::Xor => left ^ right,
            Self::ShiftLeft => left.wrapping_shl((right & 63) as u32),
            Self::ShiftRight => left.wrapping_shr((right & 63) as u32),
        };
        Some(value & mask)
    }
}

/// Every key on the Programmer keypad (`programmer_keypad.rs` lays them out).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Digit(u8),
    Operator(Operator),
    Not,
    ToggleSign,
    Clear,
    Backspace,
    Equals,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClearLabel {
    AllClear,
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

#[derive(Clone, Debug)]
pub struct ProgrammerCalculator {
    base: Base,
    word_size: WordSize,
    /// The settled left operand/accumulated value, always already masked to
    /// `word_size`.
    value: u64,
    pending: Option<Operator>,
    /// Digits typed for the operand currently being entered, in `base`.
    /// Empty means nothing has been typed since the last operator/clear, so
    /// `value` itself is what is shown.
    entry: String,
    error: bool,
}

impl Default for ProgrammerCalculator {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgrammerCalculator {
    pub fn new() -> Self {
        Self {
            base: Base::Dec,
            word_size: WordSize::QWord,
            value: 0,
            pending: None,
            entry: String::new(),
            error: false,
        }
    }

    pub fn base(&self) -> Base {
        self.base
    }

    pub fn word_size(&self) -> WordSize {
        self.word_size
    }

    /// Switch base, converting the value being typed (if any) so the typed
    /// number keeps its value rather than its digit string.
    pub fn set_base(&mut self, base: Base) {
        if base == self.base {
            return;
        }
        self.commit_entry();
        self.base = base;
    }

    /// Switch word size, clipping the current value into the new width.
    pub fn set_word_size(&mut self, word_size: WordSize) {
        if word_size == self.word_size {
            return;
        }
        self.commit_entry();
        self.word_size = word_size;
        self.value &= self.mask();
    }

    fn mask(&self) -> u64 {
        self.word_size.mask()
    }

    /// Fold any digits being typed into `value`, clearing `entry`. Used
    /// before a base/word-size change so neither has stale digits to
    /// reinterpret.
    fn commit_entry(&mut self) {
        if !self.entry.is_empty() {
            self.value = self.operand();
            self.entry.clear();
        }
    }

    /// The operand to use right now: the typed entry if any, else `value`.
    fn operand(&self) -> u64 {
        if self.entry.is_empty() {
            self.value
        } else {
            u64::from_str_radix(&self.entry, self.base.radix()).unwrap_or(0) & self.mask()
        }
    }

    pub fn press(&mut self, key: Key) {
        match key {
            Key::Digit(digit) => self.digit(digit),
            Key::Operator(operator) => self.operator(operator),
            Key::Not => self.not(),
            Key::ToggleSign => self.negate(),
            Key::Clear => self.clear(),
            Key::Backspace => self.backspace(),
            Key::Equals => self.equals(),
        }
    }

    fn digit(&mut self, digit: u8) {
        if !self.base.allows(digit) {
            return;
        }
        if self.error {
            self.clear_all();
        }
        let character = std::char::from_digit(u32::from(digit), self.base.radix())
            .unwrap_or('0')
            .to_ascii_uppercase();
        let mut candidate = self.entry.clone();
        candidate.push(character);
        // Reject a keystroke that would overflow the current word size
        // rather than silently wrapping mid-entry.
        if u64::from_str_radix(&candidate, self.base.radix())
            .is_ok_and(|value| value <= self.mask())
        {
            self.entry = candidate;
        }
    }

    fn operator(&mut self, operator: Operator) {
        if self.error {
            return;
        }
        let operand = self.operand();
        match self.pending {
            Some(pending) => match pending.apply(self.value, operand, self.mask()) {
                Some(result) => self.value = result,
                None => {
                    self.error = true;
                    self.value = 0;
                    self.entry.clear();
                    self.pending = None;
                    return;
                }
            },
            None => self.value = operand,
        };
        self.entry.clear();
        self.pending = Some(operator);
    }

    fn equals(&mut self) {
        if self.error {
            return;
        }
        let operand = self.operand();
        if let Some(pending) = self.pending {
            match pending.apply(self.value, operand, self.mask()) {
                Some(result) => self.value = result,
                None => {
                    self.error = true;
                    self.value = 0;
                }
            }
        } else {
            self.value = operand;
        }
        self.entry.clear();
        self.pending = None;
    }

    /// Unary bitwise NOT, applied immediately like `±` on the Mac.
    fn not(&mut self) {
        if self.error {
            return;
        }
        self.value = !self.operand() & self.mask();
        self.entry.clear();
        self.pending = None;
    }

    /// Two's-complement negate within the current word size, applied
    /// immediately.
    fn negate(&mut self) {
        if self.error {
            return;
        }
        self.value = self.operand().wrapping_neg() & self.mask();
        self.entry.clear();
        self.pending = None;
    }

    fn clear(&mut self) {
        if self.error || !self.entry.is_empty() {
            self.entry.clear();
            self.error = false;
        } else {
            self.clear_all();
        }
    }

    fn clear_all(&mut self) {
        self.value = 0;
        self.pending = None;
        self.entry.clear();
        self.error = false;
    }

    fn backspace(&mut self) {
        if self.error {
            self.clear_all();
            return;
        }
        self.entry.pop();
    }

    pub fn clear_label(&self) -> ClearLabel {
        if self.entry.is_empty() && self.pending.is_none() && !self.error {
            ClearLabel::AllClear
        } else {
            ClearLabel::Clear
        }
    }

    pub fn is_error(&self) -> bool {
        self.error
    }

    pub fn highlighted_operator(&self) -> Option<Operator> {
        self.pending.filter(|_| self.entry.is_empty())
    }

    /// The formatted value in the current base, with no leading zeroes
    /// (`"0"` for zero itself). Binary/octal/hex show the bare digits;
    /// macOS's own Programmer grouping (nibble spacing) is not reproduced.
    pub fn display(&self) -> String {
        if self.error {
            return crate::engine::ERROR_TEXT.to_owned();
        }
        if !self.entry.is_empty() {
            return self.entry.clone();
        }
        format_radix(self.value, self.base.radix())
    }

    /// The current value as an ordinary `u64`, for mode-switch handoff with
    /// Basic/Scientific (which carry a plain `f64`).
    pub fn current_value(&self) -> u64 {
        self.operand()
    }

    pub fn restore_value(&mut self, value: f64) {
        self.clear_all();
        self.value = if value.is_finite() && value >= 0.0 {
            (value as u64) & self.mask()
        } else {
            0
        };
    }
}

fn format_radix(mut value: u64, radix: u32) -> String {
    if value == 0 {
        return "0".to_owned();
    }
    let mut digits = Vec::new();
    while value > 0 {
        let digit = (value % u64::from(radix)) as u32;
        digits.push(
            std::char::from_digit(digit, radix)
                .unwrap_or('0')
                .to_ascii_uppercase(),
        );
        value /= u64::from(radix);
    }
    digits.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_arithmetic_wraps_to_the_word_size() {
        let mut calculator = ProgrammerCalculator::new();
        calculator.set_base(Base::Hex);
        calculator.set_word_size(WordSize::Byte);
        for digit in [15, 15] {
            calculator.press(Key::Digit(digit));
        }
        assert_eq!(calculator.display(), "FF");
        calculator.press(Key::Operator(Operator::Add));
        calculator.press(Key::Digit(1));
        calculator.press(Key::Equals);
        // 0xFF + 1 wraps a byte to 0x00.
        assert_eq!(calculator.display(), "0");
    }

    #[test]
    fn bitwise_operators_are_exact() {
        let mut calculator = ProgrammerCalculator::new();
        calculator.set_base(Base::Dec);
        calculator.set_word_size(WordSize::Byte);
        calculator.press(Key::Digit(1));
        calculator.press(Key::Digit(2));
        calculator.press(Key::Operator(Operator::And));
        calculator.press(Key::Digit(1));
        calculator.press(Key::Digit(0));
        calculator.press(Key::Equals);
        assert_eq!(calculator.display(), "8"); // 12 & 10 = 8

        calculator.clear_all();
        calculator.press(Key::Digit(1));
        calculator.press(Key::Operator(Operator::ShiftLeft));
        calculator.press(Key::Digit(3));
        calculator.press(Key::Equals);
        assert_eq!(calculator.display(), "8"); // 1 << 3 = 8
    }

    #[test]
    fn base_switch_converts_the_live_value() {
        let mut calculator = ProgrammerCalculator::new();
        calculator.set_base(Base::Dec);
        calculator.press(Key::Digit(2));
        calculator.press(Key::Digit(5));
        calculator.press(Key::Digit(5));
        assert_eq!(calculator.display(), "255");
        calculator.set_base(Base::Hex);
        assert_eq!(calculator.display(), "FF");
        calculator.set_base(Base::Bin);
        assert_eq!(calculator.display(), "11111111");
    }

    #[test]
    fn digits_outside_the_base_are_rejected() {
        let mut calculator = ProgrammerCalculator::new();
        calculator.set_base(Base::Bin);
        calculator.press(Key::Digit(1));
        calculator.press(Key::Digit(9)); // not a binary digit
        assert_eq!(calculator.display(), "1");
    }

    #[test]
    fn word_size_change_clips_the_value() {
        let mut calculator = ProgrammerCalculator::new();
        calculator.set_base(Base::Hex);
        calculator.set_word_size(WordSize::QWord);
        for digit in [15, 15, 15, 15] {
            calculator.press(Key::Digit(digit));
        }
        assert_eq!(calculator.display(), "FFFF");
        calculator.set_word_size(WordSize::Byte);
        assert_eq!(calculator.display(), "FF");
    }

    #[test]
    fn divide_by_zero_sets_an_error_state() {
        let mut calculator = ProgrammerCalculator::new();
        calculator.press(Key::Digit(5));
        calculator.press(Key::Operator(Operator::Divide));
        calculator.press(Key::Digit(0));
        calculator.press(Key::Equals);
        assert!(calculator.is_error());
        assert_eq!(calculator.display(), crate::engine::ERROR_TEXT);
    }

    #[test]
    fn not_and_negate_apply_immediately() {
        let mut calculator = ProgrammerCalculator::new();
        calculator.set_word_size(WordSize::Byte);
        calculator.press(Key::Digit(0));
        calculator.press(Key::Not);
        assert_eq!(calculator.display(), "255");
    }
}
