use std::fmt;
use std::path::PathBuf;

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
    pub(crate) detail: &'static str,
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

impl std::error::Error for Error {}
impl std::error::Error for UriError {}
