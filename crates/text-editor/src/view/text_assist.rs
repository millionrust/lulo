//! Edit ▸ Spelling and Grammar / Substitutions / Speech over the
//! document — the shared behaviour in `rmac_ui::text_assist`/
//! `rmac_ui::speech` and the dictionary in `rmac_spelling`, applied to
//! the document body (the plain field or the rich-text editor) the same way
//! `editing::transform_selection` already does (`editing_blocked` gates both).

use super::*;

impl EditorView {
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
