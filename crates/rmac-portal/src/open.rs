use std::fmt;
use std::path::Path;
use std::process::Command;

use crate::{Error, Operation, UriError};

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
pub(crate) fn containing_directory(path: &Path) -> &Path {
    if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    }
}

pub(crate) fn failure(operation: Operation, path: &Path, error: impl fmt::Display) -> Error {
    Error {
        operation,
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

pub(crate) fn uri_failure(_error: impl fmt::Display) -> UriError {
    UriError {
        detail: "the desktop handler did not accept the request",
    }
}
