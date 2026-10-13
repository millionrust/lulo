//! Edit ▸ Complete (⌥⎋) for editable text.
//!
//! Cocoa's `complete:` offers words that begin with what has been typed.
//! Here the first press completes the word before the caret with the best
//! match, shown selected after the caret so typing on replaces it; each
//! further press moves to the next match, and after the last one back to
//! exactly what was typed. Matches are the words already in the document,
//! most used first, then words from the system word list when the machine
//! has one (`/usr/share/dict/words`), so a half-typed long word completes
//! even the first time it appears.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader};
use std::ops::Range;
use std::path::Path;

use gpui::{App, Entity, Window};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::text_assist::EditableText;

/// The most completions offered for one word.
const MAX_CANDIDATES: usize = 30;
/// How much of a system word list is read looking for matches.
const MAX_DICTIONARY_BYTES: usize = 4 * 1024 * 1024;
/// The system word list, when installed.
const SYSTEM_WORD_LIST: &str = "/usr/share/dict/words";

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '\'' || character == '’' || character == '_'
}

/// Where the word being typed starts: the offset of its first character,
/// scanning back from `caret` (a byte offset on a character boundary).
fn word_start(text: &str, caret: usize) -> usize {
    let mut start = caret;
    for (index, character) in text[..caret].char_indices().rev() {
        if is_word_char(character) {
            start = index;
        } else {
            break;
        }
    }
    start
}

/// Whole words of `text`, with how often each occurs, leaving out the word
/// being typed (the one that ends exactly at `caret`).
fn document_words(text: &str, caret: usize) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for (offset, word) in text.split_word_bound_indices() {
        if !word.chars().next().is_some_and(is_word_char) {
            continue;
        }
        if offset + word.len() == caret {
            continue;
        }
        *counts.entry(word.to_owned()).or_insert(0) += 1;
    }
    counts
}

/// Case-insensitive prefix test that treats a curly apostrophe as straight.
fn starts_with_ignore_case(word: &str, prefix: &str) -> bool {
    let normal = |c: char| if c == '’' { '\'' } else { c };
    let mut word_chars = word.chars().map(normal).flat_map(char::to_lowercase);
    prefix
        .chars()
        .map(normal)
        .flat_map(char::to_lowercase)
        .all(|expected| word_chars.next() == Some(expected))
}

/// The completions for the word before `caret`, best first: document words
/// by use then alphabetically, then `dictionary` words alphabetically.
/// Words are returned complete (including the typed prefix) and never equal
/// the prefix. Empty when no word is being typed.
pub fn candidates(text: &str, caret: usize, dictionary: &[String]) -> (usize, Vec<String>) {
    if caret > text.len() || !text.is_char_boundary(caret) {
        return (caret, Vec::new());
    }
    let start = word_start(text, caret);
    let prefix = &text[start..caret];
    if prefix.is_empty() {
        return (start, Vec::new());
    }
    let mut from_document: Vec<(String, usize)> = document_words(text, caret)
        .into_iter()
        .filter(|(word, _)| word.len() > prefix.len() && starts_with_ignore_case(word, prefix))
        .collect();
    from_document.sort_by(|(left, left_count), (right, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left.to_lowercase().cmp(&right.to_lowercase()))
            .then_with(|| left.cmp(right))
    });
    let mut found: Vec<String> = from_document.into_iter().map(|(word, _)| word).collect();
    for word in dictionary {
        if found.len() >= MAX_CANDIDATES {
            break;
        }
        if word.len() > prefix.len()
            && starts_with_ignore_case(word, prefix)
            && !found
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(word))
        {
            found.push(word.clone());
        }
    }
    found.truncate(MAX_CANDIDATES);
    // Keep the case the person typed: `Gra` completes to `Grand`, `gra` to
    // `grand`, whichever case the source word had.
    let mut shown: Vec<String> = Vec::new();
    for word in found {
        let tail: String = word.chars().skip(prefix.chars().count()).collect();
        let completed = format!("{prefix}{tail}");
        // `Grand` and `grand` in the document are one suggestion once the
        // typed case is applied.
        if !shown.contains(&completed) {
            shown.push(completed);
        }
    }
    (start, shown)
}

/// Words from a word list file that begin with `prefix`, read until enough
/// are found or the byte budget is spent. Returns nothing when the file is
/// missing.
pub fn dictionary_matches(path: &Path, prefix: &str) -> Vec<String> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    let mut read = 0usize;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        read += line.len() + 1;
        if read > MAX_DICTIONARY_BYTES || matches.len() >= MAX_CANDIDATES {
            break;
        }
        let word = line.trim();
        if !word.is_empty() && starts_with_ignore_case(word, prefix) {
            matches.push(word.to_owned());
        }
    }
    matches
}

/// Read the system word list once, off the UI thread, so the operating
/// system has it cached by the time Complete is first used and the read on
/// that press is a memory copy. Nothing is kept in memory.
pub fn warm_word_list() {
    static WARMED: std::sync::Once = std::sync::Once::new();
    WARMED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("word-list-warmup".to_owned())
            .spawn(|| {
                let _ = dictionary_matches(Path::new(SYSTEM_WORD_LIST), "\u{0}");
            });
    });
}

/// The state of a run of Complete presses on one word.
#[derive(Clone, Debug)]
pub struct CompletionCycle {
    /// Where the typed prefix ends and the completion begins.
    caret: usize,
    /// The whole-word completions, each beginning with the typed prefix.
    words: Vec<String>,
    prefix_len: usize,
    /// Which word is showing; `words.len()` means none (what was typed).
    position: usize,
    /// The text inserted after the caret for the current position.
    shown: String,
}

impl CompletionCycle {
    fn suffix(&self, position: usize) -> String {
        self.words
            .get(position)
            .map(|word| word[self.prefix_len..].to_owned())
            .unwrap_or_default()
    }

    fn next_position(&self) -> usize {
        match self.position {
            usize::MAX => 0,
            position => (position + 1) % (self.words.len() + 1),
        }
    }

    /// The range of the inserted completion in the document.
    fn shown_range(&self) -> Range<usize> {
        self.caret..self.caret + self.shown.len()
    }
}

/// Edit ▸ Complete: start a completion at the caret or advance `cycle`.
/// Returns false when there is nothing to complete.
pub fn complete<T: EditableText>(
    field: &Entity<T>,
    cycle: &mut Option<CompletionCycle>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let (text, selection) = {
        let editor = field.read(cx);
        (editor.editable_text(), editor.editable_selection())
    };
    // A further press only continues a run while the completion it made is
    // still exactly what is selected.
    let continuing = cycle.as_ref().is_some_and(|run| {
        selection == run.shown_range() && text.get(run.shown_range()) == Some(run.shown.as_str())
    });
    if !continuing {
        *cycle = None;
        if !selection.is_empty() || selection.start > text.len() {
            return false;
        }
        let caret = selection.start;
        // Only the end of a word is completed; inside one there is nothing
        // sensible to add.
        if text
            .get(caret..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(is_word_char)
        {
            return false;
        }
        let start = word_start(&text, caret);
        let prefix = text.get(start..caret).unwrap_or_default().to_owned();
        let dictionary = if prefix.is_empty() {
            Vec::new()
        } else {
            dictionary_matches(Path::new(SYSTEM_WORD_LIST), &prefix)
        };
        let (start, words) = candidates(&text, caret, &dictionary);
        if words.is_empty() {
            return false;
        }
        *cycle = Some(CompletionCycle {
            caret,
            prefix_len: caret - start,
            words,
            position: usize::MAX,
            shown: String::new(),
        });
    }
    let Some(run) = cycle.as_mut() else {
        return false;
    };
    // Next position: the first match, then each further one, then none.
    run.position = run.next_position();
    let replace = run.shown_range();
    let suffix = run.suffix(run.position);
    let inserted = run.caret..run.caret + suffix.len();
    field.update(cx, |editor, cx| {
        editor.replace_editable_range(replace, &suffix, window, cx);
        // Leave the completion selected so typing replaces it and a further
        // press moves on; with none showing, the caret sits after the
        // prefix.
        editor.select_editable_range(inserted, window, cx);
    });
    run.shown = suffix;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_word_before_the_caret_is_the_prefix() {
        assert_eq!(word_start("one two thr", 11), 8);
        assert_eq!(word_start("one two thr", 3), 0);
        assert_eq!(word_start("don’t sto", 11), 8);
        assert_eq!(word_start("end. ", 5), 5);
    }

    #[test]
    fn document_words_are_offered_most_used_first() {
        let text = "apple apricot apple avocado ap";
        let (start, words) = candidates(text, text.len(), &[]);
        assert_eq!(start, 28);
        assert_eq!(words, ["apple", "apricot"]);
    }

    #[test]
    fn the_word_being_typed_is_not_its_own_completion() {
        let (_, words) = candidates("grand gra", 9, &[]);
        assert_eq!(words, ["grand"]);
        let (_, words) = candidates("gra", 3, &[]);
        assert!(words.is_empty());
    }

    #[test]
    fn the_typed_case_is_kept() {
        let (_, words) = candidates("Grandfather grand Gra", 21, &[]);
        assert_eq!(words, ["Grand", "Grandfather"]);
        let (_, words) = candidates("Grandfather gra", 15, &[]);
        assert_eq!(words, ["grandfather"]);
    }

    #[test]
    fn words_that_differ_only_in_case_are_offered_once() {
        let (_, words) = candidates("Grand grand gra", 15, &[]);
        assert_eq!(words, ["grand"]);
    }

    #[test]
    fn dictionary_words_follow_document_words_without_repeats() {
        let dictionary = vec![
            "apple".to_owned(),
            "applesauce".to_owned(),
            "apply".to_owned(),
        ];
        let (_, words) = candidates("apple app", 9, &dictionary);
        assert_eq!(words, ["apple", "applesauce", "apply"]);
    }

    #[test]
    fn nothing_is_offered_between_words_or_off_a_boundary() {
        assert!(candidates("abc ", 4, &[]).1.is_empty());
        assert!(candidates("é", 1, &[]).1.is_empty());
        assert!(candidates("abc", 99, &[]).1.is_empty());
    }

    #[test]
    fn a_word_list_is_searched_by_prefix_and_a_missing_one_is_empty() {
        let directory = std::env::temp_dir().join(format!("rmac-complete-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let list = directory.join("words");
        std::fs::write(&list, "Apple\nbanana\napricot\n\napply\n").unwrap();
        assert_eq!(
            dictionary_matches(&list, "ap"),
            ["Apple", "apricot", "apply"]
        );
        assert!(dictionary_matches(&directory.join("absent"), "ap").is_empty());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn cycling_walks_every_match_then_returns_to_what_was_typed() {
        let words = vec!["apple".to_owned(), "apricot".to_owned()];
        let mut run = CompletionCycle {
            caret: 2,
            prefix_len: 2,
            words,
            position: usize::MAX,
            shown: String::new(),
        };
        let mut shown = Vec::new();
        for _ in 0..4 {
            run.position = run.next_position();
            run.shown = run.suffix(run.position);
            shown.push(run.shown.clone());
        }
        assert_eq!(shown, ["ple", "ricot", "", "ple"]);
        assert_eq!(run.shown_range(), 2..5);
    }
}
