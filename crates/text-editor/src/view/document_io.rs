//! Bounded Text Editor document I/O and pure workflow admission policy.

use super::*;

pub(super) enum LoadedFile {
    Plain(document::DecodedDocument),
    RichText {
        text: String,
        runs: Vec<rtf::RtfRun>,
    },
}

#[derive(Debug)]
pub(super) enum SaveFailure {
    Codec(document::CodecError),
    Storage(storage::SaveDocumentError),
    ConflictingCopyDestination,
}

impl std::fmt::Display for SaveFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
            Self::ConflictingCopyDestination => formatter.write_str(
                "Save a Copy requires a different file; the conflicting source was not changed",
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
    let is_rtf = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rtf"));
    if is_rtf {
        let runs = rtf::parse_rtf(&bytes)
            .ok_or_else(|| "the RTF document could not be decoded safely".to_string())?;
        let text = runs.iter().map(|run| run.text.as_str()).collect();
        Ok(LoadedFile::RichText { text, runs })
    } else {
        document::decode(bytes)
            .map(LoadedFile::Plain)
            .map_err(|error| error.to_string())
    }
}

pub(super) fn save_document(
    path: &Path,
    expected: Option<&[u8]>,
    text: &str,
    format: document::TextFormat,
) -> Result<document::DecodedDocument, SaveFailure> {
    let encoded = document::encode(text, format).map_err(SaveFailure::Codec)?;
    storage::write_document_if_unchanged(&storage::RealStorage, path, expected, &encoded)
        .map_err(SaveFailure::Storage)?;
    // Encoding a valid Rust string through a supported format is guaranteed to
    // decode. Keeping this fallible preserves the invariant without panicking.
    document::decode(encoded).map_err(SaveFailure::Codec)
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
    text: &str,
    format: document::TextFormat,
) -> Result<document::DecodedDocument, SaveFailure> {
    if forbidden_destination.is_some_and(|source| same_file_identity(source, path)) {
        Err(SaveFailure::ConflictingCopyDestination)
    } else {
        save_document(path, None, text, format)
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

pub(super) fn should_reuse_untitled_window(
    dirty: bool,
    has_path: bool,
    rich_text_preview: bool,
    empty: bool,
) -> bool {
    !dirty && !has_path && !rich_text_preview && empty
}

pub(super) fn can_begin_print(
    file_busy: bool,
    print_busy: bool,
    recovery_loading: bool,
    alert_open: bool,
    rich_text_preview: bool,
) -> bool {
    !file_busy && !print_busy && !recovery_loading && !alert_open && !rich_text_preview
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
/// PDF has no portal dialog to negotiate a page size with, so it uses
/// `PageLayout::default()` — the same A4 layout the portal path itself falls
/// back to when a page description omits one.
pub(super) fn render_pdf_export(path: &Path, text: &str) -> Result<(), ExportPdfFailure> {
    let pdf = rmac_print::render_pdf(text, rmac_print::PageLayout::default())
        .map_err(ExportPdfFailure::Render)?;
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
