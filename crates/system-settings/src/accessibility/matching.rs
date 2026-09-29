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

/// How well `category` ranks for `query`, for ordering search results the
/// way the Mac does: a hit on the pane's own name (e.g. "Wallpaper" for
/// "wallpaper") outranks one that only hit its description, which in turn
/// outranks one that only hit its hidden search vocabulary (a pane like
/// "Desktop & Dock" whose search terms happen to mention "wallpaper" in
/// passing). Lower is better; `None` when `category` doesn't match `query`
/// at all (see [`category_matches`]).
pub fn category_match_rank(
    query: &str,
    category_name: &str,
    category_description: &str,
    search_terms: &[&str],
) -> Option<u8> {
    if !category_matches(query, category_name, category_description, search_terms) {
        return None;
    }
    let query_terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    if query_terms.is_empty() {
        return Some(0);
    }
    let name = category_name.to_lowercase();
    if query_terms
        .iter()
        .all(|term| text_has_word_prefix(&name, term))
    {
        return Some(0);
    }
    let description = category_description.to_lowercase();
    if query_terms
        .iter()
        .all(|term| text_has_word_prefix(&description, term))
    {
        return Some(1);
    }
    Some(2)
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
