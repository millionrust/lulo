//! Edit ▸ Spelling and Grammar / Substitutions / Speech over the note
//! body — the shared behaviour in `rmac_ui::text_assist`/`rmac_ui::speech`
//! and the dictionary in `rmac_spelling`, applied to Notes' one prose
//! field the same way `note_format_controller`'s Transformations already
//! does (`body_format_editable` gates both). Check Spelling While Typing
//! marks misspelled words with the Mac's red dots, a moment after typing
//! stops, checked off the UI thread.

use std::ops::Range;

use rmac_ui::text_assist::SpellChecker;

use super::*;

/// How long typing must pause before the body is spell-checked again.
const SPELL_CHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(400);

/// The misspelled words of `text`, leaving out the word the insertion point
/// is in (it may still be being typed).
pub(super) fn misspelled_ranges(
    text: &str,
    caret: usize,
    checker: &dyn SpellChecker,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut word_start: Option<usize> = None;
    let check = |range: Range<usize>, ranges: &mut Vec<Range<usize>>| {
        let word = text[range.clone()].trim_end_matches('\'');
        let range = range.start..range.start + word.len();
        let typing = range.start <= caret && caret <= range.end;
        if !typing
            && word.chars().any(char::is_alphabetic)
            && !word.chars().any(|character| character.is_ascii_digit())
            && checker.is_correct(word) == Some(false)
        {
            ranges.push(range);
        }
    };
    for (at, character) in text.char_indices() {
        let in_word = character.is_alphabetic()
            || (character == '\'' && word_start.is_some())
            || (character.is_ascii_digit() && word_start.is_some());
        match (in_word, word_start) {
            (true, None) => word_start = Some(at),
            (false, Some(start)) => {
                check(start..at, &mut ranges);
                word_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = word_start {
        check(start..text.len(), &mut ranges);
    }
    ranges
}

/// Smart Links: the address a just-finished word stands for, if it is one
/// (`https://…`, `www.…`, or a mail address).
pub(super) fn smart_link_target(word: &str) -> Option<String> {
    let lower = word.to_ascii_lowercase();
    if ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme) && word.len() > scheme.len() + 2)
    {
        return Some(word.to_owned());
    }
    if lower.starts_with("www.") && word[4..].contains('.') {
        return Some(format!("https://{word}"));
    }
    let (user, domain) = word.split_once('@')?;
    let plausible = !user.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || "@._-+".contains(c));
    plausible.then(|| format!("mailto:{word}"))
}

impl NotesView {
    /// Smart Links: a web or mail address followed by a space or Return
    /// becomes a link, as the Mac's Notes does.
    pub(super) fn apply_smart_link(&mut self, cx: &mut Context<Self>) {
        if !self.text_assist.smart_links || !self.body_format_editable() {
            return;
        }
        let (text, selection) = {
            let editor = self.body.read(cx);
            (editor.text(), editor.selected_range())
        };
        let caret = selection.start;
        if !selection.is_empty() || caret == 0 {
            return;
        }
        let Some(typed) = text[..caret].chars().next_back() else {
            return;
        };
        if !typed.is_whitespace() {
            return;
        }
        let end = caret - typed.len_utf8();
        let start = text[..end]
            .char_indices()
            .rev()
            .find(|(_, character)| character.is_whitespace())
            .map_or(0, |(at, character)| at + character.len_utf8());
        let word = text[start..end].trim_end_matches(|c: char| ".,;:!?)\"'".contains(c));
        let Some(target) = smart_link_target(word) else {
            return;
        };
        let range = start..start + word.len();
        if self
            .body
            .read(cx)
            .document()
            .style_of_char_at(start)
            .link
            .is_some()
        {
            return;
        }
        let target: Arc<str> = Arc::from(target);
        self.body
            .update(cx, |editor, cx| editor.set_link_in(range, Some(target), cx));
    }

    /// Edit ▸ Spelling and Grammar ▸ Show Spelling and Grammar / Check
    /// Document Now.
    pub(super) fn check_document_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        let checker = Arc::clone(&self.spell_checker);
        rmac_ui::text_assist::check_document_now(
            &self.body,
            checker.as_ref(),
            self.text_assist.check_grammar_with_spelling,
            window,
            cx,
        );
    }

    /// Check Spelling While Typing: check the body once typing pauses. A
    /// newer change replaces the pending check.
    pub(super) fn schedule_spell_check(&mut self, cx: &mut Context<Self>) {
        let (text, revision, caret) = {
            let editor = self.body.read(cx);
            (editor.text(), editor.revision(), editor.cursor())
        };
        let body = self.body.clone();
        if !self.text_assist.check_spelling_while_typing {
            self.spell_task = None;
            body.update(cx, |editor, cx| {
                editor.set_misspelled(revision, Vec::new(), cx)
            });
            return;
        }
        let checker = Arc::clone(&self.spell_checker);
        self.spell_task = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(SPELL_CHECK_DELAY).await;
            let ranges = cx
                .background_executor()
                .spawn(async move { misspelled_ranges(&text, caret, checker.as_ref()) })
                .await;
            let _ = body.update(cx, |editor, cx| editor.set_misspelled(revision, ranges, cx));
        }));
    }

    /// Edit ▸ Speech ▸ Start Speaking.
    pub(super) fn start_speaking(&mut self, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        rmac_ui::start_speaking(&self.body, "notes::StopSpeaking", cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Words;

    impl SpellChecker for Words {
        fn is_correct(&self, word: &str) -> Option<bool> {
            Some(["hello", "world", "don't", "it"].contains(&word.to_lowercase().as_str()))
        }

        fn suggest(&self, _: &str) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn smart_links_recognise_web_and_mail_addresses() {
        assert_eq!(
            smart_link_target("https://example.com/a").as_deref(),
            Some("https://example.com/a")
        );
        assert_eq!(
            smart_link_target("www.example.com").as_deref(),
            Some("https://www.example.com")
        );
        assert_eq!(
            smart_link_target("me@example.com").as_deref(),
            Some("mailto:me@example.com")
        );
        assert_eq!(smart_link_target("example"), None);
        assert_eq!(smart_link_target("a@b"), None);
        assert_eq!(smart_link_target("http://"), None);
    }

    #[test]
    fn misspelled_words_are_found_but_not_the_one_being_typed() {
        let text = "hello wrold, don't it. helo";
        assert_eq!(misspelled_ranges(text, 0, &Words), [6..11, 23..27]);
        // The caret at the end of "helo": still being typed.
        assert_eq!(misspelled_ranges(text, text.len(), &Words), [6..11]);
        // Words with digits (codes, versions) are never marked.
        assert!(misspelled_ranges("v2beta abc123", 0, &Words).is_empty());
    }
}
