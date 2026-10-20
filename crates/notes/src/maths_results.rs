//! Format ▸ Maths Results: typing `2+3=` at the end of a line inserts the
//! result, like the Mac's Insert Results mode (docs/parity.md NOTES-22).

/// Result text for a line that ends with `=` after an arithmetic
/// expression, or `None` when there is nothing to compute.
pub(super) fn result_for_line(line_before_cursor: &str) -> Option<String> {
    let before = line_before_cursor.strip_suffix('=')?;
    let allowed = |c: char| c.is_ascii_digit() || " .+-*/×÷^()%".contains(c);
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| allowed(*c))
        .last()
        .map_or(before.len(), |(i, _)| i);
    let mut expr = before[start..].trim();
    // Drop operators that belong to surrounding prose ("item-2+3=").
    let at_line_edge = start == 0 || before[..start].ends_with(char::is_whitespace);
    while let Some(first) = expr.chars().next() {
        if first.is_ascii_digit()
            || first == '('
            || first == '.'
            || (first == '-' && at_line_edge && expr.len() == before[start..].trim().len())
        {
            break;
        }
        expr = expr[first.len_utf8()..].trim_start();
    }
    if !expr.chars().any(|c| "+-*/×÷^%".contains(c)) || !expr.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    let value = evaluate(expr)?;
    value.is_finite().then(|| format_number(value))
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let text = format!("{value:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub(super) fn evaluate(expr: &str) -> Option<f64> {
    let chars: Vec<char> = expr.chars().filter(|c| !c.is_whitespace()).collect();
    let mut parser = Parser { chars, pos: 0 };
    let value = parser.sum()?;
    (parser.pos == parser.chars.len()).then_some(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        while let Some(op @ ('+' | '-')) = self.peek() {
            self.pos += 1;
            let rhs = self.product()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    fn product(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        while let Some(op @ ('*' | '×' | '/' | '÷')) = self.peek() {
            self.pos += 1;
            let rhs = self.power()?;
            value = if matches!(op, '*' | '×') {
                value * rhs
            } else {
                value / rhs
            };
        }
        Some(value)
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.peek() == Some('^') {
            self.pos += 1;
            let exponent = self.power()?;
            return Some(base.powf(exponent));
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            '-' => {
                self.pos += 1;
                Some(-self.unary()?)
            }
            '+' => {
                self.pos += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Option<f64> {
        let mut value = if self.peek()? == '(' {
            self.pos += 1;
            let inner = self.sum()?;
            if self.peek()? != ')' {
                return None;
            }
            self.pos += 1;
            inner
        } else {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                self.pos += 1;
            }
            self.chars[start..self.pos]
                .iter()
                .collect::<String>()
                .parse()
                .ok()?
        };
        while self.peek() == Some('%') {
            self.pos += 1;
            value /= 100.0;
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_simple_lines() {
        assert_eq!(result_for_line("2+3="), Some("5".into()));
        assert_eq!(result_for_line("Total: 12 × (3 + 1) ="), Some("48".into()));
        assert_eq!(result_for_line("7/2="), Some("3.5".into()));
        assert_eq!(result_for_line("2^3^2="), Some("512".into()));
        assert_eq!(result_for_line("-5+2="), Some("-3".into()));
        assert_eq!(result_for_line("1/3="), Some("0.333333".into()));
    }

    #[test]
    fn ignores_non_expressions() {
        assert_eq!(result_for_line("hello="), None);
        assert_eq!(result_for_line("42="), None);
        assert_eq!(result_for_line("2+="), None);
        assert_eq!(result_for_line("1/0="), None);
        assert_eq!(result_for_line("2+3"), None);
    }
}
