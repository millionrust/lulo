//! Bounded Text Editor document I/O and pure workflow admission policy.

use super::*;

pub(super) enum LoadedFile {
    Plain(document::DecodedDocument),
    /// An `.rtf` document, editable with its formatting (TE-03).
    RichText {
        document: rich::Document,
        original_bytes: Vec<u8>,
    },
}

/// What a save writes: plain text in the document's text format, or the
/// rich document as RTF.
#[derive(Clone)]
pub(super) enum SaveContent {
    Plain(String),
    Rich(rich::Document),
}

impl SaveContent {
    pub(super) fn is_rich(&self) -> bool {
        matches!(self, Self::Rich(_))
    }

    /// The extension a name typed without one gets.
    pub(super) fn default_extension(&self) -> &'static str {
        if self.is_rich() {
            "rtf"
        } else {
            "txt"
        }
    }
}

/// What an exact save wrote and read back.
#[derive(Debug)]
pub(super) enum SavedDocument {
    Plain(document::DecodedDocument),
    Rich { original_bytes: Vec<u8> },
}

/// Whether `path` names a rich-text file (by extension, as TextEdit
/// decides which format to open a file in).
pub(super) fn is_rich_text_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rtf"))
}

#[derive(Debug)]
pub(super) enum SaveFailure {
    Codec(document::CodecError),
    Storage(storage::SaveDocumentError),
    ConflictingCopyDestination,
    DestinationExists,
}

impl std::fmt::Display for SaveFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
            Self::ConflictingCopyDestination => formatter.write_str(
                "Save a Copy requires a different file; the conflicting source was not changed",
            ),
            Self::DestinationExists => formatter.write_str(
                "a document with that name already exists; choose another name or use Other… to review the destination",
            ),
        }
    }
}

pub(super) fn load_selected_document(path: &Path) -> Result<LoadedFile, String> {
    let bytes = storage::read_bounded(
        &storage::RealStorage,
        storage::Operation::LoadDocument,
        path,
        document::MAX_DOCUMENT_BYTES,
    )
    .map_err(|_| "Text Editor could not read the selected document".to_string())?;
    if is_rich_text_path(path) {
        let document = rich::rtf::parse(&bytes)
            .ok_or_else(|| "the RTF document could not be decoded safely".to_string())?;
        Ok(LoadedFile::RichText {
            document,
            original_bytes: bytes,
        })
    } else {
        document::decode(bytes)
            .map(LoadedFile::Plain)
            .map_err(|error| error.to_string())
    }
}

pub(super) fn save_document(
    path: &Path,
    expected: Option<&[u8]>,
    content: &SaveContent,
    format: document::TextFormat,
) -> Result<SavedDocument, SaveFailure> {
    match content {
        SaveContent::Plain(text) => {
            let encoded = document::encode(text, format).map_err(SaveFailure::Codec)?;
            storage::write_document_if_unchanged(&storage::RealStorage, path, expected, &encoded)
                .map_err(SaveFailure::Storage)?;
            // Encoding a valid Rust string through a supported format is
            // guaranteed to decode. Keeping this fallible preserves the
            // invariant without panicking.
            document::decode(encoded)
                .map(SavedDocument::Plain)
                .map_err(SaveFailure::Codec)
        }
        SaveContent::Rich(document) => {
            let encoded = rich::rtf::write(document);
            if encoded.len() > document::MAX_DOCUMENT_BYTES {
                return Err(SaveFailure::Codec(document::CodecError::EncodedTooLarge));
            }
            storage::write_document_if_unchanged(&storage::RealStorage, path, expected, &encoded)
                .map_err(SaveFailure::Storage)?;
            Ok(SavedDocument::Rich {
                original_bytes: encoded,
            })
        }
    }
}

pub(super) fn same_file_identity(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    let (Ok(left_metadata), Ok(right_metadata)) =
        (std::fs::metadata(left), std::fs::metadata(right))
    else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        left_metadata.dev() == right_metadata.dev() && left_metadata.ino() == right_metadata.ino()
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    }
}

pub(super) fn save_document_copy(
    path: &Path,
    forbidden_destination: Option<&Path>,
    content: &SaveContent,
    format: document::TextFormat,
) -> Result<SavedDocument, SaveFailure> {
    if forbidden_destination.is_some_and(|source| same_file_identity(source, path)) {
        Err(SaveFailure::ConflictingCopyDestination)
    } else {
        save_document(path, None, content, format)
    }
}

pub(super) fn inspect_external_revision(path: &Path, expected: &[u8]) -> Option<ExternalChange> {
    match storage::read_bounded(
        &storage::RealStorage,
        storage::Operation::ValidateDocumentRevision,
        path,
        document::MAX_DOCUMENT_BYTES,
    ) {
        Ok(current) if current == expected => None,
        Ok(_) => Some(ExternalChange::Modified),
        Err(failure) if failure.error_kind == std::io::ErrorKind::NotFound => {
            Some(ExternalChange::Missing)
        }
        Err(_) => Some(ExternalChange::Unreadable),
    }
}

pub(super) fn should_reuse_untitled_window(dirty: bool, has_path: bool, empty: bool) -> bool {
    !dirty && !has_path && empty
}

pub(super) fn can_begin_print(
    file_busy: bool,
    print_busy: bool,
    recovery_loading: bool,
    alert_open: bool,
    rich_text: bool,
) -> bool {
    !file_busy && !print_busy && !recovery_loading && !alert_open && !rich_text
}

#[derive(Debug)]
pub(super) enum ExportPdfFailure {
    Render(rmac_print::Error),
    Storage(storage::Failure),
}

impl std::fmt::Display for ExportPdfFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Render(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
        }
    }
}

/// Render the document to PDF and write it to `path`, off the UI thread.
///
/// This reuses the print pipeline's own renderer (`rmac_print::render_pdf`,
/// the same one `crates/rmac-print-linux` calls after the print portal
/// negotiates page settings) rather than a second implementation. Export as
/// PDF has no portal dialog to negotiate a page size with, so it uses this
/// document's own File ▸ Page Setup… choice (TXT-MENU-006), defaulting to
/// the same A4 layout the portal path itself falls back to.
pub(super) fn render_pdf_export(
    path: &Path,
    text: &str,
    layout: rmac_print::PageLayout,
) -> Result<(), ExportPdfFailure> {
    let pdf = rmac_print::render_pdf(text, layout).map_err(ExportPdfFailure::Render)?;
    storage::write(
        &storage::RealStorage,
        storage::Operation::ExportPdf,
        path,
        &pdf,
    )
    .map_err(ExportPdfFailure::Storage)
}

/// The suggested Export as PDF filename: the open document's name with its
/// extension replaced by `.pdf`, or "Untitled.pdf" for a document with no
/// path yet.
pub(super) fn pdf_export_filename(path: Option<&Path>) -> String {
    let stem = path
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("Untitled");
    format!("{stem}.pdf")
}
