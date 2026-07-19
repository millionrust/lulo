use std::fmt;
use std::ops::Range;

pub const MAX_SEARCH_TITLE_FRAGMENT_CHARS: usize = 160;
pub const MAX_SEARCH_DETAIL_FRAGMENT_CHARS: usize = 120;
pub const MAX_SEARCH_LABEL_FRAGMENT_CHARS: usize = 80;

#[derive(Clone, PartialEq, Eq)]
pub struct SearchTextFragment {
    text: String,
    highlight: Option<Range<usize>>,
}

impl SearchTextFragment {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn highlight(&self) -> Option<Range<usize>> {
        self.highlight.clone()
    }

    pub fn with_prefix(mut self, prefix: &str) -> Self {
        if prefix.is_empty() {
            return self;
        }
        let prefix_bytes = prefix.len();
        self.text.insert_str(0, prefix);
        self.highlight = self
            .highlight
            .map(|range| range.start + prefix_bytes..range.end + prefix_bytes);
        self
    }
}

impl fmt::Debug for SearchTextFragment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchTextFragment")
            .field("text_bytes", &self.text.len())
            .field("highlight", &self.highlight)
            .finish()
    }
}

pub fn plain_search_fragment(source: &str, max_chars: usize) -> SearchTextFragment {
    if max_chars == 0 {
        return SearchTextFragment {
            text: String::new(),
            highlight: None,
        };
    }
    let end = boundary_after_chars(source, 0, max_chars);
    let mut text = String::new();
    push_compacted(&mut text, &source[..end]);
    if end < source.len() {
        push_ellipsis(&mut text, false);
    }
    SearchTextFragment {
        text,
        highlight: None,
    }
}

pub fn matched_search_fragment(
    source: &str,
    span: Range<usize>,
    max_chars: usize,
) -> Option<SearchTextFragment> {
    if max_chars == 0
        || span.start >= span.end
        || span.end > source.len()
        || !source.is_char_boundary(span.start)
        || !source.is_char_boundary(span.end)
    {
        return None;
    }

    let match_chars = source[span.clone()].chars().count();
    let visible_match_chars = match_chars.min(max_chars);
    let visible_match_end =
        boundary_after_chars(source, span.start, visible_match_chars).min(span.end);
    let context_budget = max_chars.saturating_sub(visible_match_chars);
    let desired_before = context_budget / 2;
    let visible_start = boundary_before_chars(source, span.start, desired_before);
    let actual_before = source[visible_start..span.start].chars().count();
    let desired_after = context_budget.saturating_sub(actual_before);
    let visible_end = boundary_after_chars(source, visible_match_end, desired_after);

    let mut text = String::new();
    if visible_start != 0 {
        push_ellipsis(&mut text, true);
    }
    push_compacted(&mut text, &source[visible_start..span.start]);
    let highlight_start = text.len();
    push_compacted(&mut text, &source[span.start..visible_match_end]);
    let highlight_end = text.len();
    push_compacted(&mut text, &source[visible_match_end..visible_end]);
    if visible_end < source.len() {
        push_ellipsis(&mut text, false);
    }
    (highlight_start < highlight_end).then_some(SearchTextFragment {
        text,
        highlight: Some(highlight_start..highlight_end),
    })
}

fn boundary_before_chars(source: &str, end: usize, count: usize) -> usize {
    if count == 0 {
        return end;
    }
    source[..end]
        .char_indices()
        .rev()
        .nth(count.saturating_sub(1))
        .map_or(0, |(index, _)| index)
}

fn boundary_after_chars(source: &str, start: usize, count: usize) -> usize {
    if count == 0 {
        return start;
    }
    source[start..]
        .char_indices()
        .nth(count)
        .map_or(source.len(), |(index, _)| start + index)
}

fn push_compacted(target: &mut String, source: &str) {
    for character in source.chars() {
        if character.is_whitespace() || character.is_control() {
            if !target.is_empty() && !target.ends_with(' ') {
                target.push(' ');
            }
        } else {
            target.push(character);
        }
    }
}

fn push_ellipsis(target: &mut String, leading: bool) {
    if leading {
        target.push('…');
        if !target.ends_with(' ') {
            target.push(' ');
        }
    } else {
        while target.ends_with(' ') {
            target.pop();
        }
        target.push_str(" …");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_unicode_span_into_a_bounded_visible_fragment() {
        let source = "alpha βeta\nprivate café omega";
        let start = source.find("café").unwrap();
        let fragment = matched_search_fragment(source, start..start + "café".len(), 14).unwrap();
        let range = fragment.highlight().unwrap();
        assert_eq!(&fragment.text()[range], "café");
        assert!(fragment.text().chars().count() <= 18);
        assert!(!fragment.text().contains('\n'));
    }

    #[test]
    fn far_body_match_does_not_copy_the_complete_source() {
        let source = format!(
            "{}needle{}",
            "before ".repeat(10_000),
            " after".repeat(10_000)
        );
        let start = source.find("needle").unwrap();
        let fragment = matched_search_fragment(
            &source,
            start..start + "needle".len(),
            MAX_SEARCH_DETAIL_FRAGMENT_CHARS,
        )
        .unwrap();
        assert!(fragment.text().starts_with('…'));
        assert!(fragment.text().ends_with('…'));
        assert!(fragment.text().len() < 512);
    }

    #[test]
    fn rejects_invalid_or_non_boundary_spans() {
        let source = "café";
        assert!(matched_search_fragment(source, 4..5, 20).is_none());
        assert!(matched_search_fragment(source, 3..4, 20).is_none());
    }

    #[test]
    fn fragment_debug_redacts_private_text() {
        let fragment = plain_search_fragment("private note body", 80);
        let debug = format!("{fragment:?}");
        assert!(!debug.contains("private"));
        assert!(!debug.contains("note body"));
    }
}
