//! RPN Mode (View ▸ RPN Mode, ⌘R): a stack-based entry engine, pure logic,
//! no GPUI.
//!
//! On the Mac, RPN Mode is a View-menu checkbox that changes how Basic (and
//! Scientific) take numbers in: instead of `12 + 3 =`, you type `12`,
//! press Enter to push it onto a stack, type `3`, then press an operator to
//! pop the top two entries, apply it, and push the result back. This mode
//! is scoped to Basic's keypad for this pass — see the Calculator parity
//! row CALC-06 for why Scientific's much larger function set is left
//! alone — toggling it while Scientific is active is disabled in the menu.

/// Lines shown above the current entry, oldest first (so the *last* line is
/// the one right above the entry, like a classic HP stack display).
const VISIBLE_STACK_LINES: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl Operator {
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
            Self::Divide => {
                if right == 0.0 {
                    return None;
                }
                left / right
            }
        };
        value.is_finite().then_some(value)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RpnEngine {
    stack: Vec<f64>,
    entry: String,
    error: bool,
}

impl RpnEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn digit(&mut self, digit: u8) {
        if self.error {
            self.clear_all();
        }
        self.entry.push((b'0' + digit.min(9)) as char);
    }

    pub fn decimal(&mut self) {
        if self.error {
            self.clear_all();
        }
        if !self.entry.contains('.') {
            if self.entry.is_empty() {
                self.entry.push('0');
            }
            self.entry.push('.');
        }
    }

    pub fn toggle_sign(&mut self) {
        if self.entry.starts_with('-') {
            self.entry.remove(0);
        } else if !self.entry.is_empty() {
            self.entry.insert(0, '-');
        } else if let Some(top) = self.stack.last_mut() {
            *top = -*top;
        }
    }

    pub fn backspace(&mut self) {
        self.entry.pop();
    }

    /// `%` divides the entry being typed (or, with nothing typed, the top
    /// of the stack) by 100, in place — the same "percent of itself" the
    /// Mac's own `%` key falls back to with nothing else pending.
    pub fn percent(&mut self) {
        if !self.entry.is_empty() {
            let value = self.current_entry_value() / 100.0;
            self.entry = format_value(value);
        } else if let Some(top) = self.stack.last_mut() {
            *top /= 100.0;
        }
    }

    /// `C` clears the entry only; with nothing typed, `AC` clears the
    /// whole stack, matching Basic's own clear-key convention.
    pub fn clear(&mut self) {
        if self.error || !self.entry.is_empty() {
            self.entry.clear();
            self.error = false;
        } else {
            self.clear_all();
        }
    }

    fn clear_all(&mut self) {
        self.stack.clear();
        self.entry.clear();
        self.error = false;
    }

    /// Push the typed entry onto the stack (the Mac's Enter key, which
    /// takes over the `=` key's position in RPN mode).
    pub fn enter(&mut self) {
        if self.error {
            return;
        }
        let value = self.current_entry_value();
        self.stack.push(value);
        self.entry.clear();
    }

    /// An operator pops the top two stack entries (implicitly pushing a
    /// typed-but-not-entered operand first) and pushes the result.
    pub fn operator(&mut self, operator: Operator) {
        if self.error {
            return;
        }
        if !self.entry.is_empty() {
            self.enter();
        }
        let Some(right) = self.stack.pop() else {
            return;
        };
        let Some(left) = self.stack.pop() else {
            // Not enough operands: put it back rather than losing it.
            self.stack.push(right);
            return;
        };
        match operator.apply(left, right) {
            Some(result) => self.stack.push(result),
            None => self.error = true,
        }
    }

    fn current_entry_value(&self) -> f64 {
        self.entry.parse().unwrap_or(0.0)
    }

    pub fn is_error(&self) -> bool {
        self.error
    }

    pub fn has_entry(&self) -> bool {
        !self.entry.is_empty()
    }

    /// The bottom (working) line: the entry being typed, or the stack's
    /// top value once something has been entered.
    pub fn current_text(&self) -> String {
        if self.error {
            return crate::engine::ERROR_TEXT.to_owned();
        }
        if !self.entry.is_empty() {
            return self.entry.clone();
        }
        self.stack
            .last()
            .map(|value| format_value(*value))
            .unwrap_or_else(|| "0".to_owned())
    }

    /// Up to [`VISIBLE_STACK_LINES`] lines above the working line, oldest
    /// first, skipping the top entry (already shown by `current_text` once
    /// nothing is being typed).
    pub fn stack_lines(&self) -> Vec<String> {
        let skip_top = usize::from(self.entry.is_empty() && !self.stack.is_empty());
        let visible = self.stack.len().saturating_sub(skip_top);
        let start = visible.saturating_sub(VISIBLE_STACK_LINES);
        self.stack[start..visible]
            .iter()
            .map(|value| format_value(*value))
            .collect()
    }

    pub fn stack_depth(&self) -> usize {
        self.stack.len()
    }
}

fn format_value(value: f64) -> String {
    if !value.is_finite() {
        return crate::engine::ERROR_TEXT.to_owned();
    }
    let fixed = format!("{value:.9}");
    let trimmed = if fixed.contains('.') {
        fixed.trim_end_matches('0').trim_end_matches('.')
    } else {
        &fixed
    };
    let trimmed = if trimmed.is_empty() || trimmed == "-0" {
        "0"
    } else {
        trimmed
    };
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_pushes_and_operator_pops_two_and_pushes_the_result() {
        let mut rpn = RpnEngine::new();
        rpn.digit(1);
        rpn.digit(2);
        rpn.enter();
        rpn.digit(3);
        rpn.operator(Operator::Add);
        assert_eq!(rpn.current_text(), "15");
        assert_eq!(rpn.stack_depth(), 1);
    }

    #[test]
    fn operator_implicitly_enters_an_untouched_typed_value() {
        let mut rpn = RpnEngine::new();
        rpn.digit(4);
        rpn.enter();
        rpn.digit(5);
        // No explicit Enter before the operator: 4, 5, × = 20.
        rpn.operator(Operator::Multiply);
        assert_eq!(rpn.current_text(), "20");
    }

    #[test]
    fn divide_by_zero_sets_an_error_state() {
        let mut rpn = RpnEngine::new();
        rpn.digit(5);
        rpn.enter();
        rpn.digit(0);
        rpn.operator(Operator::Divide);
        assert!(rpn.is_error());
        assert_eq!(rpn.current_text(), crate::engine::ERROR_TEXT);
    }

    #[test]
    fn clear_drops_the_entry_before_the_stack() {
        let mut rpn = RpnEngine::new();
        rpn.digit(7);
        rpn.enter();
        rpn.digit(8);
        rpn.clear();
        assert_eq!(rpn.current_text(), "7");
        assert_eq!(rpn.stack_depth(), 1);
        rpn.clear();
        assert_eq!(rpn.stack_depth(), 0);
    }

    #[test]
    fn percent_divides_the_entry_or_stack_top_by_a_hundred() {
        let mut rpn = RpnEngine::new();
        rpn.digit(5);
        rpn.digit(0);
        rpn.percent();
        assert_eq!(rpn.current_text(), "0.5");
        rpn.enter();
        rpn.percent();
        assert_eq!(rpn.current_text(), "0.005");
    }

    #[test]
    fn stack_lines_show_everything_below_the_working_line() {
        let mut rpn = RpnEngine::new();
        for value in [1, 2, 3, 4] {
            rpn.digit(value);
            rpn.enter();
        }
        assert_eq!(rpn.stack_lines(), vec!["1", "2", "3"]);
        assert_eq!(rpn.current_text(), "4");
    }
}
