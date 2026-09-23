//! Inline completion of the top hit and the Spotlight results sections.
//!
//! macOS completes the query in place with the rest of the top hit's name
//! ("term" + "inal — Open") and lists the other results under a "Top Hit"
//! section followed by one section per kind.

use std::ops::Range;

/// Section title macOS uses for the first result of a query.
pub(crate) const TOP_HIT_SECTION: &str = "Top Hit";

/// Longest query that still shows an inline completion. Beyond this the
/// field may scroll horizontally, and the completion would drift away from
/// the typed text.
const MAX_COMPLETED_QUERY_CHARS: usize = 32;

/// The rest of `title` after `query` when `title` starts with `query`,
/// ignoring case. An exact match completes with an empty suffix so the
/// action hint still shows. Returns `None` for an empty query or a title
/// that does not start with it.
pub(crate) fn inline_completion<'a>(query: &str, title: &'a str) -> Option<&'a str> {
    if query.is_empty() || query.chars().count() > MAX_COMPLETED_QUERY_CHARS {
        return None;
    }
    let mut title_chars = title.char_indices();
    for query_char in query.chars() {
        let (_, title_char) = title_chars.next()?;
        if !query_char.to_lowercase().eq(title_char.to_lowercase()) {
            return None;
        }
    }
    let consumed = title_chars.next().map_or(title.len(), |(index, _)| index);
    Some(&title[consumed..])
}

/// The text drawn on the completion plate: the remaining name, then the
/// primary action ("inal — Open").
pub(crate) fn completion_label(suffix: &str, action: &str) -> String {
    if action.is_empty() {
        suffix.to_owned()
    } else {
        format!("{suffix} — {action}")
    }
}

/// Splits query results into sections: the first row alone under "Top Hit",
/// then consecutive rows sharing a category label under that label.
pub(crate) fn query_sections(labels: &[&'static str]) -> Vec<(&'static str, Range<usize>)> {
    let mut sections: Vec<(&'static str, Range<usize>)> = Vec::new();
    if labels.is_empty() {
        return sections;
    }
    sections.push((TOP_HIT_SECTION, 0..1));
    for (index, label) in labels.iter().copied().enumerate().skip(1) {
        match sections.last_mut() {
            Some((last, range)) if index > 1 && *last == label => range.end = index + 1,
            _ => sections.push((label, index..index + 1)),
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_the_rest_of_the_top_hit_ignoring_case() {
        assert_eq!(inline_completion("term", "Terminal"), Some("inal"));
        assert_eq!(inline_completion("TERM", "Terminal"), Some("inal"));
        assert_eq!(inline_completion("Terminal", "Terminal"), Some(""));
        assert_eq!(inline_completion("fin", "Finder"), Some("der"));
    }

    #[test]
    fn declines_non_prefix_and_empty_queries() {
        assert_eq!(inline_completion("", "Terminal"), None);
        assert_eq!(inline_completion("minal", "Terminal"), None);
        assert_eq!(inline_completion("Terminals", "Terminal"), None);
        let long = "a".repeat(MAX_COMPLETED_QUERY_CHARS + 1);
        assert_eq!(inline_completion(&long, &long), None);
    }

    #[test]
    fn completion_respects_character_boundaries() {
        assert_eq!(
            inline_completion("réglages", "Réglages Système"),
            Some(" Système")
        );
        assert_eq!(inline_completion("é", "Éditeur"), Some("diteur"));
        assert_eq!(inline_completion("日本", "日本語"), Some("語"));
    }

    #[test]
    fn completion_label_appends_the_action() {
        assert_eq!(completion_label("inal", "Open"), "inal — Open");
        assert_eq!(completion_label("", "Open"), " — Open");
        assert_eq!(completion_label("inal", ""), "inal");
    }

    #[test]
    fn sections_put_the_first_result_alone_under_top_hit() {
        assert!(query_sections(&[]).is_empty());
        assert_eq!(
            query_sections(&["Applications"]),
            vec![(TOP_HIT_SECTION, 0..1)]
        );
        assert_eq!(
            query_sections(&[
                "Applications",
                "Applications",
                "Applications",
                "Settings",
                "Files",
                "Files",
            ]),
            vec![
                (TOP_HIT_SECTION, 0..1),
                ("Applications", 1..3),
                ("Settings", 3..4),
                ("Files", 4..6),
            ]
        );
    }
}
