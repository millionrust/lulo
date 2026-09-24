use std::path::PathBuf;

use crate::{Error, NotesExportFormat, Operation};

/// Ask the desktop portal for one local desktop-entry file.
pub async fn choose_desktop_entry() -> Result<Option<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let request = OpenFileRequest::default()
            .title("Add Login Item")
            .accept_label("Choose")
            .modal(true)
            .filter(FileFilter::new("Desktop entries").glob("*.desktop"))
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(choose_failure(error)),
        };
        let Some(uri) = response.uris().first() else {
            return Ok(None);
        };
        uri.to_file_path().map(Some).map_err(|()| Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the portal returned a non-local file".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the desktop file chooser is available in the supported Linux session".into(),
        })
    }
}

/// Ask the desktop portal for one or more local images or PDFs to open in
/// Preview. An empty list means the person cancelled.
///
/// The filter is chooser guidance only; Preview recognises every returned
/// file from its contents before showing it.
pub async fn choose_preview_documents() -> Result<Vec<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let request = OpenFileRequest::default()
            .title("Open")
            .accept_label("Open")
            .modal(true)
            .multiple(true)
            .filter(
                FileFilter::new("Images and PDF documents")
                    .mimetype("application/pdf")
                    .mimetype("image/png")
                    .mimetype("image/jpeg")
                    .mimetype("image/gif")
                    .mimetype("image/webp")
                    .mimetype("image/bmp")
                    .mimetype("image/tiff"),
            )
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(Vec::new()),
            Err(error) => return Err(choose_failure(error)),
        };
        response
            .uris()
            .iter()
            .map(|uri| {
                uri.to_file_path().map_err(|()| Error {
                    operation: Operation::Choose,
                    path: PathBuf::new(),
                    detail: "the portal returned a non-local document".into(),
                })
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the Preview open panel is available in the supported Linux session".into(),
        })
    }
}

/// Ask the desktop portal for audio and video files for Media Player. The
/// MIME types match its desktop entry.
pub async fn choose_media_files() -> Result<Vec<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let filter = [
            "audio/mpeg",
            "audio/mp4",
            "audio/x-m4a",
            "audio/aac",
            "audio/flac",
            "audio/x-flac",
            "audio/ogg",
            "audio/opus",
            "audio/x-vorbis+ogg",
            "audio/x-wav",
            "audio/wav",
            "video/mp4",
            "video/x-m4v",
            "video/quicktime",
            "video/x-matroska",
            "video/webm",
            "video/x-msvideo",
            "video/mpeg",
            "video/ogg",
        ]
        .into_iter()
        .fold(FileFilter::new("Movies and Audio"), FileFilter::mimetype);
        let request = OpenFileRequest::default()
            .title("Open")
            .accept_label("Open")
            .modal(true)
            .multiple(true)
            .filter(filter)
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(Vec::new()),
            Err(error) => return Err(choose_failure(error)),
        };
        response
            .uris()
            .iter()
            .map(|uri| {
                uri.to_file_path().map_err(|()| Error {
                    operation: Operation::Choose,
                    path: PathBuf::new(),
                    detail: "the portal returned a non-local file".into(),
                })
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the Media Player open panel is available in the supported Linux session"
                .into(),
        })
    }
}

/// Ask the desktop portal for one local PNG, JPEG, or WebP wallpaper file.
///
/// The filter is only chooser guidance. Callers must still validate the file
/// contents through their typed image authority before persisting a choice.
pub async fn choose_wallpaper_file() -> Result<Option<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let request = OpenFileRequest::default()
            .title("Choose Wallpaper")
            .accept_label("Choose")
            .modal(true)
            .filter(
                FileFilter::new("Wallpaper images")
                    .mimetype("image/png")
                    .mimetype("image/jpeg")
                    .mimetype("image/webp")
                    .glob("*.png")
                    .glob("*.jpg")
                    .glob("*.jpeg")
                    .glob("*.webp"),
            )
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(choose_failure(error)),
        };
        let Some(uri) = response.uris().first() else {
            return Ok(None);
        };
        uri.to_file_path().map(Some).map_err(|()| Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the portal returned a non-local wallpaper file".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the wallpaper chooser is available in the supported Linux session".into(),
        })
    }
}

/// Ask the desktop portal for one local PNG, JPEG, or WebP image to attach to
/// a note.
///
/// The filter is chooser guidance only. Notes must content-recognize, fully
/// decode, bound, and copy the selected source through its storage authority.
pub async fn choose_notes_image() -> Result<Option<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let request = OpenFileRequest::default()
            .title("Add Photo to Note")
            .accept_label("Add")
            .modal(true)
            .filter(
                FileFilter::new("Images")
                    .mimetype("image/png")
                    .mimetype("image/jpeg")
                    .mimetype("image/webp")
                    .glob("*.png")
                    .glob("*.jpg")
                    .glob("*.jpeg")
                    .glob("*.webp"),
            )
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(choose_failure(error)),
        };
        let Some(uri) = response.uris().first() else {
            return Ok(None);
        };
        uri.to_file_path().map(Some).map_err(|()| Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the portal returned a non-local image file".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the Notes image chooser is available in the supported Linux session".into(),
        })
    }
}

/// Ask the desktop portal for one local plain-text or Markdown file to import
/// as a note.
///
/// The filter is chooser guidance only. Notes must bound and strictly decode
/// the returned source through its text-import storage authority.
pub async fn choose_notes_text() -> Result<Option<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let request = OpenFileRequest::default()
            .title("Import Note")
            .accept_label("Import")
            .modal(true)
            .filter(
                FileFilter::new("Text and Markdown")
                    .mimetype("text/plain")
                    .mimetype("text/markdown")
                    .glob("*.txt")
                    .glob("*.md")
                    .glob("*.markdown"),
            )
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(choose_failure(error)),
        };
        let Some(uri) = response.uris().first() else {
            return Ok(None);
        };
        uri.to_file_path().map(Some).map_err(|()| Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the portal returned a non-local note file".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the Notes text importer is available in the supported Linux session".into(),
        })
    }
}

/// Ask the desktop portal for one local versioned rmac Notes bundle to import.
/// Notes must still parse, hash, decode, and review the complete returned file
/// before any library mutation.
pub async fn choose_notes_bundle() -> Result<Option<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let request = OpenFileRequest::default()
            .title("Import Notes Bundle")
            .accept_label("Review")
            .modal(true)
            .filter(
                FileFilter::new("Notes bundles")
                    .mimetype("application/octet-stream")
                    .glob("*.rmacnotes"),
            )
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(choose_failure(error)),
        };
        let Some(uri) = response.uris().first() else {
            return Ok(None);
        };
        uri.to_file_path().map(Some).map_err(|()| Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the portal returned a non-local Notes bundle".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the Notes bundle importer is available in the supported Linux session".into(),
        })
    }
}

/// Ask the desktop portal for one local destination for a reviewed Notes
/// export. The caller supplies a filename-safe suggestion and must still
/// revalidate the final destination at its storage boundary.
pub async fn choose_notes_export_destination(
    format: NotesExportFormat,
    suggested_name: &str,
) -> Result<Option<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, SaveFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let filter = match format {
            NotesExportFormat::Markdown => FileFilter::new("Markdown")
                .mimetype("text/markdown")
                .glob("*.md"),
            NotesExportFormat::Bundle => FileFilter::new("Notes bundles")
                .mimetype("application/octet-stream")
                .glob("*.rmacnotes"),
        };
        let request = SaveFileRequest::default()
            .title("Export Notes")
            .accept_label("Export")
            .modal(true)
            .current_name(suggested_name)
            .filter(filter)
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(choose_failure(error)),
        };
        let Some(uri) = response.uris().first() else {
            return Ok(None);
        };
        uri.to_file_path().map(Some).map_err(|()| Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the portal returned a non-local export destination".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (format, suggested_name);
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the Notes exporter is available in the supported Linux session".into(),
        })
    }
}

/// Ask the desktop portal for one local VPN configuration file.
///
/// VPN plugins own their input formats, so this chooser deliberately uses a
/// generic bounded configuration filter. The selected plugin and
/// `rmac-network` must still validate and stage the file before persistence.
pub async fn choose_vpn_configuration() -> Result<Option<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let request = OpenFileRequest::default()
            .title("Import VPN Configuration")
            .accept_label("Choose")
            .modal(true)
            .filter(FileFilter::new("VPN configurations").glob("*"))
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(choose_failure(error)),
        };
        let Some(uri) = response.uris().first() else {
            return Ok(None);
        };
        uri.to_file_path().map(Some).map_err(|()| Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the portal returned a non-local VPN configuration".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the VPN chooser is available in the supported Linux session".into(),
        })
    }
}

/// Ask the desktop portal for one local directory to exclude from search.
pub async fn choose_search_exclusion() -> Result<Option<PathBuf>, Error> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::file_chooser::OpenFileRequest;
        use ashpd::{desktop::ResponseError, Error as PortalError};

        let request = OpenFileRequest::default()
            .title("Exclude Folder from Search")
            .accept_label("Exclude")
            .modal(true)
            .directory(true)
            .send()
            .await
            .map_err(choose_failure)?;
        let response = match request.response() {
            Ok(response) => response,
            Err(PortalError::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(choose_failure(error)),
        };
        let Some(uri) = response.uris().first() else {
            return Ok(None);
        };
        uri.to_file_path().map(Some).map_err(|()| Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the portal returned a non-local search exclusion".into(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Choose,
            path: PathBuf::new(),
            detail: "the folder chooser is available in the supported Linux session".into(),
        })
    }
}

#[cfg(target_os = "linux")]
fn choose_failure(error: ashpd::Error) -> Error {
    Error {
        operation: Operation::Choose,
        path: PathBuf::new(),
        detail: error.to_string(),
    }
}
