//! Deterministic local calculator and unit-conversion launcher provider.
//!
//! Spotlight on macOS 26.2 answers arithmetic ("12*7" = 84, "2^10" =
//! 1,024, "sqrt(2)" = 1.4142135624) and unit conversions ("5 km in miles"
//! = 3.11 miles) in the answer card under the bar. Results carry at most
//! ten decimals with trailing zeros dropped and follow the locale's digit
//! grouping, as measured on the owner's Mac.

use super::*;
use crate::conversion;
use crate::locale::Locale;

/// Decimals a calculation keeps ("1/3" = 0.3333333333).
const CALCULATION_DECIMALS: usize = 10;
const MAX_QUERY_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Default)]
pub struct CalculatorProvider;

impl Provider for CalculatorProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(
            CALCULATOR_PROVIDER,
            Category::Calculator,
            Privacy::default(),
        )
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        Ok(answer(query, &Locale::from_environment())
            .map(|text| answer_result(CALCULATOR_PROVIDER, query, text, None))
            .into_iter()
            .collect())
    }
}

/// The answer card's text for `query`: a unit conversion ("3.11 miles") or
/// a calculation ("84"), in `locale`'s number format.
pub fn answer(query: &str, locale: &Locale) -> Option<String> {
    let query = query.trim();
    if query.is_empty() || query.len() > MAX_QUERY_BYTES {
        return None;
    }
    conversion::convert(query, locale)
        .or_else(|| evaluate(query).and_then(|value| locale.format(value, CALCULATION_DECIMALS)))
}

/// One answer-card result: `text` is the answer, the subtitle echoes the
/// query ("12*7 ="), and Return copies the answer.
pub(crate) fn answer_result(
    provider: &str,
    query: &str,
    text: String,
    detail: Option<String>,
) -> SearchResult {
    SearchResult {
        id: ResultId {
            provider: provider_id(provider),
            local: query.trim().into(),
        },
        category: Category::Calculator,
        application_group: None,
        title: text.clone(),
        subtitle: Some(query.trim().into()),
        detail,
        icon: None,
        primary: Action::CopyText { text },
        alternate: None,
        recency_rank: 0,
    }
}

/// Evaluate an arithmetic expression: `+ - * / ^` (also `× ÷ **`),
/// parentheses, unary signs, `sqrt abs ln log exp` and the constants `pi`
/// and `e`. A bare number is not a calculation.
pub fn evaluate(input: &str) -> Option<f64> {
    let input = input.trim();
    if input.is_empty() || input.len() > MAX_QUERY_BYTES {
        return None;
    }
    let tokens = tokenize(input)?;
    let calculation = tokens.iter().any(|token| {
        matches!(
            token,
            Token::Plus
                | Token::Minus
                | Token::Times
                | Token::Divide
                | Token::Power
                | Token::Function(_)
        )
    });
    if !calculation {
        return None;
    }
    let mut parser = Parser {
        tokens: &tokens,
        position: 0,
    };
    let value = parser.expression()?;
    (parser.position == tokens.len() && value.is_finite()).then_some(value)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Number(f64),
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    Open,
    Close,
    Function(Function),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Function {
    Sqrt,
    Abs,
    Ln,
    Log,
    Exp,
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let characters = input.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        match character {
            ' ' | '\t' => index += 1,
            '+' => {
                tokens.push(Token::Plus);
                index += 1;
            }
            '-' | '−' => {
                tokens.push(Token::Minus);
                index += 1;
            }
            '*' | '×' | '·' => {
                if character == '*' && characters.get(index + 1) == Some(&'*') {
                    tokens.push(Token::Power);
                    index += 2;
                } else {
                    tokens.push(Token::Times);
                    index += 1;
                }
            }
            '/' | '÷' => {
                tokens.push(Token::Divide);
                index += 1;
            }
            '^' => {
                tokens.push(Token::Power);
                index += 1;
            }
            '(' => {
                tokens.push(Token::Open);
                index += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                index += 1;
            }
            'π' => {
                tokens.push(Token::Number(std::f64::consts::PI));
                index += 1;
            }
            '0'..='9' | '.' => {
                let start = index;
                let mut decimal = false;
                while let Some(&next) = characters.get(index) {
                    match next {
                        '0'..='9' => index += 1,
                        '.' if !decimal => {
                            decimal = true;
                            index += 1;
                        }
                        _ => break,
                    }
                }
                let text = characters[start..index].iter().collect::<String>();
                if text == "." {
                    return None;
                }
                tokens.push(Token::Number(text.parse().ok()?));
            }
            letter if letter.is_ascii_alphabetic() => {
                let start = index;
                while characters
                    .get(index)
                    .is_some_and(|next| next.is_ascii_alphabetic())
                {
                    index += 1;
                }
                let word = characters[start..index]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();
                tokens.push(match word.as_str() {
                    "pi" => Token::Number(std::f64::consts::PI),
                    "e" => Token::Number(std::f64::consts::E),
                    "sqrt" => Token::Function(Function::Sqrt),
                    "abs" => Token::Function(Function::Abs),
                    "ln" => Token::Function(Function::Ln),
                    "log" => Token::Function(Function::Log),
                    "exp" => Token::Function(Function::Exp),
                    _ => return None,
                });
            }
            _ => return None,
        }
    }
    Some(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.position).copied()
    }

    fn consume(&mut self, token: Token) -> bool {
        if self.peek() == Some(token) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            if self.consume(Token::Plus) {
                value += self.term()?;
            } else if self.consume(Token::Minus) {
                value -= self.term()?;
            } else {
                return Some(value);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if self.consume(Token::Times) {
                value *= self.unary()?;
            } else if self.consume(Token::Divide) {
                let divisor = self.unary()?;
                if divisor == 0.0 {
                    return None;
                }
                value /= divisor;
            } else {
                return Some(value);
            }
        }
    }

    /// Signs bind looser than powers: -2^2 = -4.
    fn unary(&mut self) -> Option<f64> {
        if self.consume(Token::Plus) {
            return self.unary();
        }
        if self.consume(Token::Minus) {
            return self.unary().map(|value| -value);
        }
        self.power()
    }

    /// Right-associative: 2^3^2 = 2^9.
    fn power(&mut self) -> Option<f64> {
        let base = self.primary()?;
        if self.consume(Token::Power) {
            let exponent = self.unary()?;
            let value = base.powf(exponent);
            return value.is_finite().then_some(value);
        }
        Some(base)
    }

    fn primary(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Number(value) => {
                self.position += 1;
                Some(value)
            }
            Token::Open => {
                self.position += 1;
                let value = self.expression()?;
                self.consume(Token::Close).then_some(value)
            }
            Token::Function(function) => {
                self.position += 1;
                if !self.consume(Token::Open) {
                    return None;
                }
                let argument = self.expression()?;
                if !self.consume(Token::Close) {
                    return None;
                }
                let value = match function {
                    Function::Sqrt if argument >= 0.0 => argument.sqrt(),
                    Function::Abs => argument.abs(),
                    Function::Ln if argument > 0.0 => argument.ln(),
                    Function::Log if argument > 0.0 => argument.log10(),
                    Function::Exp => argument.exp(),
                    _ => return None,
                };
                value.is_finite().then_some(value)
            }
            _ => None,
        }
    }
}
