//! Ranked catalog search: exact, prefix, word-prefix, substring and
//! subsequence matching, in that order of quality.
//!
//! This mirrors the grading `rmac_launcher::engine::score` uses for
//! Spotlight, kept as a small local copy rather than a dependency on that
//! crate — `rmac-launcher` also pulls in `rmac-compositor` and shell
//! invocation for its own session/window handling, which App Drawer's
//! catalog search has no other reason to depend on.

/// The best match quality across `haystack`'s `\n`-separated fields (each
/// already lowercased — see `rmac_apps::Application::searchable_text`).
/// Higher is a stronger match; `None` means `query` doesn't match at all.
/// An empty query matches everything, ranked lowest so it never reorders a
/// non-empty search's results.
pub(crate) fn match_score(query: &str, haystack: &str) -> Option<u32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    let query = query.to_lowercase();
    haystack
        .split('\n')
        .filter_map(|field| field_score(&query, field))
        .max()
}

fn field_score(query: &str, field: &str) -> Option<u32> {
    if field == query {
        Some(1_000)
    } else if field.starts_with(query) {
        Some(850)
    } else if field
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        Some(700)
    } else if field.contains(query) {
        Some(550)
    } else if is_subsequence(query, field) {
        Some(300)
    } else {
        None
    }
}

/// Does `value` contain every character of `query`, in order, allowing gaps?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything_at_the_lowest_rank() {
        assert_eq!(match_score("", "calculator\nmath"), Some(0));
        assert_eq!(match_score("   ", "anything"), Some(0));
    }

    #[test]
    fn exact_beats_prefix_beats_word_prefix_beats_substring_beats_subsequence() {
        let exact = match_score("calculator", "calculator\ncalc").unwrap();
        let prefix = match_score("calc", "calculator").unwrap();
        let word_prefix = match_score("term", "app terminal").unwrap();
        let substring = match_score("cula", "calculator").unwrap();
        let subsequence = match_score("cltr", "calculator").unwrap();
        assert!(exact > prefix);
        assert!(prefix > word_prefix);
        assert!(word_prefix > substring);
        assert!(substring > subsequence);
    }

    #[test]
    fn matches_any_field_not_just_the_first() {
        // "calculator" is the second `\n`-separated field (a keyword).
        assert!(match_score("calc", "math tool\ncalculator\nutilities").is_some());
    }

    #[test]
    fn non_matches_return_none() {
        assert_eq!(match_score("xyz", "calculator\nmath"), None);
    }

    #[test]
    fn case_is_ignored_in_the_query() {
        assert_eq!(
            match_score("CALC", "calculator"),
            match_score("calc", "calculator")
        );
    }
}
