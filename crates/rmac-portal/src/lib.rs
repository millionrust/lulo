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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Open,
    Show,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Could not {} “{}”: {}",
            match self.operation {
                Operation::Open => "open",
                Operation::Show => "show",
            },
            self.path.display(),
            self.detail
        )
    }
}

impl std::error::Error for Error {}

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
}
