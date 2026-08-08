//! Text Editor recovery, save, conflict, overwrite, and error alert projection.

use super::*;

impl EditorView {
    pub(super) fn render_alert(
        &self,
        alert: ActiveAlert,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};
        let (title, message, buttons): (&str, String, Vec<gpui::AnyElement>) = match alert {
            ActiveAlert::Recover(prompt) => (
                "Recover unsaved changes?",
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
                "Do you want to save the changes you made?",
                "Your changes will be lost if you don't save them.".into(),
                vec![
                    rmac_ui::dialog_button("alert-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-dontsave", "Don't Save", Destructive)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.alert_secondary(window, cx)),
                        )
                        .into_any_element(),
                    rmac_ui::dialog_button("alert-save", "Save", Primary)
                        .on_click(cx.listener(|this, _, window, cx| this.alert_confirm(window, cx)))
                        .into_any_element(),
                ],
            ),
            ActiveAlert::Conflict => (
                "The document changed in another application.",
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
                "Overwrite the external document?",
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
            ActiveAlert::Error { title, message } => (
                title,
                message,
                vec![rmac_ui::dialog_button("alert-ok", "OK", Primary)
                    .on_click(cx.listener(|this, _, _, cx| this.alert_cancel(cx)))
                    .into_any_element()],
            ),
        };
        rmac_ui::alert(title, message, buttons)
    }
}
