//! Edit ▸ Spelling and Grammar / Substitutions / Speech over the
//! document — the shared behaviour in `rmac_ui::text_assist`/
//! `rmac_ui::speech` and the dictionary in `rmac_spelling`, applied to
//! `self.input` the same way `editing::transform_selection` already does
//! (`editing_blocked` gates both).

use super::*;

impl EditorView {
    /// Edit ▸ Spelling and Grammar ▸ Show Spelling and Grammar / Check
    /// Document Now.
    pub(super) fn check_document_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_blocked() {
            return;
        }
        let checker = Arc::clone(&self.spell_checker);
        rmac_ui::text_assist::check_document_now(
            &self.input,
            checker.as_ref(),
            self.text_assist.check_grammar_with_spelling,
            window,
            cx,
        );
    }

    /// Edit ▸ Speech ▸ Start Speaking.
    pub(super) fn start_speaking(&mut self, cx: &mut Context<Self>) {
        if self.editing_blocked() {
            return;
        }
        rmac_ui::start_speaking(&self.input, "text_editor::StopSpeaking", cx);
    }
}
