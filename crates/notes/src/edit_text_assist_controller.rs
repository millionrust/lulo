//! Edit ▸ Spelling and Grammar / Substitutions / Speech over the note
//! body — the shared behaviour in `rmac_ui::text_assist`/`rmac_ui::speech`
//! and the dictionary in `rmac_spelling`, applied to Notes' one prose
//! field the same way `note_format_controller`'s Transformations already
//! does (`body_format_editable` gates both).

use super::*;

impl NotesView {
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

    /// Edit ▸ Speech ▸ Start Speaking.
    pub(super) fn start_speaking(&mut self, cx: &mut Context<Self>) {
        if !self.body_format_editable() {
            return;
        }
        rmac_ui::start_speaking(&self.body, "notes::StopSpeaking", cx);
    }
}
