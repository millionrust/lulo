//! Shared Edit ▸ Spelling and Grammar / Substitutions behaviour for
//! editable text fields.
//!
//! This module holds the parts that need no dictionary at all (document
//! scanning, the live-typing hook, smart quotes/dashes, text replacement)
//! plus the [`SpellChecker`] trait a dictionary-backed checker implements.
//! The dictionary itself (hunspell files via the `spellbook` crate) lives
//! in the separate `rmac-spelling` crate, kept out of `rmac-ui` so the
//! shell binaries that depend on `rmac-ui` (the menu bar, dock, …) never
//! pull in a spell-checking dependency they have no use for; only the four
//! apps with a Spelling and Grammar menu (Clock, Notes, Preview, Text
//! Editor) depend on it directly.
//!
//! There is no squiggly-underline presentation here: gpui-component's
//! `InputState` only exposes diagnostic ranges in its `CodeEditor` input
//! mode (see `vendor/gpui-component/crates/ui/src/input/mode.rs`), which
//! Notes/Text Editor/Clock/Preview's plain and rich-text fields do not use
//! and switching them to it would pull in syntax/LSP semantics nobody
//! wants here. Until gpui-component exposes a decoration hook on ordinary
//! text fields, the real, non-stub behaviour this module offers instead
//! is: jumping the selection to the next misspelled or repeated word
//! (Check Document Now / Show Spelling and Grammar), and silently fixing
//! one as you finish typing it (Correct Spelling Automatically). See
//! docs/parity.md.

use std::ops::Range;

use gpui::{App, Entity, EntityInputHandler as _, Window};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::InputState;

/// A dictionary-backed spelling checker. Implemented by `rmac-spelling`'s
/// `HunspellChecker`; kept as a trait here so `rmac-ui` never depends on
/// the dictionary crate itself.
pub trait SpellChecker: Send + Sync {
    /// `None` while the dictionary is still loading (fail open: callers
    /// treat an unknown word as correct rather than flashing false
    /// positives while the background load completes).
    fn is_correct(&self, word: &str) -> Option<bool>;
    /// Ranked corrections, best first. Empty when the dictionary is not
    /// ready yet or has nothing to offer.
    fn suggest(&self, word: &str) -> Vec<String>;
}

/// Per-field live-typing and document-check behaviour, one set per
/// editable surface. Mac defaults: spelling/grammar checking and every
/// substitution are on; automatic correction is off (the owner corrects
/// deliberately).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextAssistSettings {
    pub check_spelling_while_typing: bool,
    pub check_grammar_with_spelling: bool,
    pub correct_spelling_automatically: bool,
    pub smart_quotes: bool,
    pub smart_dashes: bool,
    pub text_replacement: bool,
    /// Checked and persisted for the session; no live effect is wired yet
    /// (see docs/parity.md). Smart Copy/Paste's edit-boundary heuristics
    /// and Smart Links' auto-linking need, respectively, a hook into
    /// gpui-component's otherwise-opaque built-in Cut/Paste actions and a
    /// per-app rich-text link model to apply to.
    pub smart_copy_paste: bool,
    pub smart_links: bool,
}

impl Default for TextAssistSettings {
    fn default() -> Self {
        Self {
            check_spelling_while_typing: true,
            check_grammar_with_spelling: false,
            correct_spelling_automatically: false,
            smart_quotes: true,
            smart_dashes: true,
            text_replacement: true,
            smart_copy_paste: true,
            smart_links: true,
        }
    }
}

/// One thing [`next_issue`] found: a misspelling (with suggestions) or a
/// repeated word (Mac's "Check Grammar With Spelling" catches doubled
/// words the same lightweight way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub range: Range<usize>,
    pub word: String,
    pub suggestions: Vec<String>,
    pub grammar: bool,
}

/// The first spelling or (when `include_grammar`) repeated-word issue at
/// or after `from`, wrapping around to the start of `text` when nothing
/// is found before the end — the same "keep pressing Check Document Now
/// and it cycles" behaviour as the Mac.
pub fn next_issue(
    text: &str,
    from: usize,
    checker: &dyn SpellChecker,
    include_grammar: bool,
) -> Option<Issue> {
    let tokens: Vec<(Range<usize>, &str)> = text
        .split_word_bound_indices()
        .filter(|(_, word)| word.chars().next().is_some_and(char::is_alphabetic))
        .map(|(index, word)| (index..index + word.len(), word))
        .collect();
    if tokens.is_empty() {
        return None;
    }
    let scan = |order: &[usize]| -> Option<Issue> {
        for &position in order {
            let (range, word) = &tokens[position];
            if include_grammar {
                if let Some(previous) = position.checked_sub(1) {
                    let (previous_range, previous_word) = &tokens[previous];
                    if previous_word.eq_ignore_ascii_case(word)
                        && text[previous_range.end..range.start]
                            .chars()
                            .all(char::is_whitespace)
                    {
                        return Some(Issue {
                            range: range.clone(),
                            word: (*word).to_owned(),
                            suggestions: Vec::new(),
                            grammar: true,
                        });
                    }
                }
            }
            if checker.is_correct(word) == Some(false) {
                return Some(Issue {
                    range: range.clone(),
                    word: (*word).to_owned(),
                    suggestions: checker.suggest(word),
                    grammar: false,
                });
            }
        }
        None
    };
    let after = (0..tokens.len())
        .filter(|&index| tokens[index].0.start >= from)
        .collect::<Vec<_>>();
    let before = (0..tokens.len())
        .filter(|&index| tokens[index].0.start < from)
        .collect::<Vec<_>>();
    scan(&after).or_else(|| scan(&before))
}

/// Edit ▸ Spelling and Grammar ▸ Check Document Now / Show Spelling and
/// Grammar: select the next issue, if any. Returns whether one was found.
pub fn check_document_now(
    field: &Entity<InputState>,
    checker: &dyn SpellChecker,
    include_grammar: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let (text, from) = {
        let input = field.read(cx);
        (input.text().to_string(), input.selected_range().end)
    };
    let Some(issue) = next_issue(&text, from, checker, include_grammar) else {
        return false;
    };
    field.update(cx, |input, cx| {
        input.set_selected_range(issue.range, cx);
        input.focus(window, cx);
    });
    true
}

/// Call from the field's `InputEvent::Change` subscription. Reacts only to
/// the single character or word just finished — a plain quote/hyphen
/// becomes its smart equivalent immediately, and the word before a
/// just-typed space/punctuation is checked against the replacement table
/// and (when enabled) the dictionary. Never touches an active selection.
pub fn on_text_changed(
    field: &Entity<InputState>,
    settings: TextAssistSettings,
    checker: Option<&dyn SpellChecker>,
    window: &mut Window,
    cx: &mut App,
) {
    let (text, cursor) = {
        let input = field.read(cx);
        let range = input.selected_range();
        if range.start != range.end {
            return;
        }
        (input.text().to_string(), range.start)
    };
    if (settings.smart_quotes || settings.smart_dashes)
        && apply_smart_substitution(field, &text, cursor, settings, window, cx)
    {
        return;
    }
    if !settings.text_replacement && !settings.check_spelling_while_typing {
        return;
    }
    let Some(boundary_at) = cursor.checked_sub(1).filter(|&index| {
        text.get(index..cursor)
            .is_some_and(|slice| is_word_boundary(slice))
    }) else {
        return;
    };
    if settings.text_replacement {
        if let Some((range, replacement)) = matching_replacement(&text, boundary_at) {
            replace_and_place_cursor_after(field, range, replacement, window, cx);
            return;
        }
    }
    if !settings.check_spelling_while_typing {
        return;
    }
    let Some(checker) = checker else { return };
    let Some(word_range) = word_before(&text, boundary_at) else {
        return;
    };
    let word = &text[word_range.clone()];
    if checker.is_correct(word) != Some(false) {
        return;
    }
    if settings.correct_spelling_automatically {
        if let Some(best) = checker.suggest(word).into_iter().next() {
            replace_and_place_cursor_after(field, word_range, &best, window, cx);
        }
    }
    // Else: flagged but left alone, matching the Mac with no underline
    // available to show it (see the module doc).
}

fn is_word_boundary(slice: &str) -> bool {
    slice.chars().next().is_some_and(|ch| {
        ch.is_whitespace() || matches!(ch, '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}')
    })
}

fn word_before(text: &str, end: usize) -> Option<Range<usize>> {
    let mut start = end;
    for ch in text[..end].chars().rev() {
        if ch.is_alphanumeric() || ch == '\'' || ch == '\u{2019}' {
            start -= ch.len_utf8();
        } else {
            break;
        }
    }
    (start < end).then(|| start..end)
}

/// A small, real seed of text replacements applied while typing (System
/// Settings has no Text Replacements pane yet to grow this list from; see
/// docs/parity.md SET-44). Keys are matched exactly, case-sensitively,
/// ending right at the boundary a space/punctuation character just
/// completed.
const REPLACEMENTS: &[(&str, &str)] = &[("(c)", "©"), ("(r)", "®"), ("(tm)", "™")];

fn matching_replacement(text: &str, end: usize) -> Option<(Range<usize>, &'static str)> {
    REPLACEMENTS.iter().find_map(|(key, value)| {
        let start = end.checked_sub(key.len())?;
        (text.get(start..end) == Some(*key)).then_some((start..end, *value))
    })
}

/// Replace `range` (UTF-8 byte offsets) with `replacement` and collapse the
/// selection right after it. `InputState::replace_text_in_range`'s own
/// range parameter is UTF-16 code units (it feeds gpui-component's IME
/// path), so this selects `range` first — documented as UTF-8 bytes —
/// and replaces `None` (the just-set selection) instead of ever handing it
/// a byte range to reinterpret as UTF-16.
fn replace_and_place_cursor_after(
    field: &Entity<InputState>,
    range: Range<usize>,
    replacement: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let end = range.start + replacement.len();
    field.update(cx, |input, cx| {
        input.set_selected_range(range, cx);
        input.replace_text_in_range(None, replacement, window, cx);
        input.set_selected_range(end..end, cx);
    });
}

/// Smart Quotes/Dashes: the character just typed, inspected in place
/// (never reaching back past it), decides its own replacement. Returns
/// whether a substitution was made.
fn apply_smart_substitution(
    field: &Entity<InputState>,
    text: &str,
    cursor: usize,
    settings: TextAssistSettings,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(last) = text[..cursor].chars().next_back() else {
        return false;
    };
    if settings.smart_dashes && last == '-' {
        let before_last = cursor - last.len_utf8();
        if let Some(previous) = text[..before_last].chars().next_back() {
            let replacement = match previous {
                '-' => Some("\u{2013}"),        // "--" -> en dash
                '\u{2013}' => Some("\u{2014}"), // "<en dash>-" -> em dash
                _ => None,
            };
            if let Some(replacement) = replacement {
                let start = before_last - previous.len_utf8();
                replace_and_place_cursor_after(field, start..cursor, replacement, window, cx);
                return true;
            }
        }
        return false;
    }
    if settings.smart_quotes && (last == '"' || last == '\'') {
        let quote_start = cursor - last.len_utf8();
        let opening = text[..quote_start]
            .chars()
            .next_back()
            .is_none_or(|previous| {
                previous.is_whitespace()
                    || matches!(previous, '(' | '[' | '{' | '\u{2018}' | '\u{201c}')
            });
        let replacement = match (last, opening) {
            ('"', true) => "\u{201c}",
            ('"', false) => "\u{201d}",
            ('\'', true) => "\u{2018}",
            ('\'', false) => "\u{2019}",
            _ => unreachable!(),
        };
        replace_and_place_cursor_after(field, quote_start..cursor, replacement, window, cx);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeChecker {
        correct: fn(&str) -> bool,
    }

    impl SpellChecker for FakeChecker {
        fn is_correct(&self, word: &str) -> Option<bool> {
            Some((self.correct)(word))
        }
        fn suggest(&self, word: &str) -> Vec<String> {
            if word.eq_ignore_ascii_case("helllo") {
                vec!["hello".to_owned()]
            } else {
                Vec::new()
            }
        }
    }

    fn checker() -> FakeChecker {
        FakeChecker {
            correct: |word| !word.eq_ignore_ascii_case("helllo"),
        }
    }

    #[test]
    fn next_issue_finds_the_misspelling_after_the_cursor_and_wraps() {
        let text = "helllo there helllo";
        let checker = checker();
        let first = next_issue(text, 0, &checker, false).unwrap();
        assert_eq!(first.word, "helllo");
        assert_eq!(first.range, 0..6);
        assert!(first.suggestions.contains(&"hello".to_owned()));

        let second = next_issue(text, 7, &checker, false).unwrap();
        assert_eq!(second.range, 13..19);

        // Nothing left after the last misspelling: wraps to the first.
        let wrapped = next_issue(text, 19, &checker, false).unwrap();
        assert_eq!(wrapped.range, 0..6);
    }

    #[test]
    fn next_issue_flags_a_repeated_word_only_when_grammar_checking_is_on() {
        let text = "the the cat sat";
        let checker = checker();
        assert_eq!(next_issue(text, 0, &checker, false), None);
        let repeat = next_issue(text, 0, &checker, true).unwrap();
        assert!(repeat.grammar);
        assert_eq!(repeat.word, "the");
        assert_eq!(repeat.range, 4..7);
    }

    #[test]
    fn matching_replacement_only_fires_on_an_exact_boundary_match() {
        assert_eq!(matching_replacement("Patented (c)", 12), Some((9..12, "©")));
        assert_eq!(matching_replacement("a (css)", 7), None);
    }

    #[test]
    fn word_before_stops_at_the_first_non_word_character() {
        assert_eq!(word_before("don't stop", 5), Some(0..5));
        assert_eq!(word_before("  hi", 4), Some(2..4));
        assert_eq!(word_before("", 0), None);
    }
}
