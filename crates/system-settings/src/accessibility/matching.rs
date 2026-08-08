//! Navigation search matching and identifier/action validation.

use std::collections::HashSet;

use super::model::*;

pub fn category_matches(
    query: &str,
    category_name: &str,
    category_description: &str,
    search_terms: &[&str],
) -> bool {
    if query.len() > MAX_QUERY_BYTES {
        return false;
    }
    let query_terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    if query_terms.is_empty() {
        return true;
    }
    let fields = [category_name, category_description]
        .into_iter()
        .chain(search_terms.iter().copied())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    query_terms.iter().all(|query_term| {
        fields
            .iter()
            .any(|field| text_has_word_prefix(field, query_term))
    })
}

pub fn category_match_hint<'a>(query: &str, search_terms: &'a [&'a str]) -> Option<&'a str> {
    if query.len() > MAX_QUERY_BYTES {
        return None;
    }
    let query_terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    if query_terms.is_empty() {
        return None;
    }
    search_terms
        .iter()
        .copied()
        .filter_map(|term| {
            let lower = term.to_lowercase();
            let score = query_terms
                .iter()
                .filter(|query_term| text_has_word_prefix(&lower, query_term))
                .count();
            (score != 0).then_some((score, term))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, term)| term)
}

pub(super) fn text_has_word_prefix(text: &str, query_term: &str) -> bool {
    text.split(|character: char| !character.is_alphanumeric())
        .any(|word| !word.is_empty() && word.starts_with(query_term))
}

pub(super) fn validate_pane_id(pane_id: &str) -> Result<(), AccessibilityProjectionError> {
    if pane_id.is_empty()
        || pane_id.len() > 128
        || pane_id
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    {
        return Err(AccessibilityProjectionError::InvalidCategory);
    }
    Ok(())
}

pub(super) fn validate_actions(actions: &[String]) -> Result<(), AccessibilityProjectionError> {
    let mut seen = HashSet::with_capacity(actions.len());
    if actions.iter().any(|action| !seen.insert(action.as_str())) {
        return Err(AccessibilityProjectionError::InvalidAction);
    }
    Ok(())
}
