use std::collections::BTreeMap;

use crate::{enabled_providers, ProviderDescriptor, Request, SearchResult, Session};

#[derive(Clone, Debug, Default)]
pub struct Launcher {
    open: bool,
    session: Session,
}

impl Launcher {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    pub fn open(
        &mut self,
        descriptors: &[ProviderDescriptor],
        policies: &BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Request {
        self.open = true;
        self.session
            .begin(String::new(), enabled_providers(descriptors, policies))
    }

    pub fn set_query(
        &mut self,
        query: impl Into<String>,
        descriptors: &[ProviderDescriptor],
        policies: &BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Option<Request> {
        self.open.then(|| {
            self.session
                .begin(query, enabled_providers(descriptors, policies))
        })
    }

    pub fn escape(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.open = false;
        self.session.cancel();
        true
    }
}

/// Textual relevance, category and recency, plus what was learned from
/// earlier choices (`learned`, at most [`crate::MAX_BOOST`]).
pub(crate) fn score(query: &str, result: &SearchResult, learned: u16) -> Option<u16> {
    let title = normalize(&result.title);
    let subtitle = result
        .subtitle
        .as_deref()
        .map(normalize)
        .unwrap_or_default();
    let textual = if query.is_empty() {
        100
    } else if result.category.matches_any_query() {
        1_000
    } else {
        match_quality(query, &title)
            .or_else(|| match_quality(query, &subtitle).map(|score| score.saturating_sub(80)))?
    };
    Some(
        textual
            .saturating_add(result.category.rank())
            .saturating_add(u16::from(result.recency_rank.min(100)))
            .saturating_add(learned.min(crate::MAX_BOOST)),
    )
}

pub fn query_matches(query: &str, title: &str, subtitle: Option<&str>) -> bool {
    let query = normalize(query);
    query.is_empty()
        || match_quality(&query, &normalize(title)).is_some()
        || subtitle.is_some_and(|subtitle| match_quality(&query, &normalize(subtitle)).is_some())
}

fn match_quality(query: &str, value: &str) -> Option<u16> {
    if value == query {
        Some(1_000)
    } else if value.starts_with(query) {
        Some(850)
    } else if value
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        Some(700)
    } else if value.contains(query) {
        Some(550)
    } else if is_subsequence(query, value) {
        Some(300)
    } else {
        None
    }
}

fn is_subsequence(query: &str, value: &str) -> bool {
    let mut query = query.chars();
    let mut expected = query.next();
    for character in value.chars() {
        if Some(character) == expected {
            expected = query.next();
            if expected.is_none() {
                return true;
            }
        }
    }
    expected.is_none()
}

pub(crate) fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
