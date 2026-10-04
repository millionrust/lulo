//! Shared Edit ▸ Transformations behavior for editable text fields.

use gpui::{App, Entity, Window};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::text_assist::EditableText;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransformation {
    Uppercase,
    Lowercase,
    Capitalise,
}

fn transformed(text: &str, transformation: TextTransformation) -> String {
    match transformation {
        TextTransformation::Uppercase => text.to_uppercase(),
        TextTransformation::Lowercase => text.to_lowercase(),
        TextTransformation::Capitalise => text
            .split_word_bounds()
            .map(|word| {
                if let Some(first) = word.chars().next().filter(|c| c.is_alphabetic()) {
                    let rest = &word[first.len_utf8()..];
                    let mut capitalised = first.to_uppercase().collect::<String>();
                    capitalised.push_str(&rest.to_lowercase());
                    capitalised
                } else {
                    word.to_owned()
                }
            })
            .collect(),
    }
}

/// Transform the selected text through the field's undoable edit path. The
/// new selection follows the replacement even when Unicode case conversion
/// changes its byte length (for example, `ß` → `SS`).
pub fn transform_selection<T: EditableText>(
    field: &Entity<T>,
    transformation: TextTransformation,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let (range, replacement) = {
        let input = field.read(cx);
        let range = input.editable_selection();
        let text = input.editable_text();
        let Some(selected) = text.get(range.clone()).filter(|text| !text.is_empty()) else {
            return false;
        };
        let replacement = transformed(selected, transformation);
        if replacement == selected {
            return false;
        }
        (range, replacement)
    };
    let end = range.start + replacement.len();
    field.update(cx, |input, cx| {
        input.replace_editable_range(range.clone(), &replacement, window, cx);
        input.select_editable_range(range.start..end, window, cx);
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_transformations_keep_unicode_words_intact() {
        assert_eq!(
            transformed("Straße", TextTransformation::Uppercase),
            "STRASSE"
        );
        assert_eq!(transformed("İ", TextTransformation::Lowercase), "i\u{307}");
        assert_eq!(
            transformed("hELLO 世界, wORLD", TextTransformation::Capitalise),
            "Hello 世界, World"
        );
    }
}
