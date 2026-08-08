//! Terminal close and unprotected-paste confirmation projection.

use super::*;

impl TerminalView {
    pub(super) fn render_close_confirmation(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let pending = self.pending_close?;
        let (title, message, confirm_label): (&str, String, &str) = match pending {
            PendingClose::Tab { .. } => (
                "Close this terminal tab?",
                "A foreground process group is still using this terminal. Closing sends hangup to that group and its shell, then removes the tab.".into(),
                "Close Tab",
            ),
            PendingClose::Window {
                foreground_sessions,
            } => (
                "Close this Terminal window?",
                if foreground_sessions == 1 {
                    "One tab has an active foreground process group. Closing sends hangup to active groups and shells, then removes the window.".into()
                } else {
                    format!(
                        "{foreground_sessions} tabs have active foreground process groups. Closing sends hangup to active groups and shells, then removes the window."
                    )
                },
                "Close Window",
            ),
        };
        Some(rmac_ui::alert(
            title,
            message,
            vec![
                rmac_ui::dialog_button("terminal-close-cancel", "Cancel", Normal)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_close(window, cx);
                    }))
                    .into_any_element(),
                rmac_ui::dialog_button("terminal-close-confirm", confirm_label, Destructive)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_close(window, cx);
                    }))
                    .into_any_element(),
            ],
        ))
    }

    pub(super) fn render_paste_confirmation(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let pending = self.pending_paste.as_ref()?;
        let message = format!(
            "The clipboard contains {} lines ({} bytes), but the active program did not enable bracketed paste. Continuing sends line breaks as Return and may run commands.",
            pending.line_count, pending.byte_count
        );
        Some(rmac_ui::alert(
            "Paste multiple lines?",
            message,
            vec![
                rmac_ui::dialog_button("terminal-paste-cancel", "Cancel", Normal)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_paste(window, cx);
                    }))
                    .into_any_element(),
                rmac_ui::dialog_button("terminal-paste-confirm", "Paste Anyway", Destructive)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.confirm_paste(window, cx);
                    }))
                    .into_any_element(),
            ],
        ))
    }
}
