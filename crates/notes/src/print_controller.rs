//! Notes › Print… (⌘P) through the desktop Print portal, the same
//! generation-bound transaction Text Editor uses, and Export as PDF, which
//! reuses the same renderer without a portal dialog of its own.

use super::*;
use rmac_storage::Backend as _;

/// The longest title passed to the print dialog, in characters. The portal
/// adapter bounds titles too; this keeps a long first line readable.
const MAX_PRINT_TITLE_CHARS: usize = 120;

/// The print dialog's title and the text printed for a note: the title as
/// the first line, a blank line, then the body. A note without a title is
/// named after its first non-blank line, as the note list names it.
pub(super) fn printable_note(title: &str, body: &str) -> (String, String) {
    let title = title.trim();
    let (name, text) = if title.is_empty() {
        let first_line = body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("New Note");
        (first_line.to_string(), body.to_string())
    } else {
        (title.to_string(), format!("{title}\n\n{body}"))
    };
    (name.chars().take(MAX_PRINT_TITLE_CHARS).collect(), text)
}

impl NotesView {
    pub(super) fn print_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy || self.closing || !self.is_interactive_ready() {
            return;
        }
        if self.session.selected_note().is_none() {
            self.message = Some("Select a note to print.".into());
            cx.notify();
            return;
        }
        let (title, text) =
            printable_note(&self.title.read(cx).value(), &self.body.read(cx).value());
        #[cfg(target_os = "linux")]
        {
            use std::sync::atomic::Ordering;

            let raw_window = raw_window_handle::HasWindowHandle::window_handle(window)
                .map(|handle| handle.as_raw());
            let raw_display = raw_window_handle::HasDisplayHandle::display_handle(window)
                .map(|handle| handle.as_raw());
            let (Ok(raw_window), Ok(raw_display)) = (raw_window, raw_display) else {
                self.message = Some(
                    "Could not open the print dialog: printing needs the Notes window to be shown."
                        .into(),
                );
                cx.notify();
                return;
            };
            let request = rmac_print_linux::PrintDocument {
                window: raw_window,
                display: raw_display,
                // Notes has one window for the life of the process.
                window_generation: 1,
                document_generation: self.print_generation.load(Ordering::Acquire),
                current_document_generation: self.print_generation.clone(),
                title,
                text,
            };
            // Printing keeps the window open until the portal answers
            // (`continue_close` waits for it), as the portal requires.
            self.print_busy = true;
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let result = rmac_print_linux::print_document(request).await;
                // The view is gone only if the window already closed, and then
                // there is nobody left to tell.
                this.update_in(cx, |this, _, cx| {
                    this.print_busy = false;
                    if let Err(error) = result {
                        this.message = Some(format!("Could not print the note: {error}.").into());
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (window, title, text);
            self.message = Some("Printing is available in the Lulo OS session on Linux.".into());
            cx.notify();
        }
    }

    /// Export the selected note as a PDF, like TextEdit-style Export as
    /// PDF…. Reuses the same `rmac_print::render_pdf` renderer `print_note`
    /// calls after the print portal negotiates a page size, but this path
    /// has no portal dialog of its own — it renders with the default layout
    /// straight to a file chosen from a Save panel. Unlike printing, this
    /// needs no XDG print portal or Wayland window handle, so it isn't
    /// Linux-only.
    pub(super) fn export_note_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.print_busy || self.closing || !self.is_interactive_ready() {
            return;
        }
        if self.session.selected_note().is_none() {
            self.message = Some("Select a note to export.".into());
            cx.notify();
            return;
        }
        let (title, text) =
            printable_note(&self.title.read(cx).value(), &self.body.read(cx).value());
        let suggested_name = format!("{}.pdf", safe_export_stem(&title));
        let directory = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        // `print_busy` covers this too: both are the print pipeline, and
        // `continue_close` already blocks the window on it.
        self.print_busy = true;
        self.message = None;
        cx.notify();
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn_in(window, async move |this, cx| {
            let picker = receiver.await;
            let Ok(Ok(Some(path))) = picker else {
                this.update_in(cx, |this, _, cx| {
                    this.print_busy = false;
                    if !matches!(picker, Ok(Ok(None))) {
                        this.message =
                            Some("The desktop file chooser is temporarily unavailable.".into());
                    }
                    cx.notify();
                })
                .ok();
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { render_note_pdf_export(&path, &text) })
                .await;
            this.update_in(cx, |this, _, cx| {
                this.print_busy = false;
                match result {
                    Ok(()) => {
                        this.message = Some("Exported the note as PDF.".into());
                    }
                    Err(error) => {
                        this.message = Some(format!("Could not export the note: {error}.").into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Failure rendering a note to PDF or writing it to disk.
#[derive(Debug)]
pub(super) enum ExportNotePdfError {
    Render(rmac_print::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for ExportNotePdfError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Render(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

/// Render `text` to PDF and write it to `path`, off the UI thread — the
/// same renderer `crates/rmac-print-linux` calls after the print portal
/// negotiates page settings, using its `PageLayout::default()` since this
/// path has no portal dialog to negotiate one with.
pub(super) fn render_note_pdf_export(
    path: &std::path::Path,
    text: &str,
) -> Result<(), ExportNotePdfError> {
    let pdf = rmac_print::render_pdf(text, rmac_print::PageLayout::default())
        .map_err(ExportNotePdfError::Render)?;
    rmac_storage::FileSystem
        .write_atomic(path, &pdf)
        .map_err(ExportNotePdfError::Io)
}

#[cfg(test)]
mod tests {
    use super::{printable_note, render_note_pdf_export};

    #[test]
    fn the_title_heads_the_printed_note() {
        assert_eq!(
            printable_note("  Groceries ", "Milk\nEggs"),
            (
                "Groceries".to_string(),
                "Groceries\n\nMilk\nEggs".to_string()
            )
        );
    }

    #[test]
    fn an_untitled_note_is_named_after_its_first_line() {
        assert_eq!(
            printable_note("", "\n  Trip plan \nDay one"),
            (
                "Trip plan".to_string(),
                "\n  Trip plan \nDay one".to_string()
            )
        );
        assert_eq!(
            printable_note(" ", ""),
            ("New Note".to_string(), String::new())
        );
    }

    #[test]
    fn a_long_title_is_shortened_for_the_dialog_only() {
        let long = "é".repeat(300);
        let (name, text) = printable_note(&long, "body");
        assert_eq!(name.chars().count(), 120);
        assert!(text.starts_with(&long));
    }

    #[test]
    fn export_pdf_writes_a_valid_pdf_to_the_chosen_path() {
        let directory =
            std::env::temp_dir().join(format!("rmac-notes-pdf-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("export.pdf");

        render_note_pdf_export(&path, "Exported from Notes").unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF"));

        std::fs::remove_dir_all(directory).unwrap();
    }
}
