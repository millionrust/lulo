//! Maths Notes (View ▸ Maths Notes… ⌥⌘M): a pure line-by-line expression
//! evaluator, no GPUI.
//!
//! Each line is either a plain expression (`"12 + 3 * 4"`) or a variable
//! assignment (`"rent = 1200"`); later lines can refer to names assigned by
//! earlier ones (`"rent * 12"`). This is a deliberately small, from-scratch
//! recursive-descent parser — `+ − × ÷ ^ ( )`, decimals and unary minus —
//! not the Mac's full Soulver-derived Maths Notes (unit-aware arithmetic,
//! running totals, sums over a block). The Calculator parity row says so.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum LineOutcome {
    /// A blank line: nothing to show.
    Empty,
    /// A plain expression's value.
    Value(f64),
    /// `name = expr`: the assigned value, stored for later lines too.
    Assigned { name: String, value: f64 },
    /// Could not be parsed or evaluated (unknown name, divide by zero, …).
    Error,
}

/// Evaluate every line of `source` in order, left-to-right, top-to-bottom,
/// sharing one variable environment. Returns one display string per line,
/// in the Mac's style (`"= 42"` for a value, `""` for a blank line, `"⚠︎"`
/// for an error).
pub fn evaluate(source: &str) -> Vec<String> {
    let mut vars = HashMap::new();
    source
        .lines()
        .map(|line| display(&evaluate_line(line, &mut vars)))
        .collect()
}

pub fn display(outcome: &LineOutcome) -> String {
    match outcome {
        LineOutcome::Empty => String::new(),
        LineOutcome::Value(value) => format!("= {}", format_value(*value)),
        LineOutcome::Assigned { value, .. } => format!("= {}", format_value(*value)),
        LineOutcome::Error => "⚠︎".to_owned(),
    }
}

pub fn evaluate_line(line: &str, vars: &mut HashMap<String, f64>) -> LineOutcome {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return LineOutcome::Empty;
    }
    if let Some((name, rhs)) = split_assignment(trimmed) {
        return match parse_and_eval(rhs, vars) {
            Some(value) => {
                vars.insert(name.clone(), value);
                LineOutcome::Assigned { name, value }
            }
            None => LineOutcome::Error,
        };
    }
    match parse_and_eval(trimmed, vars) {
        Some(value) => LineOutcome::Value(value),
        None => LineOutcome::Error,
    }
}

/// `"rent = 1200 / 12"` -> `Some(("rent", "1200 / 12"))`. Only a single
/// `=` with a valid identifier on the left counts as an assignment, so
/// `"2 = 2"` and anything with `==`/`<=`/`>=` is left as a plain (failing)
/// expression rather than mis-parsed.
fn split_assignment(line: &str) -> Option<(String, &str)> {
    let position = line.find('=')?;
    if line[position + 1..].starts_with('=') || line[..position].ends_with(['<', '>', '!', '=']) {
        return None;
    }
    let name = line[..position].trim();
    if !is_identifier(name) {
        return None;
    }
    Some((name.to_owned(), line[position + 1..].trim()))
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

fn format_value(value: f64) -> String {
    if !value.is_finite() {
        return "Error".to_owned();
    }
    let fixed = format!("{value:.10}");
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

fn parse_and_eval(expression: &str, vars: &HashMap<String, f64>) -> Option<f64> {
    let tokens = tokenize(expression)?;
    let mut parser = Parser {
        tokens,
        position: 0,
        vars,
    };
    let value = parser.expr()?;
    if parser.position != parser.tokens.len() {
        return None; // trailing garbage, such as "2 3" or an unmatched ")".
    }
    value.is_finite().then_some(value)
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
}

fn tokenize(text: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        match character {
            ' ' | '\t' => index += 1,
            '+' => {
                tokens.push(Token::Plus);
                index += 1;
            }
            '-' | '\u{2212}' => {
                tokens.push(Token::Minus);
                index += 1;
            }
            '*' | '×' => {
                tokens.push(Token::Star);
                index += 1;
            }
            '/' | '÷' => {
                tokens.push(Token::Slash);
                index += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                index += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                index += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                index += 1;
            }
            '0'..='9' | '.' => {
                let start = index;
                while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.')
                {
                    index += 1;
                }
                let text: String = chars[start..index].iter().collect();
                tokens.push(Token::Number(text.parse().ok()?));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = index;
                while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_')
                {
                    index += 1;
                }
                let text: String = chars[start..index].iter().collect();
                tokens.push(Token::Ident(text));
            }
            _ => return None, // an unrecognised character fails the whole line.
        }
    }
    Some(tokens)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    position: usize,
    vars: &'a HashMap<String, f64>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.position);
        self.position += 1;
        token
    }

    /// `term (('+' | '-') term)*`
    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.advance();
                    value += self.term()?;
                }
                Some(Token::Minus) => {
                    self.advance();
                    value -= self.term()?;
                }
                _ => return Some(value),
            }
        }
    }

    /// `power (('*' | '/') power)*`
    fn term(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.advance();
                    value *= self.power()?;
                }
                Some(Token::Slash) => {
                    self.advance();
                    let divisor = self.power()?;
                    if divisor == 0.0 {
                        return None;
                    }
                    value /= divisor;
                }
                _ => return Some(value),
            }
        }
    }

    /// `unary ('^' power)?` (right-associative).
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if matches!(self.peek(), Some(Token::Caret)) {
            self.advance();
            let exponent = self.power()?;
            Some(base.powf(exponent))
        } else {
            Some(base)
        }
    }

    fn unary(&mut self) -> Option<f64> {
        if matches!(self.peek(), Some(Token::Minus)) {
            self.advance();
            return Some(-self.unary()?);
        }
        if matches!(self.peek(), Some(Token::Plus)) {
            self.advance();
            return self.unary();
        }
        self.primary()
    }

    fn primary(&mut self) -> Option<f64> {
        match self.advance()?.clone() {
            Token::Number(value) => Some(value),
            Token::Ident(name) => self.vars.get(&name).copied(),
            Token::LParen => {
                let value = self.expr()?;
                match self.advance() {
                    Some(Token::RParen) => Some(value),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(line: &str, vars: &mut HashMap<String, f64>) -> Option<f64> {
        match evaluate_line(line, vars) {
            LineOutcome::Value(value) | LineOutcome::Assigned { value, .. } => Some(value),
            _ => None,
        }
    }

    #[test]
    fn respects_operator_precedence_and_parentheses() {
        let mut vars = HashMap::new();
        assert_eq!(eval("2 + 3 * 4", &mut vars), Some(14.0));
        assert_eq!(eval("(2 + 3) * 4", &mut vars), Some(20.0));
        assert_eq!(eval("2 ^ 3 ^ 2", &mut vars), Some(512.0)); // right-assoc
        assert_eq!(eval("-4 + 1", &mut vars), Some(-3.0));
    }

    #[test]
    fn assignments_are_visible_to_later_lines() {
        let notes = "rent = 1200\nrent * 12";
        let lines = evaluate(notes);
        assert_eq!(lines[0], "= 1200");
        assert_eq!(lines[1], "= 14400");
    }

    #[test]
    fn blank_lines_produce_no_output_and_do_not_break_later_lines() {
        let notes = "1 + 1\n\n2 + 2";
        let lines = evaluate(notes);
        assert_eq!(lines, vec!["= 2", "", "= 4"]);
    }

    #[test]
    fn unknown_names_and_divide_by_zero_are_errors() {
        let mut vars = HashMap::new();
        assert_eq!(eval("nope + 1", &mut vars), None);
        assert_eq!(eval("1 / 0", &mut vars), None);
        assert_eq!(evaluate("1 / 0")[0], "⚠︎");
    }

    #[test]
    fn only_a_valid_bare_identifier_before_a_single_equals_is_an_assignment() {
        let mut vars = HashMap::new();
        // Not an assignment: falls through to a (failing) expression.
        assert_eq!(eval("2 = 2", &mut vars), None);
        assert_eq!(eval("total = 2 + 2", &mut vars), Some(4.0));
        assert_eq!(vars.get("total"), Some(&4.0));
    }
}
