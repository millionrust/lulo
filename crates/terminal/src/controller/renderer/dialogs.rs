//! Terminal close and unprotected-paste confirmation projection.

use super::*;

impl TerminalView {
    pub(super) fn render_close_confirmation(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let pending = self.pending_close?;
        // Terminal's wording (macOS 26): the question names the tab or the
        // window, and the message lists the programs that will be stopped.
        let (title, scope, names) = match pending {
            PendingClose::Tab { session_id } => (
                "Do you want to terminate running processes in this tab?",
                "tab",
                self.tabs
                    .iter()
                    .filter(|session| session.id == session_id)
                    .filter_map(Session::foreground_job_name)
                    .collect::<Vec<_>>(),
            ),
            PendingClose::Window => (
                "Do you want to terminate running processes in this window?",
                "window",
                self.tabs
                    .iter()
                    .filter_map(Session::foreground_job_name)
                    .collect::<Vec<_>>(),
            ),
        };
        let message = running_processes_message(scope, &names);
        let confirm_label = "Terminate";
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

/// "Closing this window will terminate the running processes: vim, top."
fn running_processes_message(scope: &str, names: &[String]) -> String {
    let mut unique: Vec<&str> = Vec::with_capacity(names.len());
    for name in names {
        if !unique.contains(&name.as_str()) {
            unique.push(name);
        }
    }
    if unique.is_empty() {
        format!("Closing this {scope} will terminate the running processes.")
    } else {
        format!(
            "Closing this {scope} will terminate the running processes: {}.",
            unique.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::running_processes_message;

    #[test]
    fn the_close_review_names_each_running_program_once() {
        assert_eq!(
            running_processes_message("window", &["vim".into(), "top".into(), "vim".into()]),
            "Closing this window will terminate the running processes: vim, top."
        );
        assert_eq!(
            running_processes_message("tab", &[]),
            "Closing this tab will terminate the running processes."
        );
    }
}
