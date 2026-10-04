//! Mail search terms shared by the cache and the fixture-backed window.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchField {
    Any,
    From,
    To,
    Subject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTerm {
    pub field: SearchField,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub terms: Vec<SearchTerm>,
}

impl SearchQuery {
    /// Understands Mail's From, To and Subject tokens, including quoted values.
    /// Unrecognised prefixes are searched as ordinary text.
    pub fn parse(input: &str) -> Self {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quoted = false;
        for ch in input.chars() {
            match ch {
                '"' => quoted = !quoted,
                ch if ch.is_whitespace() && !quoted => {
                    if !word.is_empty() {
                        words.push(std::mem::take(&mut word));
                    }
                }
                _ => word.push(ch),
            }
        }
        if !word.is_empty() {
            words.push(word);
        }
        let mut terms: Vec<SearchTerm> = Vec::new();
        let mut pending = None;
        for word in words {
            let (field, value) = match word.split_once(':') {
                Some((name, value)) if name.eq_ignore_ascii_case("from") => {
                    (SearchField::From, value)
                }
                Some((name, value)) if name.eq_ignore_ascii_case("to") => (SearchField::To, value),
                Some((name, value)) if name.eq_ignore_ascii_case("subject") => {
                    (SearchField::Subject, value)
                }
                _ => (SearchField::Any, word.as_str()),
            };
            if value.is_empty() {
                // Mail accepts `from: Anna` as well as `from:Anna`.
                pending = Some(field);
            } else {
                terms.push(SearchTerm {
                    field: if field == SearchField::Any {
                        pending.take().unwrap_or(field)
                    } else {
                        pending = None;
                        field
                    },
                    value: value.to_owned(),
                });
            }
        }
        Self { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn matches_fields(
        &self,
        sender: &str,
        recipients: &str,
        subject: &str,
        body: &str,
    ) -> bool {
        self.terms.iter().all(|term| {
            let needle = term.value.to_lowercase();
            match term.field {
                SearchField::From => sender.to_lowercase().contains(&needle),
                SearchField::To => recipients.to_lowercase().contains(&needle),
                SearchField::Subject => subject.to_lowercase().contains(&needle),
                SearchField::Any => [sender, recipients, subject, body]
                    .iter()
                    .any(|field| field.to_lowercase().contains(&needle)),
            }
        })
    }

    pub(crate) fn fts_expression(&self) -> String {
        self.terms
            .iter()
            .map(|term| {
                let column = match term.field {
                    SearchField::Any => "",
                    SearchField::From => "sender:",
                    SearchField::To => "recipients:",
                    SearchField::Subject => "subject:",
                };
                format!("{column}\"{}\"", term.value.replace('"', "\"\""))
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}
