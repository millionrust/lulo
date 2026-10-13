//! Edit ▸ Spelling and Grammar / Substitutions / Speech over the
//! document — the shared behaviour in `rmac_ui::text_assist`/
//! `rmac_ui::speech` and the dictionary in `rmac_spelling`, applied to
//! the document body (the plain field or the rich-text editor) the same way
//! `editing::transform_selection` already does (`editing_blocked` gates both).

use super::*;

/// `assist` as a plain text document sees it under Settings' "...in rich
/// text documents only" scopes (TXT-SETTINGS-001/003): spelling, grammar and
/// automatic correction, and smart quotes and dashes, are switched off there
/// while the window's own Edit menu choices (and rich text) keep theirs.
fn scoped_for_plain_text(
    mut assist: rmac_ui::text_assist::TextAssistSettings,
    spelling_rich_text_only: bool,
    smart_quotes_rich_text_only: bool,
) -> rmac_ui::text_assist::TextAssistSettings {
    if spelling_rich_text_only {
        assist.check_spelling_while_typing = false;
        assist.check_grammar_with_spelling = false;
        assist.correct_spelling_automatically = false;
    }
    if smart_quotes_rich_text_only {
        assist.smart_quotes = false;
        assist.smart_dashes = false;
    }
    assist
}

impl EditorView {
    /// What Edit ▸ Spelling and Grammar / Substitutions do to the plain
    /// text body, after Settings' rich-text-only scopes.
    pub(super) fn plain_text_assist(&self) -> rmac_ui::text_assist::TextAssistSettings {
        scoped_for_plain_text(
            self.text_assist,
            self.spelling_rich_text_only,
            self.smart_quotes_rich_text_only,
        )
    }

    /// Edit ▸ Spelling and Grammar ▸ Show Spelling and Grammar / Check
    /// Document Now.
    pub(super) fn check_document_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_blocked() {
            return;
        }
        let checker = Arc::clone(&self.spell_checker);
        let grammar = self.text_assist.check_grammar_with_spelling;
        if self.rich_text {
            rmac_ui::text_assist::check_document_now(
                &self.rich,
                checker.as_ref(),
                grammar,
                window,
                cx,
            );
        } else {
            rmac_ui::text_assist::check_document_now(
                &self.input,
                checker.as_ref(),
                grammar,
                window,
                cx,
            );
        }
    }

    /// Edit ▸ Speech ▸ Start Speaking.
    pub(super) fn start_speaking(&mut self, cx: &mut Context<Self>) {
        if self.editing_blocked() {
            return;
        }
        if self.rich_text {
            rmac_ui::start_speaking(&self.rich, "text_editor::StopSpeaking", cx);
        } else {
            rmac_ui::start_speaking(&self.input, "text_editor::StopSpeaking", cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scoped_for_plain_text;
    use rmac_ui::text_assist::TextAssistSettings;

    #[test]
    fn rich_text_only_scopes_switch_plain_text_off_independently() {
        let assist = TextAssistSettings {
            correct_spelling_automatically: true,
            check_grammar_with_spelling: true,
            ..TextAssistSettings::default()
        };
        // Neither scope on: a plain document is untouched.
        assert_eq!(scoped_for_plain_text(assist, false, false), assist);

        let spelling = scoped_for_plain_text(assist, true, false);
        assert!(!spelling.check_spelling_while_typing);
        assert!(!spelling.check_grammar_with_spelling);
        assert!(!spelling.correct_spelling_automatically);
        assert!(spelling.smart_quotes && spelling.smart_dashes);

        let quotes = scoped_for_plain_text(assist, false, true);
        assert!(!quotes.smart_quotes && !quotes.smart_dashes);
        assert!(quotes.check_spelling_while_typing && quotes.correct_spelling_automatically);
        // Other substitutions are not part of either scope.
        assert!(quotes.text_replacement && quotes.smart_links && quotes.smart_copy_paste);
    }
}
