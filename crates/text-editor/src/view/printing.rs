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
                text: self.document_text(cx),
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

    /// File › Export as PDF…, like TextEdit's own. Reuses the same
    /// `rmac_print::render_pdf` renderer `print_document` calls after the
    /// print portal negotiates page settings, but this path has no portal
    /// dialog of its own — it renders with the default layout straight to a
    /// file chosen from a Save panel. Unlike printing, this needs no XDG
    /// print portal or Wayland window handle, so it isn't Linux-only.
    pub(super) fn export_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                title: "Could not export the document as PDF.",
                message: "Exporting the formatted RTF preview as PDF is not supported yet. Continue as plain text to export without implying the original formatting is preserved."
                    .into(),
            });
            cx.notify();
            return;
        }

        let directory = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let suggested_name = pdf_export_filename(self.path.as_deref());
        let text = self.document_text(cx);

        self.print_busy = true;
        self.status_notice = None;
        cx.notify();
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn_in(window, async move |this, cx| {
            let picker = receiver.await;
            let Ok(Ok(Some(path))) = picker else {
                let _ = this.update_in(cx, |this, _, cx| {
                    this.print_busy = false;
                    if !matches!(picker, Ok(Ok(None))) {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not open the save dialog.",
                            message: "The desktop file chooser is temporarily unavailable.".into(),
                        });
                    }
                    cx.notify();
                });
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { render_pdf_export(&path, &text) })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.print_busy = false;
                match result {
                    Ok(()) => {
                        this.status_notice = Some("Exported the document as PDF.".into());
                    }
                    Err(error) => {
                        this.alert = Some(ActiveAlert::Error {
                            title: "Could not export the document as PDF.",
                            message: error.to_string(),
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
