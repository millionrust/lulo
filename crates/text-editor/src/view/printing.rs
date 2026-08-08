//! Text Editor generation-bound Linux print transaction orchestration.

use super::*;

impl EditorView {
    pub(super) fn print_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !can_begin_print(
            self.file_busy,
            self.print_busy,
            self.recovery_loading,
            self.alert.is_some(),
            false,
        ) {
            return;
        }
        if self.rtf_runs.is_some() {
            self.alert = Some(ActiveAlert::Error {
                title: "Could not print the document.",
                message: "Printing the formatted RTF preview is not supported yet. Continue as plain text to print without implying the original formatting is preserved."
                    .into(),
            });
            cx.notify();
            return;
        }
        #[cfg(target_os = "linux")]
        {
            let raw_window = raw_window_handle::HasWindowHandle::window_handle(window)
                .map(|handle| handle.as_raw());
            let raw_display = raw_window_handle::HasDisplayHandle::display_handle(window)
                .map(|handle| handle.as_raw());
            let (raw_window, raw_display) = match (raw_window, raw_display) {
                (Ok(raw_window), Ok(raw_display)) => (raw_window, raw_display),
                (Err(_), _) | (_, Err(_)) => {
                    self.alert = Some(ActiveAlert::Error {
                        title: "Could not open the print dialog.",
                        message:
                            "Printing requires the current exported Wayland application window."
                                .into(),
                    });
                    cx.notify();
                    return;
                }
            };
            let request = rmac_print_linux::PrintDocument {
                window: raw_window,
                display: raw_display,
                window_generation: self.window_generation,
                document_generation: self.document_generation,
                current_document_generation: self.current_document_generation.clone(),
                title: self.filename().to_string(),
                text: self.input.read(cx).value().to_string(),
            };
            self.print_busy = true;
            self.status_notice = None;
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let result = rmac_print_linux::print_document(request).await;
                let _ = this.update_in(cx, |this, _, cx| {
                    this.print_busy = false;
                    match result {
                        Ok(rmac_print_linux::Outcome::Printed) => {
                            this.status_notice =
                                Some("The desktop print service accepted the document.".into());
                        }
                        Ok(rmac_print_linux::Outcome::Cancelled) => {}
                        Err(error) => {
                            this.alert = Some(ActiveAlert::Error {
                                title: "Could not print the document.",
                                message: error.to_string(),
                            });
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = window;
            let _ = (self.window_generation, &self.current_document_generation);
            self.alert = Some(ActiveAlert::Error {
                title: "Printing is unavailable.",
                message: "Printing is implemented for the supported Linux session.".into(),
            });
            cx.notify();
        }
    }
}
