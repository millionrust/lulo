//! Deterministic local calculator launcher provider.

use super::*;

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
        let Some(value) = evaluate(query) else {
            return Ok(Vec::new());
        };
        let text = format_number(value);
        Ok(vec![SearchResult {
            id: ResultId {
                provider: provider_id(CALCULATOR_PROVIDER),
                local: query.trim().into(),
            },
            category: Category::Calculator,
            title: text.clone(),
            subtitle: Some(query.trim().into()),
            icon: None,
            primary: Action::CopyText { text },
            alternate: None,
            recency_rank: 0,
        }])
    }
}

fn evaluate(input: &str) -> Option<f64> {
    let input = input.trim();
    if input.is_empty()
        || input.len() > 256
        || !input
            .chars()
            .any(|character| matches!(character, '+' | '-' | '*' | '/'))
    {
        return None;
    }
    let mut parser = Parser::new(input);
    let value = parser.expression()?;
    parser.skip_whitespace();
    (parser.position == parser.input.len() && value.is_finite()).then_some(value)
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
        }
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            self.skip_whitespace();
            if self.consume(b'+') {
                value += self.term()?;
            } else if self.consume(b'-') {
                value -= self.term()?;
            } else {
                return Some(value);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.factor()?;
        loop {
            self.skip_whitespace();
            if self.consume(b'*') {
                value *= self.factor()?;
            } else if self.consume(b'/') {
                let divisor = self.factor()?;
                if divisor == 0.0 {
                    return None;
                }
                value /= divisor;
            } else {
                return Some(value);
            }
        }
    }

    fn factor(&mut self) -> Option<f64> {
        self.skip_whitespace();
        if self.consume(b'+') {
            return self.factor();
        }
        if self.consume(b'-') {
            return self.factor().map(|value| -value);
        }
        if self.consume(b'(') {
            let value = self.expression()?;
            self.skip_whitespace();
            return self.consume(b')').then_some(value);
        }
        self.number()
    }

    fn number(&mut self) -> Option<f64> {
        self.skip_whitespace();
        let start = self.position;
        let mut decimal = false;
        while let Some(byte) = self.input.get(self.position) {
            match byte {
                b'0'..=b'9' => self.position += 1,
                b'.' if !decimal => {
                    decimal = true;
                    self.position += 1;
                }
                _ => break,
            }
        }
        (self.position > start)
            .then(|| std::str::from_utf8(&self.input[start..self.position]).ok())
            .flatten()?
            .parse()
            .ok()
    }

    fn skip_whitespace(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.input.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }
}

fn format_number(value: f64) -> String {
    let formatted = format!("{value:.10}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".into()
    } else {
        trimmed.into()
    }
}
