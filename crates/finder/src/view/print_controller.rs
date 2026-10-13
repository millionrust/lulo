//! File ▸ Print (⌘P): the selected document goes to the print portal. A PDF
//! is sent unchanged; a plain-text file is laid out by the same renderer
//! Text Editor and System Monitor print with. Anything else is refused with
//! a plain message instead of printing a wrong rendering of it. The portal
//! still owns printer choice and its print dialog.

use super::*;

/// The largest PDF handed to the portal (it is read into memory once).
const MAX_PDF_BYTES: u64 = 64 * 1024 * 1024;
/// The print renderer's own source limit (`rmac_print::MAX_SOURCE_BYTES`).
const MAX_TEXT_BYTES: u64 = 1024 * 1024;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "text", "md", "markdown", "log", "csv", "tsv", "json", "toml", "yaml", "yml", "ini",
    "conf", "cfg", "xml", "html", "htm", "css", "js", "ts", "rs", "py", "sh", "c", "h", "cpp",
    "hpp", "java", "go", "rb", "tex", "srt",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PrintKind {
    Pdf,
    Text,
}

/// How Files prints `path`, judged by its extension; `None` when it cannot.
pub(super) fn print_kind(path: &Path) -> Option<PrintKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if extension == "pdf" {
        Some(PrintKind::Pdf)
    } else if TEXT_EXTENSIONS.contains(&extension.as_str()) {
        Some(PrintKind::Text)
    } else {
        None
    }
}

/// The bytes of a PDF, checked for the `%PDF-` header and a sane size.
pub(super) fn check_pdf(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > MAX_PDF_BYTES {
        Err("the PDF is too large to print from Files".to_owned())
    } else if !bytes.starts_with(b"%PDF-") {
        Err("the file is not a valid PDF".to_owned())
    } else {
        Ok(bytes)
    }
}

/// The text of a plain-text file: UTF-8, no NUL bytes, within the limit.
pub(super) fn check_text(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() as u64 > MAX_TEXT_BYTES {
        return Err("the text file is larger than 1 MB".to_owned());
    }
    if bytes.contains(&0) {
        return Err("the file is not plain text".to_owned());
    }
    String::from_utf8(bytes).map_err(|_| "the file is not UTF-8 text".to_owned())
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Printable {
    Pdf(Vec<u8>),
    Text(String),
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn load_printable(path: &Path, kind: PrintKind) -> Result<Printable, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    match kind {
        PrintKind::Pdf => check_pdf(bytes).map(Printable::Pdf),
        PrintKind::Text => check_text(bytes).map(Printable::Text),
    }
}

impl FinderView {
    /// File ▸ Print: one selected PDF or text file.
    pub(super) fn print_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.trash_view || self.applications_view {
            return;
        }
        let items = self.selected_paths();
        let [path] = items.as_slice() else {
            self.operation_error = Some("Select one document to print.".into());
            cx.notify();
            return;
        };
        let Some(kind) = print_kind(path) else {
            self.operation_error =
                Some("Files can print PDF and plain-text documents. Open this one in its app to print it.".into());
            cx.notify();
            return;
        };
        #[cfg(target_os = "linux")]
        {
            let raw_window = raw_window_handle::HasWindowHandle::window_handle(window)
                .map(|handle| handle.as_raw());
            let raw_display = raw_window_handle::HasDisplayHandle::display_handle(window)
                .map(|handle| handle.as_raw());
            let (Ok(raw_window), Ok(raw_display)) = (raw_window, raw_display) else {
                self.operation_error = Some(
                    "Could not open the print dialog: printing needs the window to be shown."
                        .into(),
                );
                cx.notify();
                return;
            };
            let path = path.clone();
            let title = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Document".to_owned());
            let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
            cx.spawn_in(window, async move |this, cx| {
                let loaded = cx
                    .background_executor()
                    .spawn(async move { load_printable(&path, kind) })
                    .await;
                let outcome = match loaded {
                    Ok(Printable::Pdf(pdf)) => rmac_print_linux::print_prepared_document(
                        rmac_print_linux::PreparedPrintDocument {
                            window: raw_window,
                            display: raw_display,
                            window_generation: 1,
                            document_generation: 0,
                            current_document_generation: generation,
                            title,
                            pdf,
                        },
                    )
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                    Ok(Printable::Text(text)) => {
                        rmac_print_linux::print_document(rmac_print_linux::PrintDocument {
                            window: raw_window,
                            display: raw_display,
                            window_generation: 1,
                            document_generation: 0,
                            current_document_generation: generation,
                            title,
                            text,
                            rich: None,
                        })
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                    }
                    Err(message) => Err(message),
                };
                let _ = this.update(cx, |this: &mut FinderView, cx| {
                    if let Err(message) = outcome {
                        this.operation_error = Some(format!("Could not print: {message}.").into());
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (window, kind);
            self.operation_error =
                Some("Printing is available in the Lulo OS session on Linux.".into());
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_and_text_files_are_printable_by_extension() {
        assert_eq!(print_kind(Path::new("/a/Report.PDF")), Some(PrintKind::Pdf));
        assert_eq!(print_kind(Path::new("/a/notes.txt")), Some(PrintKind::Text));
        assert_eq!(print_kind(Path::new("/a/main.rs")), Some(PrintKind::Text));
        assert_eq!(print_kind(Path::new("/a/photo.jpg")), None);
        assert_eq!(print_kind(Path::new("/a/archive.zip")), None);
        assert_eq!(print_kind(Path::new("/a/README")), None);
    }

    #[test]
    fn a_pdf_needs_its_header() {
        assert!(check_pdf(b"%PDF-1.7\n".to_vec()).is_ok());
        assert!(check_pdf(b"hello".to_vec()).is_err());
    }

    #[test]
    fn text_must_be_small_utf8_without_nul() {
        assert_eq!(
            check_text("héllo\n".as_bytes().to_vec()).unwrap(),
            "héllo\n"
        );
        assert!(check_text(vec![b'a', 0, b'b']).is_err());
        assert!(check_text(vec![0xff, 0xfe, 0x41]).is_err());
        assert!(check_text(vec![b'a'; (MAX_TEXT_BYTES + 1) as usize]).is_err());
    }
}
