//! Small cross-platform boundary for user-mediated desktop open operations.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub operation: Operation,
    pub path: PathBuf,
    pub detail: String,
}

/// Privacy-safe failure for an arbitrary URI open request.
///
/// The requested URI is intentionally not retained: terminal links can contain
/// credentials, private paths, query strings, or fragments that must not leak
/// into a visible error or log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UriError {
    detail: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Open,
    Show,
    Choose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotesExportFormat {
    Markdown,
    Bundle,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.operation == Operation::Choose {
            return write!(f, "Could not choose a file: {}", self.detail);
        }
        write!(
            f,
            "Could not {} “{}”: {}",
            match self.operation {
                Operation::Open => "open",
                Operation::Show => "show",
                Operation::Choose => "choose",
            },
            self.path.display(),
            self.detail
        )
    }
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Could not open link: {}", self.detail)
    }
}

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
                FileFilter::new("rmac Notes bundles")
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
            NotesExportFormat::Bundle => FileFilter::new("rmac Notes bundles")
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

impl std::error::Error for Error {}
impl std::error::Error for UriError {}

/// Show a local item in the platform file manager, selecting it when supported.
pub async fn show_item(path: &Path) -> Result<(), Error> {
    #[cfg(target_os = "linux")]
    {
        if show_item_portal(path).await.is_ok() {
            return Ok(());
        }
        let directory = containing_directory(path);
        Command::new("xdg-open")
            .arg(directory)
            .spawn()
            .map(|_| ())
            .map_err(|error| failure(Operation::Show, path, error))
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| failure(Operation::Show, path, error))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(containing_directory(path))
            .spawn()
            .map(|_| ())
            .map_err(|error| failure(Operation::Show, path, error))
    }
}

/// Open a local item in its default application.
pub async fn open_item(path: &Path) -> Result<(), Error> {
    #[cfg(target_os = "linux")]
    {
        if open_item_portal(path).await.is_ok() {
            return Ok(());
        }
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| failure(Operation::Open, path, error))
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| failure(Operation::Open, path, error))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| failure(Operation::Open, path, error))
    }
}

/// Open a caller-validated non-file URI in its desktop handler.
///
/// Callers own scheme and content policy. This boundary never invokes a shell
/// and never includes the URI in its error value.
pub async fn open_uri(uri: &str) -> Result<(), UriError> {
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::open_uri::OpenFileRequest;

        let parsed = url::Url::parse(uri).map_err(uri_failure)?;
        match OpenFileRequest::default().send_uri(&parsed).await {
            Ok(request) => request.response().map_err(uri_failure),
            Err(_) => Command::new("xdg-open")
                .arg(uri)
                .spawn()
                .map(|_| ())
                .map_err(uri_failure),
        }
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(uri)
            .spawn()
            .map(|_| ())
            .map_err(uri_failure)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(uri)
            .spawn()
            .map(|_| ())
            .map_err(uri_failure)
    }
}

/// Open the freedesktop Trash location through the desktop portal, with the
/// standard desktop URI fallback used by ordinary Linux file managers.
pub async fn open_trash() -> Result<(), Error> {
    let target = Path::new("Trash");
    #[cfg(target_os = "linux")]
    {
        use ashpd::desktop::open_uri::OpenFileRequest;

        let uri = url::Url::parse("trash:///").expect("the fixed Trash URI is valid");
        if let Ok(request) = OpenFileRequest::default().send_uri(&uri).await {
            return request
                .response()
                .map_err(|error| failure(Operation::Open, target, error));
        }
        Command::new("xdg-open")
            .arg("trash:///")
            .spawn()
            .map(|_| ())
            .map_err(|error| failure(Operation::Open, target, error))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error {
            operation: Operation::Open,
            path: target.to_path_buf(),
            detail: "the freedesktop Trash location is available in the supported Linux session"
                .into(),
        })
    }
}

#[cfg(target_os = "linux")]
async fn show_item_portal(path: &Path) -> Result<(), Error> {
    use std::os::fd::AsFd as _;

    use ashpd::desktop::open_uri::OpenDirectoryRequest;

    let file = std::fs::File::open(path).map_err(|error| failure(Operation::Show, path, error))?;
    let request = OpenDirectoryRequest::default()
        .send(&file.as_fd())
        .await
        .map_err(|error| failure(Operation::Show, path, error))?;
    request
        .response()
        .map_err(|error| failure(Operation::Show, path, error))
}

#[cfg(target_os = "linux")]
async fn open_item_portal(path: &Path) -> Result<(), Error> {
    use std::os::fd::AsFd as _;

    use ashpd::desktop::open_uri::OpenFileRequest;

    let file = std::fs::File::open(path).map_err(|error| failure(Operation::Open, path, error))?;
    let request = OpenFileRequest::default()
        .send_file(&file.as_fd())
        .await
        .map_err(|error| failure(Operation::Open, path, error))?;
    request
        .response()
        .map_err(|error| failure(Operation::Open, path, error))
}

#[cfg(any(not(target_os = "macos"), test))]
fn containing_directory(path: &Path) -> &Path {
    if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    }
}

fn failure(operation: Operation, path: &Path, error: impl fmt::Display) -> Error {
    Error {
        operation,
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

fn uri_failure(_error: impl fmt::Display) -> UriError {
    UriError {
        detail: "the desktop handler did not accept the request",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_targets_the_containing_directory_for_files() {
        assert_eq!(
            containing_directory(Path::new("/usr/share/applications/demo.desktop")),
            Path::new("/usr/share/applications")
        );
    }

    #[test]
    fn errors_preserve_the_requested_path() {
        let error = failure(
            Operation::Show,
            Path::new("demo.desktop"),
            "portal unavailable",
        );
        assert_eq!(error.operation, Operation::Show);
        assert_eq!(error.path, Path::new("demo.desktop"));
        assert!(error.to_string().contains("portal unavailable"));
    }

    #[test]
    fn open_and_show_errors_name_the_operation() {
        let path = Path::new("report.txt");
        assert!(failure(Operation::Open, path, "offline")
            .to_string()
            .starts_with("Could not open"));
        assert!(failure(Operation::Show, path, "offline")
            .to_string()
            .starts_with("Could not show"));
    }

    #[test]
    fn uri_errors_do_not_retain_or_display_the_requested_uri() {
        let private_uri = "https://example.test/private?token=secret";
        let error = uri_failure(private_uri);

        assert!(!format!("{error:?}").contains(private_uri));
        assert!(!error.to_string().contains(private_uri));
        assert_eq!(
            error.to_string(),
            "Could not open link: the desktop handler did not accept the request"
        );
    }
}
