//! Shell ▸ Use Settings as Default, Export Settings…, Export Text As…,
//! Export Selected Text As…, Print… and Print Selection….

use std::path::{Path, PathBuf};

use super::*;

impl TerminalView {
    /// Shell ▸ Use Settings as Default: the live per-window values that
    /// `set_font` (⌘+ / ⌘− / ⌘0) only ever applies to *this* window become
    /// the default for windows opened after this one — the one escape
    /// hatch `settings_window.rs` promises for that otherwise window-local
    /// zoom. The active profile is already written to disk the moment it
    /// changes (`set_profile`), so there is nothing more to persist for it.
    pub(super) fn use_settings_as_default(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = profiles::save_font_size(self.font_size) {
            self.operation_error = Some(error.to_string().into());
            cx.notify();
        }
    }

    /// Shell ▸ Export Settings…: the Settings… window's own values, as the
    /// JSON file it already loads/saves, saved wherever the user chooses.
    pub(super) fn export_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(text) = settings::export_json() else {
            self.operation_error = Some("Could not read Terminal's settings.".into());
            cx.notify();
            return;
        };
        self.save_text_to_chosen_file(
            "Terminal Settings.json",
            text,
            "Could not export settings.",
            window,
            cx,
        );
    }

    /// Shell ▸ Export Text As…: the active tab's whole buffer (scrollback
    /// and screen), as plain text.
    pub(super) fn export_text_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.buffer_text() else {
            return;
        };
        self.save_text_to_chosen_file(
            &export_filename(&self.tabs[self.active]),
            text,
            "Could not export the buffer's text.",
            window,
            cx,
        );
    }

    /// Shell ▸ Export Selected Text As…: just the current selection.
    pub(super) fn export_selected_text_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.selection_text().filter(|text| !text.is_empty()) else {
            return;
        };
        self.save_text_to_chosen_file(
            &export_filename(&self.tabs[self.active]),
            text,
            "Could not export the selected text.",
            window,
            cx,
        );
    }

    fn save_text_to_chosen_file(
        &mut self,
        suggested_name: &str,
        text: String,
        failure_title: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = self.tabs[self.active]
            .working_directory()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let receiver = cx.prompt_for_new_path(&directory, Some(suggested_name));
        cx.spawn_in(window, async move |this, cx| {
            let picked = receiver.await;
            let Ok(Ok(Some(path))) = picked else {
                if !matches!(picked, Ok(Ok(None))) {
                    let _ = this.update(cx, |this, cx| {
                        this.operation_error = Some("Could not open the save dialog.".into());
                        cx.notify();
                    });
                }
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { write_text_file(&path, &text) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.operation_error = Some(format!("{failure_title} {error}").into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Shell ▸ Print…: the active tab's whole buffer, through the desktop
    /// print portal. The text is captured once, when the menu command
    /// runs — like a print preview, it prints that snapshot even if the
    /// shell keeps producing output while the portal's dialog is open.
    pub(super) fn print(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.buffer_text() else {
            return;
        };
        self.print_text(text, window, cx);
    }

    /// Shell ▸ Print Selection…: just the current selection.
    pub(super) fn print_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.selection_text().filter(|text| !text.is_empty()) else {
            return;
        };
        self.print_text(text, window, cx);
    }

    #[cfg(target_os = "linux")]
    fn print_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let raw_window =
            raw_window_handle::HasWindowHandle::window_handle(window).map(|handle| handle.as_raw());
        let raw_display = raw_window_handle::HasDisplayHandle::display_handle(window)
            .map(|handle| handle.as_raw());
        let (raw_window, raw_display) = match (raw_window, raw_display) {
            (Ok(raw_window), Ok(raw_display)) => (raw_window, raw_display),
            _ => {
                self.operation_error = Some(
                    "Printing requires the current exported Wayland application window.".into(),
                );
                cx.notify();
                return;
            }
        };
        let title = self.tabs[self.active]
            .tab_title()
            .unwrap_or_else(|| "Terminal".to_string());
        // One disposable generation per print: Terminal prints the text it
        // just captured rather than tracking later PTY output, so this is
        // never advanced after the request is built (see `print` above).
        let request = rmac_print_linux::PrintDocument {
            window: raw_window,
            display: raw_display,
            window_generation: self.window_generation,
            document_generation: 0,
            current_document_generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            title,
            text,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = rmac_print_linux::print_document(request).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.operation_error = Some(error.to_string().into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    #[cfg(not(target_os = "linux"))]
    fn print_text(&mut self, _text: String, _window: &mut Window, cx: &mut Context<Self>) {
        // `window_generation` only has a reader on Linux (the real print
        // path, above); this keeps the field from being dead code in a
        // macOS clippy build, which has no print path to read it at all.
        let _ = self.window_generation;
        self.operation_error =
            Some("Printing is implemented for the supported Linux session.".into());
        cx.notify();
    }
}

/// Shell ▸ Export Text As…/Export Selected Text As…: a plain name derived
/// from the tab's own title/job/directory label, never the shell's literal
/// command line.
fn export_filename(session: &Session) -> String {
    filename_for_title(session.tab_title())
}

fn filename_for_title(title: Option<String>) -> String {
    let base = title.unwrap_or_else(|| "Terminal".to_string());
    let sanitized: String = base
        .chars()
        .map(|character| if character == '/' { '-' } else { character })
        .collect();
    format!("{sanitized}.txt")
}

fn write_text_file(path: &Path, text: &str) -> Result<(), std::io::Error> {
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::filename_for_title;

    #[test]
    fn falls_back_to_terminal_with_no_tab_title() {
        assert_eq!(filename_for_title(None), "Terminal.txt");
    }

    #[test]
    fn uses_the_tab_title_and_strips_slashes() {
        assert_eq!(filename_for_title(Some("vim".into())), "vim.txt");
        assert_eq!(
            filename_for_title(Some("~/code/rmac".into())),
            "~-code-rmac.txt"
        );
    }
}
