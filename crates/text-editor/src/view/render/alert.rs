//! Text Editor recovery, save, conflict, overwrite, and error alert projection.

use super::*;

impl EditorView {
    pub(super) fn render_alert(
        &self,
        alert: ActiveAlert,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};
        // TE-18: Esc must cancel the Save sheet (only) — every other alert
        // here already has an unambiguous Cancel button reachable by mouse,
        // and several (Recover, Conflict) have no safe "this is Cancel"
        // default, so Esc is left to do nothing for them rather than guess.
        let escape_cancels = matches!(&alert, ActiveAlert::ConfirmSave(_));
        let (title, message, buttons): (String, String, Vec<gpui::AnyElement>) = match alert {
            ActiveAlert::Recover(prompt) => (
                "Recover unsaved changes?".to_owned(),
                format!(
                    "An autosaved draft for “{}” was found.{}",
                    prompt.document_label,
                    if prompt.additional_drafts == 0 {
                        String::new()
                    } else {
                        format!(
                            " {} additional {} will remain available for a later launch.",
                            prompt.additional_drafts,
                            if prompt.additional_drafts == 1 {
                                "draft"
                            } else {
                                "drafts"
                            }
                        )
                    }
                ),
                vec![
                    rmac_ui::dialog_button("alert-discard", "Discard", Normal)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.alert_secondary(window, cx)),
                        )
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-restore", "Restore", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::ConfirmSave(_) => (
                // TE-18: this only ever fires for a document that has never
                // been saved (`guarded` autosaves a path-backed one instead)
                // — the Mac's Save sheet for a new "Untitled" document.
                format!(
                    "Do you want to keep this new document “{}”?",
                    self.filename()
                ),
                "You can choose to save your changes, or delete this document immediately."
                    .to_owned(),
                vec![
                    rmac_ui::dialog_button("alert-delete", "Delete", Destructive)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.alert_secondary(window, cx)),
                        )
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-save", "Save", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::Conflict => (
                "The document changed in another application.".to_owned(),
                "Text Editor did not overwrite the external version. Reload discards this local buffer, Save a Copy preserves it at a new location, and Overwrite requires a fresh review plus another exact preflight."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-reload", "Discard & Reload", Destructive)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.reload_conflicting_document(window, cx);
                        }))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-save-copy", "Save a Copy…", Primary)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.save_conflicting_copy(window, cx);
                        }))
                        .into_any_element(),
                    rmac_ui::dialog_button(
                        "alert-review-overwrite",
                        "Overwrite Anyway…",
                        Destructive,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.review_conflict_overwrite(window, cx);
                    }))
                    .into_any_element(),
                ],
            ),
            ActiveAlert::ConfirmOverwrite { .. } => (
                "Overwrite the external document?".to_owned(),
                "Text Editor reread the complete external revision. Overwrite will run a second exact preflight and stop if the document changes again. This cannot preserve the external edits."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel-overwrite", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button(
                        "alert-confirm-overwrite",
                        "Overwrite Anyway",
                        Destructive,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.alert_confirm(window, cx);
                    }))
                    .into_any_element(),
                ],
            ),
            ActiveAlert::ConfirmPlainTextConversion => (
                "Convert to plain text?".to_owned(),
                "This document's formatting — bold, italic, underline, fonts, sizes and colour — will be lost. This cannot be undone, though the original file is never changed unless you save over it."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel-plain-text", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-confirm-plain-text", "Convert", Destructive)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::Error { title, message } => (
                title.to_owned(),
                message,
                vec![rmac_ui::dialog_button("alert-ok", "OK", Primary)
                    .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                    .into_any_element()],
            ),
        };
        let dialog = rmac_ui::alert(title, message, buttons);
        if escape_cancels {
            dialog
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key.as_str() == "escape" {
                        cx.stop_propagation();
                        this.alert_cancel(cx);
                    }
                }))
                .into_any_element()
        } else {
            dialog.into_any_element()
        }
    }
}
