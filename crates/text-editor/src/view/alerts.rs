//! Text Editor dirty-close, recovery, conflict, and error alert state machine.

use super::*;

impl EditorView {
    /// If the buffer is dirty, ask before discarding; otherwise act immediately.
    pub(super) fn guarded(
        &mut self,
        pending: Pending,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_busy || self.file_action_blocked() {
            return;
        }
        if !self.dirty {
            if self.clear_recovery(cx) {
                self.perform(pending, window, cx);
            }
            return;
        }
        self.alert = Some(ActiveAlert::ConfirmSave(pending));
        let _ = rmac_sound::play_alert();
        cx.notify();
    }

    /// Primary (default) button of the active alert: Restore / Save / OK.
    pub(super) fn alert_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.alert.take() {
            Some(ActiveAlert::Recover(prompt)) => {
                self.text_format = prompt.format;
                self.saved_format = prompt.format;
                self.path = None;
                self.saved_bytes = None;
                let longest_line = long_lines::longest_line_bytes(&prompt.content);
                self.install_document_text(prompt.content, longest_line, window, cx);
                if prompt.additional_drafts > 0 {
                    self.status_notice = Some(
                        format!(
                            "{} additional recovered {} remain available on the next launch.",
                            prompt.additional_drafts,
                            if prompt.additional_drafts == 1 {
                                "draft"
                            } else {
                                "drafts"
                            }
                        )
                        .into(),
                    );
                }
                // Recovered text is unsaved relative to the empty baseline, so
                // this marks the buffer dirty and re-arms autosave.
                self.on_buffer_changed(cx);
                if let Some(path) = self.pending_startup_path.take() {
                    if open_editor_window(cx, Some(path)).is_err() {
                        self.status_notice = Some(
                            "The recovered draft is safe, but Text Editor could not open the requested document window."
                                .into(),
                        );
                    }
                }
            }
            Some(ActiveAlert::ConfirmSave(pending)) => self.save_with(Some(pending), window, cx),
            Some(ActiveAlert::Conflict) => self.save_conflicting_copy(window, cx),
            Some(ActiveAlert::ConfirmOverwrite { reviewed_revision }) => {
                self.overwrite_conflicting_document(reviewed_revision, window, cx);
            }
            Some(ActiveAlert::Error { .. }) | None => {}
        }
        cx.notify();
    }

    /// Secondary button: Discard (recover) / Don't Save (confirm).
    pub(super) fn alert_secondary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.alert.take() {
            Some(ActiveAlert::Recover(prompt)) => {
                if !self.clear_recovery(cx) {
                    self.alert = Some(ActiveAlert::Recover(prompt));
                } else if let Some(path) = self.pending_startup_path.take() {
                    self.load_document_path(path, "The file could not be opened.", window, cx);
                }
            }
            Some(ActiveAlert::ConfirmSave(pending)) => {
                if self.clear_recovery(cx) {
                    self.perform(pending, window, cx);
                } else {
                    self.alert = Some(ActiveAlert::ConfirmSave(pending));
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Cancel / dismiss the alert without acting.
    pub(super) fn alert_cancel(&mut self, cx: &mut Context<Self>) {
        self.alert = None;
        cx.notify();
    }

    pub(super) fn perform(
        &mut self,
        pending: Pending,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match pending {
            Pending::Close => {
                self.closing = true;
                self.report_unsaved(cx);
                window.remove_window();
            }
        }
    }
}
