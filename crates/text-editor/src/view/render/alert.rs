//! Text Editor recovery, save, conflict, overwrite, and error alert projection.

use super::*;

impl EditorView {
    pub(super) fn render_alert(
        &self,
        alert: ActiveAlert,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if matches!(alert, ActiveAlert::ConfirmSave(_)) {
            return self.render_save_sheet(cx);
        }
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};
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
            ActiveAlert::ConfirmSave(_) => unreachable!(),
            ActiveAlert::Conflict => (
                "The document changed in another application.".to_owned(),
                "Text Editor did not overwrite the external version. Reload discards this local buffer, Save a Copy preserves it at a new location, and Overwrite requires a fresh review plus another exact preflight."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_cancel(window, cx)))
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
                        .on_click(cx.listener(|this, _, window, cx| this.alert_cancel(window, cx)))
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
                "Convert this document to plain text?".to_owned(),
                "Making a rich text document plain will lose all text styles (such as fonts and colours) and attachments."
                    .into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel-plain-text", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_cancel(window, cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-confirm-plain-text", "OK", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::ConfirmRevert => (
                "Revert to the last saved version?".to_owned(),
                "Your changes since the last save will be lost. This cannot be undone.".into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel-revert", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_cancel(window, cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-confirm-revert", "Revert", Destructive)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::Error { title, message } => (
                title.to_owned(),
                message,
                vec![rmac_ui::dialog_button("alert-ok", "OK", Primary)
                    .on_click(cx.listener(|this, _, window, cx| this.alert_cancel(window, cx)))
                    .into_any_element()],
            ),
        };
        rmac_ui::alert(title, message, buttons).into_any_element()
    }
}
